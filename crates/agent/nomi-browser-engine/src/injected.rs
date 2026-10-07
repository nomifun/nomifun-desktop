//! Bundled Playwright semantic core. The native/attached drivers own per-document
//! worlds and remote-object lifetime; there is no second injection manager.
//! Vendor attribution and pinned source remain in injected/NOTICE and the bundle.
pub const INJECTED_SOURCE: &str = include_str!("../injected/dist/injected.js");
pub(crate) const INJECTED_GLOBAL: &str = "__nomiInjectedExports";
pub(crate) fn injected_options_json() -> serde_json::Value {
    serde_json::json!({
        "isUnderTest": false,
        "sdkLanguage": "javascript",
        "testIdAttributeName": "data-testid",
        "stableRafCount": 2,
        "browserName": "chromium",
        "isUtilityWorld": true,
        "customEngines": [],
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundled_semantic_core_and_options_match() {
        assert!(INJECTED_SOURCE.contains(INJECTED_GLOBAL));
        assert!(INJECTED_SOURCE.contains("InjectedScript"));
        let options = injected_options_json();
        assert_eq!(options["browserName"], "chromium");
        assert_eq!(options["stableRafCount"], 2);
        assert_eq!(options["isUtilityWorld"], true);
        assert_eq!(options["customEngines"], serde_json::json!([]));
    }
}
