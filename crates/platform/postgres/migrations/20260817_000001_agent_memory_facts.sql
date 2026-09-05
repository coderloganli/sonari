-- What the agent remembers about a caller (ADR-0021).
--
-- Rows, not a vector index: the set is injected whole and never searched, so
-- there is nothing to embed. The row is structured; the sentence is not, because
-- what is worth remembering about a person is an open set.
--
-- Keyed on the caller and the persona together (ADR-0023): what one persona was
-- told, another does not know.
create table if not exists agent_memory_facts (
  id                bigserial primary key,
  user_id           bigint      not null,
  character_id      bigint      not null,
  category          text        not null,
  content           text        not null,
  -- Kept across a rewrite that keeps the fact, so "known since" survives the
  -- model restating it in different words.
  first_seen_at     timestamptz not null,
  updated_at        timestamptz not null,
  source_session_id text        not null,
  constraint agent_memory_facts_category
    check (category in ('identity', 'relationship', 'preference', 'situation', 'commitment')),
  -- Makes reconciliation three statements rather than a diff in Rust.
  constraint agent_memory_facts_unique unique (user_id, character_id, content)
);

create index if not exists agent_memory_facts_owner
  on agent_memory_facts (user_id, character_id);
