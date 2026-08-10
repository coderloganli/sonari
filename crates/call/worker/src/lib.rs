//! The media plane: it owns the audio path for every active call.
//!
//! It runs as a task inside the server process. `run` drives the dispatch loop;
//! everything it needs from the control plane is passed in as a use-case handle,
//! not reached over the network.

mod agent_client;
mod client;
mod config;
mod external_audio;
mod input;
mod local_orchestration;
mod mixer;
mod pipeline;
mod playback;
mod preprocess;
mod runtime;
mod speech_events;

mod worker;

pub use config::WorkerConfig;
pub use worker::run;
