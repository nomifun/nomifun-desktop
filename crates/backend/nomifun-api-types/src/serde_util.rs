//! Small shared serde helpers for the DTO layer.

/// Deserialize a session `target_id` from EITHER a JSON string or a JSON
/// integer, always yielding a `String`.
///
/// The frontend models conversation/terminal ids as numbers (the numeric-id
/// spec) and POSTs `{"target_id": 123, ...}` on the discriminated-target
/// endpoints (会话→自动工作 `POST /api/requirements/autowork`, 会话→智能决策
/// `POST /api/idmm`), while the backend keeps `target_id` as an opaque string
/// handle. Without this, serde rejects the integer with "invalid type: integer
/// N, expected a string" — which the handlers map to `AppError::BadRequest`,
/// surfacing as the 400 users hit when enabling either feature. Accepting both
/// shapes keeps every already-shipped client working and is forward-compatible
/// with clients that send a string.
///
/// Shared by [`crate::requirement::AutoWorkConfigRequest`] and
/// [`crate::idmm::SetIdmmRequest`]; keep it the single source of truth so the
/// two endpoints never drift.
pub(crate) fn deserialize_target_id<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct TargetIdVisitor;
    impl<'de> serde::de::Visitor<'de> for TargetIdVisitor {
        type Value = String;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a session id as a string or integer")
        }
        fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<String, E> {
            Ok(v.to_owned())
        }
        fn visit_string<E: serde::de::Error>(self, v: String) -> Result<String, E> {
            Ok(v)
        }
        fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<String, E> {
            Ok(v.to_string())
        }
        fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<String, E> {
            Ok(v.to_string())
        }
    }
    deserializer.deserialize_any(TargetIdVisitor)
}
