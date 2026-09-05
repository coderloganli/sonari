//! What the agent knows about a caller, and how it learns it.
//!
//! Two halves that never run at the same time. Reading is a local query on the
//! turn path; writing is a model call on a task the turn path spawned and forgot
//! about (ADR-0022). Between them sits a bounded set of natural-language facts,
//! injected whole rather than searched (ADR-0021).

use std::sync::Arc;

use async_trait::async_trait;
use shared_kernel::{AppError, AppResult};

use crate::domain::{
    AgentCallerIdentity, AgentMessage, ExtractedFact, MemoryCategory, MemoryFact, MessageRole,
    PromptTemplateKey, ProviderKey,
};
use crate::ports::{
    AgentMessageRepository, AgentSessionRepository, Clock, LlmCompletionRequest, LlmGateway,
    LlmProviderConfigRepository, LlmRequestMessage, MemoryStore, PromptTemplateRepository,
};

/// How memory behaves. All of it from `sonari.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryPolicy {
    pub enabled: bool,
    /// Completed turns between extractions.
    pub extract_every_turns: i32,
    pub max_facts: usize,
    pub max_facts_per_category: usize,
}

impl Default for MemoryPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            extract_every_turns: 4,
            max_facts: 40,
            max_facts_per_category: 12,
        }
    }
}

/// How the fact set is introduced to the model. Deliberately short: the persona
/// prompts are what shape the reply, and this is context, not instruction.
const MEMORY_PREAMBLE: &str = concat!(
    "What you already know about the person you are speaking to, from earlier ",
    "calls. Use it the way a friend would — do not recite it, and do not ",
    "mention that you have notes."
);

/// Renders a fact set into the one system message that carries it.
///
/// `None` for an empty set: an agent with nothing to remember sends exactly the
/// prompt it sent before this existed.
pub fn render_facts(facts: &[MemoryFact]) -> Option<String> {
    if facts.is_empty() {
        return None;
    }
    let mut rendered = String::from(MEMORY_PREAMBLE);
    // Grouped in the order the category list declares, so the stable things are
    // read before the passing ones, and the text does not reshuffle between
    // turns.
    for category in MemoryCategory::ALL {
        let mut in_category = facts
            .iter()
            .filter(|fact| fact.category == category)
            .peekable();
        if in_category.peek().is_none() {
            continue;
        }
        rendered.push_str("\n\n");
        rendered.push_str(category.as_str());
        rendered.push(':');
        for fact in in_category {
            rendered.push_str("\n- ");
            rendered.push_str(fact.content.trim());
        }
    }
    Some(rendered)
}

/// Trims a model's output down to what may be stored.
///
/// Pure, and where both caps live. Returns what survives; the caller logs what
/// did not. Order is the model's, because it puts what it thought mattered
/// first and the caps cut from the end.
pub fn validate(facts: Vec<ExtractedFact>, policy: &MemoryPolicy) -> Vec<ExtractedFact> {
    let mut seen: Vec<String> = Vec::new();
    let mut per_category: Vec<(MemoryCategory, usize)> = Vec::new();
    let mut kept = Vec::new();
    for fact in facts {
        let content = fact.content.trim();
        if content.is_empty() {
            continue;
        }
        // Content alone, not content and category. The storage key is
        // `(user_id, character_id, content)`, so the same sentence filed under
        // two categories is one row either way — deduping by the pair here would
        // only move the collision into the upsert, where the later row silently
        // rewrites the earlier one's category. Case and surrounding space are
        // not a difference worth a row: the model restates the same sentence
        // more than one way.
        let fingerprint = content.to_lowercase();
        if seen.contains(&fingerprint) {
            continue;
        }
        let count = match per_category
            .iter_mut()
            .find(|(category, _)| *category == fact.category)
        {
            Some((_, count)) => count,
            None => {
                per_category.push((fact.category, 0));
                &mut per_category.last_mut().expect("just pushed").1
            }
        };
        if *count >= policy.max_facts_per_category {
            continue;
        }
        *count += 1;
        seen.push(fingerprint);
        kept.push(ExtractedFact {
            category: fact.category,
            content: content.to_owned(),
        });
        if kept.len() == policy.max_facts {
            break;
        }
    }
    kept
}

