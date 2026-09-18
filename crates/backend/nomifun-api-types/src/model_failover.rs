//! Provider/model failover policy independent of Agent runtime supervision.

use serde::{Deserialize, Serialize};

fn default_max_switches() -> u32 {
    4
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelFailoverConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub queue: Vec<nomifun_common::ProviderWithModel>,
    #[serde(default = "default_max_switches")]
    pub max_switches: u32,
}

impl<'de> Deserialize<'de> for ModelFailoverConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            #[serde(default)]
            enabled: bool,
            #[serde(default)]
            queue: Vec<nomifun_common::ProviderWithModel>,
            #[serde(default = "default_max_switches")]
            max_switches: u32,
        }

        let wire = Wire::deserialize(deserializer)?;
        if let Some(error) = wire.queue.iter().find_map(|entry| entry.validate().err()) {
            return Err(serde::de::Error::custom(format!(
                "invalid model failover queue entry: {error}"
            )));
        }
        Ok(Self {
            enabled: wire.enabled,
            queue: wire.queue,
            max_switches: wire.max_switches,
        })
    }
}

impl Default for ModelFailoverConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            queue: Vec::new(),
            max_switches: default_max_switches(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_policy_is_disabled_and_bounded() {
        let policy: ModelFailoverConfig = serde_json::from_str("{}").unwrap();
        assert!(!policy.enabled);
        assert!(policy.queue.is_empty());
        assert_eq!(policy.max_switches, 4);
    }
}
