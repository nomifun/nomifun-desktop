use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CompactionLevel {
    Off,
    #[default]
    Safe,
    Full,
}

impl fmt::Display for CompactionLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Off => write!(f, "off"),
            Self::Safe => write!(f, "safe"),
            Self::Full => write!(f, "full"),
        }
    }
}

impl FromStr for CompactionLevel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "off" => Ok(Self::Off),
            "safe" => Ok(Self::Safe),
            "full" => Ok(Self::Full),
            other => Err(format!(
                "unknown compaction level: '{other}' (expected: off, safe, full)"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_safe() {
        assert_eq!(CompactionLevel::default(), CompactionLevel::Safe);
    }

    #[test]
    fn display_fromstr_roundtrip() {
        for level in [
            CompactionLevel::Off,
            CompactionLevel::Safe,
            CompactionLevel::Full,
        ] {
            let s = level.to_string();
            let parsed: CompactionLevel = s.parse().unwrap();
            assert_eq!(parsed, level);
        }
    }

    #[test]
    fn case_insensitive_parsing() {
        assert_eq!(
            "OFF".parse::<CompactionLevel>().unwrap(),
            CompactionLevel::Off
        );
        assert_eq!(
            "Safe".parse::<CompactionLevel>().unwrap(),
            CompactionLevel::Safe
        );
        assert_eq!(
            "FULL".parse::<CompactionLevel>().unwrap(),
            CompactionLevel::Full
        );
    }

    #[test]
    fn invalid_input_error() {
        let err = "unknown".parse::<CompactionLevel>().unwrap_err();
        assert!(err.contains("unknown compaction level"));
    }

    #[test]
    fn serde_roundtrip() {
        for level in [
            CompactionLevel::Off,
            CompactionLevel::Safe,
            CompactionLevel::Full,
        ] {
            let json = serde_json::to_string(&level).unwrap();
            let back: CompactionLevel = serde_json::from_str(&json).unwrap();
            assert_eq!(back, level);
        }
    }

    #[test]
    fn serde_lowercase_format() {
        assert_eq!(
            serde_json::to_string(&CompactionLevel::Off).unwrap(),
            "\"off\""
        );
        assert_eq!(
            serde_json::to_string(&CompactionLevel::Safe).unwrap(),
            "\"safe\""
        );
        assert_eq!(
            serde_json::to_string(&CompactionLevel::Full).unwrap(),
            "\"full\""
        );
    }
}