/// What the extraction model is asked to return.
#[derive(Debug, serde::Deserialize)]
struct ExtractionReply {
    facts: Vec<ExtractionReplyFact>,
}

#[derive(Debug, serde::Deserialize)]
struct ExtractionReplyFact {
    category: String,
    content: String,
}

/// What a reply yielded, and how much of it was unusable.
struct ParsedReply {
    facts: Vec<ExtractedFact>,
    /// Facts named with a category outside the closed list. Counted rather than
    /// discarded silently: if the prompt or the model drifts into inventing
    /// categories, the only way anyone finds out is this number.
    unknown_categories: usize,
}

/// Reads the fact set out of a reply.
///
/// `None` when there is no object to read, which is the model answering in prose
/// instead of JSON. A fact whose category is not in the closed list is dropped
/// rather than the reply rejected wholesale: one bad row is not a reason to lose
/// the other four.
fn parse_reply(reply: &str) -> Option<ParsedReply> {
    // Models wrap JSON in prose or a fence often enough that finding the object
    // is worth more than insisting the whole reply is one.
    let start = reply.find('{')?;
    let end = reply.rfind('}')?;
    if end <= start {
        return None;
    }
    let parsed: ExtractionReply = serde_json::from_str(&reply[start..=end]).ok()?;
    let mut facts = Vec::new();
    let mut unknown_categories = 0;
    for fact in parsed.facts {
        match MemoryCategory::parse(fact.category.trim()) {
            Some(category) => facts.push(ExtractedFact {
                category,
                content: fact.content,
            }),
            None => unknown_categories += 1,
        }
    }
    Some(ParsedReply {
        facts,
        unknown_categories,
    })
}

/// What the extraction model is shown: what is already known, and what was just
/// said. Not a prompt — the instruction is the template's job.
fn render_extraction_input(current: &[MemoryFact], recent: &[AgentMessage]) -> String {
    let mut input = String::from("Known so far:");
    if current.is_empty() {
        input.push_str("\n(nothing)");
    } else {
        for fact in current {
            input.push_str("\n- ");
            input.push_str(fact.category.as_str());
            input.push_str(": ");
            input.push_str(&fact.content);
        }
    }
    input.push_str("\n\nThe conversation since:");
    for message in recent {
        input.push('\n');
        input.push_str(match message.role {
            MessageRole::Assistant => "agent: ",
            _ => "caller: ",
        });
        input.push_str(message.content.trim());
    }
    input
}

/// Reads and forgets, for the caller's own routes. No writing: a caller may see
/// and delete what is held about them, not author it.
#[async_trait]
pub trait MemoryUseCases: Send + Sync {
    async fn list(&self, user_id: i64) -> AppResult<Vec<MemoryFact>>;
    async fn forget(&self, user_id: i64, character_id: Option<i64>) -> AppResult<u64>;
}

pub struct MemoryDependencies {
    pub memory: Arc<dyn MemoryStore>,
    pub sessions: Arc<dyn AgentSessionRepository>,
    pub messages: Arc<dyn AgentMessageRepository>,
    pub providers: Arc<dyn LlmProviderConfigRepository>,
    pub templates: Arc<dyn PromptTemplateRepository>,
    pub gateway: Arc<dyn LlmGateway>,
    pub clock: Arc<dyn Clock>,
    pub policy: MemoryPolicy,
}

/// Turns conversation into facts.
///
/// Trait objects rather than the type parameters `AgentService` uses: this is
/// constructed once in the composition root and handed to a spawned task, where
/// a dozen type parameters buy nothing.
pub struct MemoryService {
    deps: MemoryDependencies,
}

impl MemoryService {
    pub fn new(deps: MemoryDependencies) -> Self {
        Self { deps }
    }

