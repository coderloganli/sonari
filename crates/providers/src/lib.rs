//! The implementations behind the voice provider traits.
//!
//! Recognition and synthesis are reached at ElevenLabs (ADR-0014). Voice
//! activity detection is the one model that stays in-process, because it runs on
//! every frame and drives interruption.
//!
//! Every adapter holds its own credential, taken from the environment at
//! construction. None appears in a port signature.

mod elevenlabs_asr;
mod elevenlabs_tts;
mod sherpa;

pub use elevenlabs_asr::{AsrConfig, ElevenLabsAsrEngine};
pub use elevenlabs_tts::{ElevenLabsTtsEngine, TtsConfig};
pub use sherpa::{SherpaVad, VadConfig};
