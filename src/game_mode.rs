use serde::{
    Deserialize, 
    Serialize
};

use serde::de::{
    DeserializeSeed, 
    IntoDeserializer
};

use refx_pp::model::mode::GameMode as RefxGameMode;

use rosu_mods::{
    GameMode as RosuGameMode,

    GameMods as GameModsLazer,
    GameModsIntermode,
    GameModsLegacy,

    serde::GameModSeed,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum GameMode {
    Osu = 0,
    Taiko = 1,
    Catch = 2,
    Mania = 3,

    OsuRx = 4,
    TaikoRx = 5,
    CatchRx = 6,

    OsuAp = 8,

    OsuCheat = 12,
    OsuCheatCheat = 16,
    TD = 20,
}

const RELAX: u32 = 1 << 7;

impl GameMode {
    pub const fn base_mode(self) -> Self {
        match self {
            Self::Osu | Self::OsuRx | Self::OsuAp | Self::OsuCheat | Self::OsuCheatCheat | Self::TD => Self::Osu,
            Self::Taiko | Self::TaikoRx => Self::Taiko,
            Self::Catch | Self::CatchRx => Self::Catch,
            Self::Mania => Self::Mania,
        }
    }

    pub const fn is_relax(self) -> bool {
        matches!(
            self, 
            Self::OsuRx | 
            Self::TaikoRx | 
            Self::CatchRx |

            // NOTE: streams too easy, nerf stream 
            Self::OsuCheat |
            Self::OsuCheatCheat
        )
    }

    pub const fn to_refx_mode(self) -> RefxGameMode {
        match self.base_mode() {
            Self::Osu => RefxGameMode::Osu,
            Self::Taiko => RefxGameMode::Taiko,
            Self::Catch => RefxGameMode::Catch,
            Self::Mania => RefxGameMode::Mania,

            _ => unimplemented!(),
        }
    }

    pub const fn to_rosu_mode(self) -> RosuGameMode {
        match self.base_mode() {
            Self::Osu => RosuGameMode::Osu,
            Self::Taiko => RosuGameMode::Taiko,
            Self::Catch => RosuGameMode::Catch,
            Self::Mania => RosuGameMode::Mania,

            _ => unimplemented!(),
        }
    }
}

impl TryFrom<u8> for GameMode {
    type Error = InvalidGameModeError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Osu),
            1 => Ok(Self::Taiko),
            2 => Ok(Self::Catch),
            3 => Ok(Self::Mania),

            4 => Ok(Self::OsuRx),
            5 => Ok(Self::TaikoRx),
            6 => Ok(Self::CatchRx),
            8 => Ok(Self::OsuAp),

            12 => Ok(Self::OsuCheat),
            16 => Ok(Self::OsuCheatCheat),
            20 => Ok(Self::TD),
            _ => Err(InvalidGameModeError(value)),
        }
    }
}

impl From<GameMode> for u8 {
    fn from(mode: GameMode) -> Self {
        mode as u8
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidGameModeError(pub u8);

impl std::fmt::Display for InvalidGameModeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Invalid game mode: {}", self.0)
    }
}

impl std::error::Error for InvalidGameModeError {}

#[derive(Debug, Clone)]
pub enum GameMods {
    Legacy(GameModsLegacy),
    Intermode(GameModsIntermode),
    Lazer(GameModsLazer),
}

impl GameMods {
    pub fn from_legacy_bits(bits: u32) -> Self {
        Self::Legacy(GameModsLegacy::from_bits(bits))
    }

    pub fn from_acronyms(acronyms: &str) -> Self {
        let intermode = GameModsIntermode::from_acronyms(acronyms);

        match intermode.checked_bits() {
            Some(bits) => Self::Legacy(GameModsLegacy::from_bits(bits)),
            None => Self::Intermode(intermode),
        }
    }

    pub fn from_json_str(json_str: &str, mode: GameMode) -> Result<Self, GameModParseError> {
        let seed = GameModSeed::Mode {
            mode: mode.to_rosu_mode(),
            deny_unknown_fields: false,
        };

        if json_str.starts_with('[') {
            Self::parse_json_array(json_str, seed)
        } else {
            Self::parse_json_object(json_str, seed)
        }
    }

    fn parse_json_array(json_str: &str, seed: GameModSeed) -> Result<Self, GameModParseError> {
        let values: Vec<serde_json::Value> = serde_json::from_str(json_str)
            .map_err(|e| GameModParseError::InvalidJson(e.to_string()))?;

        let mut mods = GameModsLazer::new();

        for value in values {
            let deserializer = match value {
                serde_json::Value::String(_) | serde_json::Value::Number(_) => value.into_deserializer(),
                _ => value.into_deserializer(),
            };

            let mod_result = seed.deserialize(deserializer)
                .map_err(|e| GameModParseError::DeserializationFailed(e.to_string()))?;

            mods.insert(mod_result);
        }

        Ok(Self::Lazer(mods))
    }

    fn parse_json_object(json_str: &str, seed: GameModSeed) -> Result<Self, GameModParseError> {
        let value = serde_json::from_str::<serde_json::Value>(json_str)
            .map_err(|e| GameModParseError::InvalidJson(e.to_string()))?;

        let mod_result = seed.deserialize(value.into_deserializer())
            .map_err(|e| GameModParseError::DeserializationFailed(e.to_string()))?;

        let mut mods = GameModsLazer::new();
        mods.insert(mod_result);

        Ok(Self::Lazer(mods))
    }

    pub fn apply_mode_specific_mods(self, mode: GameMode) -> Self {
        if mode.is_relax() {
            match self {
                Self::Legacy(legacy_mods) => {
                    Self::Legacy(GameModsLegacy::from_bits(legacy_mods.bits() | RELAX))
                }
                other => other,
            }
        } else {
            self
        }
    }
}

impl Default for GameMods {
    fn default() -> Self {
        Self::Legacy(GameModsLegacy::NoMod)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameModParseError {
    InvalidJson(String),
    DeserializationFailed(String),
}

impl std::fmt::Display for GameModParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidJson(msg) => write!(f, "Invalid JSON: {}", msg),
            Self::DeserializationFailed(msg) => write!(f, "Deserialization failed: {}", msg),
        }
    }
}

impl std::error::Error for GameModParseError {}

pub fn parse_mods(input: &str, mode: GameMode) -> Result<GameMods, GameModParseError> {
    let trimmed = input.trim();

    if trimmed.is_empty() {
        return Ok(GameMods::default());
    }

    let mods = if trimmed.starts_with('[') || trimmed.starts_with('{') {
        GameMods::from_json_str(trimmed, mode)?
    } else if let Ok(bits) = trimmed.parse::<u32>() {
        GameMods::from_legacy_bits(bits)
    } else {
        GameMods::from_acronyms(trimmed)
    };

    Ok(mods.apply_mode_specific_mods(mode))
}