    /// Reads the recent turns and the current set, asks for a replacement, and
    /// writes it.
    ///
    /// Returns nothing, including on failure. Its caller is a spawned task with
    /// nowhere to return an error to, and a failed extraction means the agent
    /// learns nothing this time — not that anything is wrong with the call
    /// (ADR-0022).
    pub async fn extract(&self, session_id: &str) {
        if let Err(error) = self.try_extract(session_id).await {
            tracing::warn!(session_id, %error, "memory extraction failed");
        }
    }

    async fn try_extract(&self, session_id: &str) -> AppResult<()> {
        let session = self
            .deps
            .sessions
            .get_by_id(session_id)
            .await?
            .ok_or_else(|| AppError::not_found("agent session not found"))?;
        let AgentCallerIdentity::PlatformUser { user_id } = session.caller;
        let character_id = session.character_id;

        let current = self.deps.memory.load(user_id, character_id).await?;
        let recent = self
            .deps
            .messages
            .list_recent(session_id, self.deps.policy.extract_every_turns)
            .await?;
        if recent.is_empty() {
            return Ok(());
        }

        let provider = self
            .deps
            .providers
            .get_by_key(ProviderKey::Assistant)
            .await?
            .ok_or_else(|| AppError::invalid_input("no model is configured for extraction"))?;
        let template = self
            .deps
            .templates
            .get_by_key(PromptTemplateKey::MemoryExtraction)
            .await?
            .ok_or_else(|| AppError::invalid_input("no extraction prompt is configured"))?;

        let request = LlmCompletionRequest {
            endpoint_url: provider.endpoint_url.clone(),
            api_key: provider.api_key.clone(),
            model_name: provider.model_name.clone(),
            temperature: provider.temperature,
            frequency_penalty: provider.frequency_penalty,
            messages: vec![
                LlmRequestMessage {
                    role: MessageRole::System.as_str().to_owned(),
                    content: self.render_instruction(&template.template_text),
                },
                LlmRequestMessage {
                    role: MessageRole::User.as_str().to_owned(),
                    content: render_extraction_input(&current, &recent),
                },
            ],
            max_tokens: None,
            tools: Vec::new(),
        };

        let reply = super::collect_reply(self.deps.gateway.stream(request).await?).await?;
        let Some(parsed) = parse_reply(&reply.content) else {
            tracing::warn!(
                session_id,
                "extraction reply was not a fact set; the stored set is left alone"
            );
            return Ok(());
        };
        // Everything the model put forward, including what it named with a
        // category that does not exist, so the drop count is the whole truth.
        let offered = parsed.facts.len() + parsed.unknown_categories;
        let facts = validate(parsed.facts, &self.deps.policy);
        if facts.is_empty() {
            // A set that came back empty is far more likely to be a model having
            // a bad turn than a caller whose every fact stopped being true, and
            // the cost of the two mistakes is not symmetric.
            tracing::warn!(
                session_id,
                offered,
                "extraction produced no storable facts; the stored set is left alone"
            );
            return Ok(());
        }

        self.deps
            .memory
            .replace(user_id, character_id, session_id, &facts)
            .await?;
        tracing::info!(
            session_id,
            user_id,
            character_id,
            held_before = current.len(),
            offered,
            stored = facts.len(),
            dropped = offered - facts.len(),
            "memory extracted"
        );
        Ok(())
    }

    fn render_instruction(&self, template_text: &str) -> String {
        let categories = MemoryCategory::ALL
            .iter()
            .map(|category| category.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        template_text
            .replace("{{max_facts}}", &self.deps.policy.max_facts.to_string())
            .replace(
                "{{max_facts_per_category}}",
                &self.deps.policy.max_facts_per_category.to_string(),
            )
            .replace("{{categories}}", &categories)
    }
}

#[async_trait]
impl MemoryUseCases for MemoryService {
    async fn list(&self, user_id: i64) -> AppResult<Vec<MemoryFact>> {
        self.deps.memory.load_all(user_id).await
    }

