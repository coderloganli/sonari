use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SupplierType {
    Asr,
    Tts,
}

impl SupplierType {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "asr" => Some(Self::Asr),
            "tts" => Some(Self::Tts),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Asr => "asr",
            Self::Tts => "tts",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKey {
    Alibaba,
    Elevenlabs,
}

impl ProviderKey {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "alibaba" => Some(Self::Alibaba),
            "elevenlabs" => Some(Self::Elevenlabs),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Alibaba => "alibaba",
            Self::Elevenlabs => "elevenlabs",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AsrLanguage {
    Zh,
    En,
}

impl AsrLanguage {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "zh" => Some(Self::Zh),
            "en" => Some(Self::En),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Zh => "zh",
            Self::En => "en",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TtsVoice {
    pub provider: ProviderKey,
    pub voice_id: String,
    pub name: String,
}
