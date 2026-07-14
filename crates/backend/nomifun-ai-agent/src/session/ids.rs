use std::fmt;

macro_rules! newtype_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(s: impl Into<String>) -> Self {
                Self(s.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_owned())
            }
        }
    };
}

newtype_id!(ConversationId);
newtype_id!(SessionId);
newtype_id!(ModeId);
newtype_id!(ModelId);
newtype_id!(ConfigKey);
newtype_id!(ConfigValue);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newtype_roundtrip() {
        let id = ConversationId::new("conv-123");
        assert_eq!(id.as_str(), "conv-123");
        assert_eq!(id.to_string(), "conv-123");
        assert_eq!(id.into_inner(), "conv-123");
    }

    #[test]
    fn newtype_equality() {
        let a = ModeId::new("plan");
        let b = ModeId::new("plan");
        assert_eq!(a, b);
    }

    #[test]
    fn newtype_from_string() {
        let id: SessionId = "sess-1".into();
        assert_eq!(id.as_str(), "sess-1");
    }

    #[test]
    fn newtype_serde_transparent() {
        let id = ModelId::new("claude-4");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"claude-4\"");

        let deserialized: ModelId = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.as_str(), "claude-4");
    }

    #[test]
    fn config_key_value_as_hashmap_key() {
        use std::collections::HashMap;
        let mut map: HashMap<ConfigKey, ConfigValue> = HashMap::new();
        map.insert(ConfigKey::new("reasoning"), ConfigValue::new("high"));
        assert_eq!(map.get(&ConfigKey::new("reasoning")).unwrap().as_str(), "high");
    }
}