    async fn forget(&self, user_id: i64, character_id: Option<i64>) -> AppResult<u64> {
        let deleted = self.deps.memory.delete(user_id, character_id).await?;
        tracing::info!(user_id, ?character_id, deleted, "memory forgotten");
        Ok(deleted)
    }
}

#[cfg(test)]
mod tests {
    //! Test cases 11-17 of task.md. Case 18, the acceptance case, needs both
    //! halves and lives beside the injection tests in `application/mod.rs`.
    //!
    //! The gateway is scripted, so nothing here judges what a model would
    //! actually extract. What is under test is that a reply becomes the stored
    //! set, that the caps and the closed category list hold, and that every way
    //! the extraction can fail leaves the stored set alone.

    use super::*;
    use crate::domain::{
        AgentCallerIdentity, AgentMessage, AgentSession, LlmProviderConfig, MemoryCategory,
        MessageRole, PromptTemplate, PromptTemplateKey, ProviderKey,
    };
    use crate::ports::{LlmCompletionRequest, LlmDelta, LlmStream, LlmUsage};
    use shared_kernel::AppError;
    use std::sync::Mutex;

    /// Records every set it was asked to store, and serves a fixed one.
    #[derive(Default)]
    struct SpyMemory {
        held: Vec<MemoryFact>,
        written: Mutex<Vec<Vec<ExtractedFact>>>,
    }

    impl SpyMemory {
        fn holding(held: Vec<MemoryFact>) -> Self {
            Self {
                held,
                written: Mutex::new(Vec::new()),
            }
        }

        fn last_written(&self) -> Vec<ExtractedFact> {
            let written = self.written.lock().unwrap();
            assert_eq!(written.len(), 1, "expected exactly one write");
            written[0].clone()
        }

        fn never_written(&self) -> bool {
            self.written.lock().unwrap().is_empty()
        }
    }

    #[async_trait]
    impl MemoryStore for SpyMemory {
        async fn load(&self, _user_id: i64, _character_id: i64) -> AppResult<Vec<MemoryFact>> {
            Ok(self.held.clone())
        }
        async fn load_all(&self, _user_id: i64) -> AppResult<Vec<MemoryFact>> {
            Ok(self.held.clone())
        }
        async fn replace(
            &self,
            _user_id: i64,
            _character_id: i64,
            _source_session_id: &str,
            facts: &[ExtractedFact],
        ) -> AppResult<()> {
            self.written.lock().unwrap().push(facts.to_vec());
            Ok(())
        }
        async fn delete(&self, _user_id: i64, _character_id: Option<i64>) -> AppResult<u64> {
            Ok(0)
        }
    }

    struct ScriptedGateway {
        reply: Option<String>,
    }

    impl ScriptedGateway {
        fn saying(reply: &str) -> Self {
            Self {
                reply: Some(reply.to_owned()),
            }
        }

        fn broken() -> Self {
            Self { reply: None }
        }
    }

    #[async_trait]
    impl LlmGateway for ScriptedGateway {
        async fn stream(&self, _request: LlmCompletionRequest) -> AppResult<LlmStream> {
            match &self.reply {
                Some(reply) => Ok(Box::pin(futures::stream::iter(vec![
                    Ok(LlmDelta::Token(reply.clone())),
                    Ok(LlmDelta::Done(LlmUsage::default())),
                ]))),
                None => Err(AppError::unavailable("the model endpoint is unreachable")),
            }
        }
    }

    struct StubSessions;

    #[async_trait]
    impl AgentSessionRepository for StubSessions {
        async fn create(&self, session: &AgentSession) -> AppResult<AgentSession> {
            Ok(session.clone())
        }
        async fn get_by_id(&self, session_id: &str) -> AppResult<Option<AgentSession>> {
            Ok(Some(AgentSession {
                id: session_id.to_owned(),
                caller: AgentCallerIdentity::PlatformUser { user_id: 7 },
                character_id: 11,
                timezone: "UTC".into(),
                scene_id: None,
                started_at: chrono::Utc::now(),
                ended_at: None,
            }))
        }
        async fn end(&self, _session_id: &str) -> AppResult<()> {
            Ok(())
        }
    }

    struct StubMessages;

