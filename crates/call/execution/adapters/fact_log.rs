//! Runtime facts are handed straight to their consumer.
//!
//! Producer and consumer live in the same process, so there is no stream to
//! write to and no consumer group to read it back.

use std::sync::Arc;

use async_trait::async_trait;
use call_runtime_control::RuntimeEventFact;
use shared_kernel::AppResult;

use crate::application::RuntimeFactLogPort;

/// Consumes a runtime fact. Implemented by the bot speech service; declared
/// here so that `call-execution` does not depend on `call-control`.
#[async_trait]
pub trait RuntimeFactConsumer: Send + Sync {
    async fn consume(&self, fact: RuntimeEventFact) -> AppResult<()>;
}

#[derive(Clone)]
pub struct RuntimeFactLogAdapter {
    consumer: Arc<dyn RuntimeFactConsumer>,
}

impl RuntimeFactLogAdapter {
    pub fn new(consumer: Arc<dyn RuntimeFactConsumer>) -> Self {
        Self { consumer }
    }
}

#[async_trait]
impl RuntimeFactLogPort for RuntimeFactLogAdapter {
    async fn append(&self, fact: &RuntimeEventFact) -> AppResult<()> {
        self.consumer.consume(fact.clone()).await
    }
}
