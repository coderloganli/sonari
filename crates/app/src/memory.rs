//! Getting extraction off the turn path.
//!
//! ADR-0022 says the turn schedules and never waits. This is the only place that
//! knows how that happens — the conversation core decides *when*, the composition
//! root decides *how*. It is also where the one-at-a-time rule lives: a schedule
//! arriving while that session is already extracting is dropped, because the run
//! already going will read the same turns plus more.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use agent::{MemoryExtractionScheduler, MemoryService};

pub struct SpawningExtractionScheduler {
    memory: Arc<MemoryService>,
    in_flight: Arc<Mutex<HashSet<String>>>,
}

impl SpawningExtractionScheduler {
    pub fn new(memory: Arc<MemoryService>) -> Self {
        Self {
            memory,
            in_flight: Arc::new(Mutex::new(HashSet::new())),
        }
    }
}

impl MemoryExtractionScheduler for SpawningExtractionScheduler {
    fn schedule(&self, session_id: &str) {
        {
            let mut in_flight = match self.in_flight.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            if !in_flight.insert(session_id.to_owned()) {
                tracing::info!(
                    session_id,
                    "memory extraction already running for this session; skipped"
                );
                return;
            }
        }
        let memory = self.memory.clone();
        let in_flight = self.in_flight.clone();
        let session_id = session_id.to_owned();
        tokio::spawn(async move {
            memory.extract(&session_id).await;
            match in_flight.lock() {
                Ok(mut guard) => guard.remove(&session_id),
                Err(poisoned) => poisoned.into_inner().remove(&session_id),
            };
        });
    }
}