    #[async_trait]
    impl AgentMessageRepository for StubMessages {
        async fn append(&self, message: &AgentMessage) -> AppResult<AgentMessage> {
            Ok(message.clone())
        }
        async fn list_recent(
            &self,
            session_id: &str,
            _recent_turns: i32,
        ) -> AppResult<Vec<AgentMessage>> {
            Ok(vec![AgentMessage {
                id: 1,
                session_id: session_id.to_owned(),
                role: MessageRole::User,
                content: "I have a cat called Coal.".into(),
                turn_number: 1,
                created_at: chrono::Utc::now(),
            }])
        }
        async fn list_all(&self, _session_id: &str) -> AppResult<Vec<AgentMessage>> {
            Ok(Vec::new())
        }
        async fn next_turn_number(&self, _session_id: &str) -> AppResult<i32> {
            Ok(2)
        }
    }

    struct StubProviders;

    #[async_trait]
    impl LlmProviderConfigRepository for StubProviders {
        async fn get_by_key(
            &self,
            provider_key: ProviderKey,
        ) -> AppResult<Option<LlmProviderConfig>> {
            Ok(Some(LlmProviderConfig {
                provider_key,
                endpoint_url: "https://example.invalid".into(),
                api_key: String::new(),
                model_name: "extraction-model".into(),
                temperature: 0.0,
                frequency_penalty: 0.0,
                updated_at: chrono::Utc::now(),
            }))
        }
        async fn list_all(&self) -> AppResult<Vec<LlmProviderConfig>> {
            Ok(Vec::new())
        }
        async fn upsert(&self, config: &LlmProviderConfig) -> AppResult<LlmProviderConfig> {
            Ok(config.clone())
        }
    }

    struct StubTemplates;

    #[async_trait]
    impl PromptTemplateRepository for StubTemplates {
        async fn get_by_key(&self, key: PromptTemplateKey) -> AppResult<Option<PromptTemplate>> {
            Ok(Some(PromptTemplate {
                id: 1,
                template_key: key,
                template_text: "Extract at most {{max_facts}} facts.".into(),
                updated_at: chrono::Utc::now(),
            }))
        }
        async fn list_all(&self) -> AppResult<Vec<PromptTemplate>> {
            Ok(Vec::new())
        }
        async fn upsert(&self, template: &PromptTemplate) -> AppResult<PromptTemplate> {
            Ok(template.clone())
        }
    }

    struct StubClock;

    impl Clock for StubClock {
        fn now(&self) -> chrono::DateTime<chrono::Utc> {
            chrono::Utc::now()
        }
    }

    fn policy() -> MemoryPolicy {
        MemoryPolicy {
            enabled: true,
            extract_every_turns: 4,
            max_facts: 40,
            max_facts_per_category: 12,
        }
    }

    fn service_with(
        store: Arc<SpyMemory>,
        gateway: ScriptedGateway,
        policy: MemoryPolicy,
    ) -> MemoryService {
        MemoryService::new(MemoryDependencies {
            memory: store,
            sessions: Arc::new(StubSessions),
            messages: Arc::new(StubMessages),
            providers: Arc::new(StubProviders),
            templates: Arc::new(StubTemplates),
            gateway: Arc::new(gateway),
            clock: Arc::new(StubClock),
            policy,
        })
    }

    fn extracted(category: MemoryCategory, content: &str) -> ExtractedFact {
        ExtractedFact {
            category,
            content: content.to_owned(),
        }
    }

    /// Test case 11 — a reply becomes the stored set.
    #[tokio::test]
    async fn a_reply_becomes_the_stored_set() {
        let store = Arc::new(SpyMemory::default());
        let reply = r#"{"facts":[
            {"category":"relationship","content":"The caller has a cat called Coal."},
            {"category":"identity","content":"The caller is called Ada."}
        ]}"#;
        let service = service_with(store.clone(), ScriptedGateway::saying(reply), policy());

        service.extract("session-1").await;

