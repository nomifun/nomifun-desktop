//! Tests for the site-memory module (P7A).

use nomi_browser::site_memory::{key_for, InMemorySink, SiteMemoryEntry, SiteMemoryStore};
use std::collections::HashMap;

#[test]
fn etld1_key_groups_subdomains() {
    // mail.google.com and drive.google.com share eTLD+1 "google.com".
    let k1 = key_for("https://mail.google.com/x");
    let k2 = key_for("https://drive.google.com/y");
    assert_eq!(k1, k2);
    assert_eq!(k1, Some("google.com".to_string()));

    // co.uk multi-level suffix: a.co.uk and b.co.uk are DISTINCT eTLD+1s.
    let ka = key_for("https://www.a.co.uk/page");
    let kb = key_for("https://www.b.co.uk/page");
    assert_ne!(ka, kb);
    assert_eq!(ka, Some("a.co.uk".to_string()));
    assert_eq!(kb, Some("b.co.uk".to_string()));

    // IP / localhost → None (no registrable domain).
    assert_eq!(key_for("http://127.0.0.1/foo"), None);
    assert_eq!(key_for("http://localhost:3000/bar"), None);
}

#[test]
fn record_then_query_returns_hint() {
    let sink = InMemorySink::new();
    let store = SiteMemoryStore::new(Box::new(sink));

    let entry = SiteMemoryEntry {
        etld1: "google.com".into(),
        url_pattern: "https://mail.google.com/inbox".into(),
        intent: "click".into(),
        role: "button".into(),
        accessible_name: "Compose".into(),
        selector: Some("div[gh=cm]".into()),
        from_secret: false,
    };
    store.record(entry.clone());

    let results = store.query("google.com");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].role, "button");
    assert_eq!(results[0].accessible_name, "Compose");
    assert_eq!(results[0].selector, Some("div[gh=cm]".to_string()));

    // Different eTLD+1 returns empty.
    let results2 = store.query("github.com");
    assert!(results2.is_empty());
}

#[test]
fn record_skips_secret_sourced_descriptor() {
    let sink = InMemorySink::new();
    let store = SiteMemoryStore::new(Box::new(sink));

    // Case 1: from_secret = true → dropped.
    let secret_entry = SiteMemoryEntry {
        etld1: "bank.com".into(),
        url_pattern: "https://bank.com/login".into(),
        intent: "type".into(),
        role: "textbox".into(),
        accessible_name: "Password".into(),
        selector: Some("#pw".into()),
        from_secret: true,
    };
    store.record(secret_entry);
    assert!(store.query("bank.com").is_empty(), "from_secret=true must be dropped");

    // Case 2: accessible_name is a redaction placeholder → dropped.
    let redacted_entry = SiteMemoryEntry {
        etld1: "bank.com".into(),
        url_pattern: "https://bank.com/login".into(),
        intent: "click".into(),
        role: "textbox".into(),
        accessible_name: "[KNOWN_SECRET_REDACTED]".into(),
        selector: Some("#secret-field".into()),
        from_secret: false,
    };
    store.record(redacted_entry);
    assert!(store.query("bank.com").is_empty(), "redaction placeholder must be dropped");

    // Case 3: Another redaction marker variant.
    let redacted_entry2 = SiteMemoryEntry {
        etld1: "bank.com".into(),
        url_pattern: "https://bank.com/login".into(),
        intent: "type".into(),
        role: "textbox".into(),
        accessible_name: "OTP [REDACTED]".into(),
        selector: None,
        from_secret: false,
    };
    store.record(redacted_entry2);
    assert!(store.query("bank.com").is_empty(), "[REDACTED] in name must be dropped");

    // Case 4: Normal (non-secret) entry IS persisted.
    let normal_entry = SiteMemoryEntry {
        etld1: "bank.com".into(),
        url_pattern: "https://bank.com/dashboard".into(),
        intent: "click".into(),
        role: "button".into(),
        accessible_name: "Transfer".into(),
        selector: Some("#transfer-btn".into()),
        from_secret: false,
    };
    store.record(normal_entry);
    let results = store.query("bank.com");
    assert_eq!(results.len(), 1, "non-secret entry should persist");
    assert_eq!(results[0].accessible_name, "Transfer");
}

#[test]
fn stale_descriptor_invalidated_on_role_mismatch() {
    let sink = InMemorySink::new();
    let store = SiteMemoryStore::new(Box::new(sink));

    // Record two entries with selectors.
    let entry_a = SiteMemoryEntry {
        etld1: "example.com".into(),
        url_pattern: "https://example.com/page".into(),
        intent: "click".into(),
        role: "button".into(),
        accessible_name: "Submit".into(),
        selector: Some("#submit-btn".into()),
        from_secret: false,
    };
    let entry_b = SiteMemoryEntry {
        etld1: "example.com".into(),
        url_pattern: "https://example.com/page".into(),
        intent: "click".into(),
        role: "link".into(),
        accessible_name: "Help".into(),
        selector: Some("a.help".into()),
        from_secret: false,
    };
    store.record(entry_a);
    store.record(entry_b);
    assert_eq!(store.query("example.com").len(), 2);

    // Current observe: #submit-btn is now a "link" with name "Back" (role mismatch → stale).
    // a.help still matches.
    let mut current_by_selector = HashMap::new();
    current_by_selector.insert("#submit-btn".to_string(), ("link".to_string(), "Back".to_string()));
    current_by_selector.insert("a.help".to_string(), ("link".to_string(), "Help".to_string()));

    store.reconcile("example.com", &current_by_selector);

    let remaining = store.query("example.com");
    assert_eq!(remaining.len(), 1, "stale entry should be dropped");
    assert_eq!(remaining[0].accessible_name, "Help");
    assert_eq!(remaining[0].selector, Some("a.help".to_string()));
}
