//! The service's own HTTP surface, used the way a client uses it.
//!
//! The live solver is a caller, not a component: it creates a session, starts a
//! call, joins the room it is given, and afterwards reads the call's recorded
//! events back. Nothing here is a test hook — every endpoint is one a real
//! client already uses, except the admin timeline, which exists for exactly this
//! kind of after-the-fact reading (ADR-0017).

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::solver::timeline::TimelineEvent;

pub struct Api {
    http: reqwest::Client,
    base_url: String,
    access_token: Option<String>,
    /// The call timeline sits behind admin authorisation, and a caller's token
    /// is not one — correctly, since a caller has no business reading anyone's
    /// call events. Supplied separately rather than escalating the caller.
    admin_token: Option<String>,
}

/// Where to join, and as whom.
#[derive(Debug, Clone, Deserialize)]
pub struct Realtime {
    pub endpoint: String,
    pub room_name: String,
    pub access_token: String,
    pub participant_identity: String,
    pub bot_participant_identity: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StartedCall {
    pub session_id: i64,
    pub realtime: Realtime,
}

impl Api {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            access_token: None,
            admin_token: std::env::var("SONARI_ADMIN_TOKEN")
                .ok()
                .filter(|value| !value.trim().is_empty()),
        }
    }

    /// There is no login: a uid is presented and a token comes back.
    pub async fn create_session(&mut self, uid: &str) -> Result<()> {
        #[derive(Deserialize)]
        struct Tokens {
            access_token: String,
        }

        let tokens: Tokens = self
            .post("/api/session", serde_json::json!({ "uid": uid }))
            .await
            .context("failed to create a session")?;
        self.access_token = Some(tokens.access_token);
        Ok(())
    }

    pub async fn start_call(&self, character_id: i64) -> Result<StartedCall> {
        self.post(
            &format!("/api/call/{character_id}/start"),
            serde_json::json!({}),
        )
        .await
        .context("failed to start a call")
    }

    pub async fn end_call(&self, session_id: i64) -> Result<()> {
        let _: serde_json::Value = self
            .post(
                "/api/call/end",
                serde_json::json!({ "session_id": session_id }),
            )
            .await
            .context("failed to end the call")?;
        Ok(())
    }

    /// The call's recorded events. This is where every marker comes from — the
    /// solver never instruments the service, it reads what the service already
    /// publishes.
    pub async fn timeline(&self, session_id: i64) -> Result<Vec<TimelineEvent>> {
        let token = self.admin_token.as_deref().context(
            "SONARI_ADMIN_TOKEN must be set: the call timeline is an admin surface,              and every marker the live solver reports comes from it",
        )?;
        let request = self
            .http
            .get(format!(
                "{}/api/admin/call-logs/{session_id}/timeline",
                self.base_url
            ))
            .bearer_auth(token);
        Self::read(request)
            .await
            .context("failed to read the call timeline")
    }

    async fn post<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: serde_json::Value,
    ) -> Result<T> {
        let mut request = self
            .http
            .post(format!("{}{path}", self.base_url))
            .json(&body);
        if let Some(token) = &self.access_token {
            request = request.bearer_auth(token);
        }
        Self::read(request).await
    }

    /// Responses are wrapped; the payload sits under `data`. A failure names the
    /// status and the body, because "the harness could not reach the service" is
    /// a different problem from "the service refused", and a run that cannot
    /// tell them apart wastes an afternoon.
    async fn read<T: serde::de::DeserializeOwned>(request: reqwest::RequestBuilder) -> Result<T> {
        let response = request
            .send()
            .await
            .context("the request did not complete")?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!("the service answered {status}: {body}");
        }

        let value: serde_json::Value =
            serde_json::from_str(&body).with_context(|| format!("unreadable response: {body}"))?;
        let payload = value.get("data").cloned().unwrap_or(value);
        serde_json::from_value(payload)
            .with_context(|| format!("unexpected response shape: {body}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wrapper is unwrapped, so callers see the payload rather than the
    /// envelope.
    #[test]
    fn a_wrapped_payload_is_unwrapped() {
        let body = serde_json::json!({
            "data": {
                "session_id": 42,
                "realtime": {
                    "endpoint": "ws://livekit:7880",
                    "room_name": "call-42",
                    "access_token": "token-42",
                    "participant_identity": "user-1",
                    "bot_participant_identity": "bot-42",
                },
            },
        });

        let payload = body.get("data").cloned().expect("a data field");
        let call: StartedCall = serde_json::from_value(payload).expect("the documented shape");

        assert_eq!(call.session_id, 42);
        assert_eq!(call.realtime.room_name, "call-42");
        assert_eq!(call.realtime.bot_participant_identity, "bot-42");
    }
}