        assert_eq!(
            store.last_written(),
            vec![
                extracted(
                    MemoryCategory::Relationship,
                    "The caller has a cat called Coal."
                ),
                extracted(MemoryCategory::Identity, "The caller is called Ada."),
            ]
        );
    }

    /// Test case 12 — unknown categories are dropped, the rest kept.
    #[tokio::test]
    async fn unknown_categories_are_dropped() {
        let store = Arc::new(SpyMemory::default());
        let reply = r#"{"facts":[
            {"category":"identity","content":"The caller is called Ada."},
            {"category":"favourite_colour","content":"The caller likes green."},
            {"category":"preference","content":"The caller dislikes small talk."},
            {"category":"situation","content":"The caller has an interview on Friday."}
        ]}"#;
        let service = service_with(store.clone(), ScriptedGateway::saying(reply), policy());

        service.extract("session-1").await;

        let written = store.last_written();
        assert_eq!(written.len(), 3);
        assert!(!written.iter().any(|f| f.content.contains("green")));
    }

    /// Test case 13 — both caps hold.
    #[test]
    fn caps_hold() {
        let per_category = MemoryPolicy {
            max_facts_per_category: 2,
            ..policy()
        };
        let four_situations: Vec<ExtractedFact> = (1..=4)
            .map(|n| extracted(MemoryCategory::Situation, &format!("Situation {n}.")))
            .collect();

        assert_eq!(validate(four_situations, &per_category).len(), 2);

        let total = MemoryPolicy {
            max_facts: 3,
            ..policy()
        };
        let five_across = vec![
            extracted(MemoryCategory::Identity, "One."),
            extracted(MemoryCategory::Relationship, "Two."),
            extracted(MemoryCategory::Preference, "Three."),
            extracted(MemoryCategory::Situation, "Four."),
            extracted(MemoryCategory::Commitment, "Five."),
        ];

        assert_eq!(validate(five_across, &total).len(), 3);
    }

    /// Test case 14 — duplicates collapse, whatever category they arrive under.
    #[test]
    fn duplicates_collapse() {
        let facts = vec![
            extracted(MemoryCategory::Identity, "The caller is called Ada."),
            extracted(MemoryCategory::Identity, "the caller is called ada. "),
        ];

        assert_eq!(validate(facts, &policy()).len(), 1);

        // The storage key is the content, so the same sentence under two
        // categories is one row whatever happens here. Collapsing it now keeps
        // the decision where it can be seen, rather than in whichever upsert
        // happens to run second.
        let across_categories = vec![
            extracted(MemoryCategory::Situation, "The caller is job hunting."),
            extracted(MemoryCategory::Preference, "The caller is job hunting."),
        ];
        let kept = validate(across_categories, &policy());

        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].category, MemoryCategory::Situation);
    }

    /// Test case 15 — unparseable output changes nothing.
    #[tokio::test]
    async fn unparseable_output_changes_nothing() {
        let store = Arc::new(SpyMemory::holding(vec![MemoryFact {
            user_id: 7,
            character_id: 11,
            category: MemoryCategory::Identity,
            content: "The caller is called Ada.".into(),
            first_seen_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            source_session_id: "earlier".into(),
        }]));
        let service = service_with(
            store.clone(),
            ScriptedGateway::saying("Sure! Here is what I learned about them:"),
            policy(),
        );

        service.extract("session-1").await;

        assert!(store.never_written());
    }

    /// Test case 16 — a gateway failure changes nothing.
    #[tokio::test]
    async fn a_gateway_failure_changes_nothing() {
        let store = Arc::new(SpyMemory::default());
        let service = service_with(store.clone(), ScriptedGateway::broken(), policy());

        service.extract("session-1").await;

        assert!(store.never_written());
    }

    /// Test case 17 — an empty extracted set does not wipe memory.
    #[tokio::test]
    async fn an_empty_extracted_set_does_not_wipe_memory() {
        let store = Arc::new(SpyMemory::default());
        let service = service_with(
            store.clone(),
            ScriptedGateway::saying(r#"{"facts":[]}"#),
            policy(),
        );

        service.extract("session-1").await;

        assert!(store.never_written());
    }
}
