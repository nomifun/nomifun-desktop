//! Integration tests for dependency management (test-plan DM-1 through DM-9).
//!
//! Black-box tests exercising the public API surface of
//! `nomifun_extension::dependency`.

use std::collections::HashMap;

use nomifun_extension::{
    DependencyIssue, ExtensionManifest, ExtensionSource, ExtensionState, LoadedExtension, validate_dependencies,
};

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn ext(name: &str, version: &str, deps: &[(&str, &str)]) -> LoadedExtension {
    LoadedExtension {
        manifest: ExtensionManifest {
            name: name.to_string(),
            version: version.to_string(),
            display_name: None,
            description: None,
            author: None,
            license: None,
            homepage: None,
            icon: None,
            engine: None,
            api_version: None,
            dependencies: deps
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<HashMap<_, _>>(),
            entry_point: None,
            permissions: None,
            contributes: None,
            lifecycle: None,
            i18n: None,
        },
        directory: format!("/extensions/{name}"),
        source: ExtensionSource::Local,
        state: ExtensionState {
            name: name.to_string(),
            version: version.to_string(),
            enabled: true,
            installed_at: None,
            last_activated_at: None,
        },
    }
}

// ---------------------------------------------------------------------------
// DM-1: satisfied deps load in order
// ---------------------------------------------------------------------------

#[test]
fn dm1_satisfied_deps_load_in_order() {
    let exts = vec![
        ext("ext-a", "1.0.0", &[("ext-b", "^1.0.0")]),
        ext("ext-b", "1.2.0", &[]),
    ];
    let result = validate_dependencies(&exts);
    assert!(result.valid);
    assert!(result.issues.is_empty());
    assert_eq!(result.load_order, vec!["ext-b", "ext-a"]);
}

// ---------------------------------------------------------------------------
// DM-2: missing dependency detection
// ---------------------------------------------------------------------------

#[test]
fn dm2_missing_dependency_detected() {
    let exts = vec![ext("ext-a", "1.0.0", &[("ext-missing", "^1.0.0")])];
    let result = validate_dependencies(&exts);
    assert!(!result.valid);
    assert_eq!(result.issues.len(), 1);
    match &result.issues[0] {
        DependencyIssue::Missing {
            extension,
            dependency,
            required,
        } => {
            assert_eq!(extension, "ext-a");
            assert_eq!(dependency, "ext-missing");
            assert_eq!(required, "^1.0.0");
        }
        other => panic!("expected Missing issue, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// DM-3: version mismatch detection (exact)
// ---------------------------------------------------------------------------

#[test]
fn dm3_exact_version_mismatch() {
    let exts = vec![ext("ext-b", "1.5.0", &[]), ext("ext-a", "1.0.0", &[("ext-b", "2.0.0")])];
    let result = validate_dependencies(&exts);
    assert!(!result.valid);
    let mismatch = result
        .issues
        .iter()
        .find(|i| matches!(i, DependencyIssue::VersionMismatch { .. }))
        .expect("should contain VersionMismatch");
    match mismatch {
        DependencyIssue::VersionMismatch { required, actual, .. } => {
            assert_eq!(required, "2.0.0");
            assert_eq!(actual, "1.5.0");
        }
        _ => unreachable!(),
    }
}

// ---------------------------------------------------------------------------
// DM-4: caret (^) match success
// ---------------------------------------------------------------------------

#[test]
fn dm4_caret_match_succeeds() {
    let exts = vec![
        ext("base", "1.9.0", &[]),
        ext("consumer", "1.0.0", &[("base", "^1.2.3")]),
    ];
    let result = validate_dependencies(&exts);
    assert!(result.valid);
    assert!(result.issues.is_empty());
}

// ---------------------------------------------------------------------------
// DM-5: caret (^) match failure
// ---------------------------------------------------------------------------

#[test]
fn dm5_caret_match_fails() {
    let exts = vec![
        ext("base", "2.0.0", &[]),
        ext("consumer", "1.0.0", &[("base", "^1.2.3")]),
    ];
    let result = validate_dependencies(&exts);
    assert!(!result.valid);
    assert!(
        result
            .issues
            .iter()
            .any(|i| matches!(i, DependencyIssue::VersionMismatch { .. }))
    );
}

// ---------------------------------------------------------------------------
// DM-6: tilde (~) match success
// ---------------------------------------------------------------------------

#[test]
fn dm6_tilde_match_succeeds() {
    let exts = vec![
        ext("base", "1.2.9", &[]),
        ext("consumer", "1.0.0", &[("base", "~1.2.3")]),
    ];
    let result = validate_dependencies(&exts);
    assert!(result.valid);
    assert!(result.issues.is_empty());
}

// ---------------------------------------------------------------------------
// DM-7: tilde (~) match failure
// ---------------------------------------------------------------------------

#[test]
fn dm7_tilde_match_fails() {
    let exts = vec![
        ext("base", "1.3.0", &[]),
        ext("consumer", "1.0.0", &[("base", "~1.2.3")]),
    ];
    let result = validate_dependencies(&exts);
    assert!(!result.valid);
    assert!(
        result
            .issues
            .iter()
            .any(|i| matches!(i, DependencyIssue::VersionMismatch { .. }))
    );
}

// ---------------------------------------------------------------------------
// DM-8: circular dependency detection
// ---------------------------------------------------------------------------

#[test]
fn dm8_circular_dependency_detected() {
    let exts = vec![
        ext("ext-a", "1.0.0", &[("ext-c", "^1.0.0")]),
        ext("ext-b", "1.0.0", &[("ext-a", "^1.0.0")]),
        ext("ext-c", "1.0.0", &[("ext-b", "^1.0.0")]),
    ];
    let result = validate_dependencies(&exts);
    assert!(!result.valid);

    let circulars: Vec<&Vec<String>> = result
        .issues
        .iter()
        .filter_map(|i| match i {
            DependencyIssue::Circular { cycle } => Some(cycle),
            _ => None,
        })
        .collect();
    assert!(!circulars.is_empty(), "should detect at least one cycle");

    // Verify cycle path closes (first == last) and contains all three.
    let cycle = &circulars[0];
    assert_eq!(cycle.first(), cycle.last(), "cycle must close");
    let cycle_members: std::collections::HashSet<&str> = cycle.iter().map(|s| s.as_str()).collect();
    assert!(cycle_members.contains("ext-a"));
    assert!(cycle_members.contains("ext-b"));
    assert!(cycle_members.contains("ext-c"));

    // Still attempt to load all extensions.
    assert_eq!(result.load_order.len(), 3);
}

// ---------------------------------------------------------------------------
// DM-9: extensions with no dependencies
// ---------------------------------------------------------------------------

#[test]
fn dm9_no_deps_all_valid() {
    let exts = vec![
        ext("ext-x", "1.0.0", &[]),
        ext("ext-y", "2.0.0", &[]),
        ext("ext-z", "0.1.0", &[]),
    ];
    let result = validate_dependencies(&exts);
    assert!(result.valid);
    assert!(result.issues.is_empty());
    assert_eq!(result.load_order.len(), 3);
}

// ---------------------------------------------------------------------------
// Additional edge cases
// ---------------------------------------------------------------------------

#[test]
fn mixed_acyclic_and_cyclic_subsets() {
    // c is acyclic; a↔b form a cycle.
    let exts = vec![
        ext("a", "1.0.0", &[("b", "^1.0.0")]),
        ext("b", "1.0.0", &[("a", "^1.0.0")]),
        ext("c", "1.0.0", &[]),
    ];
    let result = validate_dependencies(&exts);
    assert!(!result.valid);

    // c should come before the cyclic pair in load order.
    let pos = |n: &str| result.load_order.iter().position(|x| x == n).unwrap();
    assert!(pos("c") < pos("a"));
    assert!(pos("c") < pos("b"));
}

#[test]
fn large_chain_preserves_order() {
    // e → d → c → b → a
    let exts = vec![
        ext("a", "1.0.0", &[]),
        ext("b", "1.0.0", &[("a", "^1.0.0")]),
        ext("c", "1.0.0", &[("b", "^1.0.0")]),
        ext("d", "1.0.0", &[("c", "^1.0.0")]),
        ext("e", "1.0.0", &[("d", "^1.0.0")]),
    ];
    let result = validate_dependencies(&exts);
    assert!(result.valid);
    assert_eq!(result.load_order, vec!["a", "b", "c", "d", "e"]);
}

#[test]
fn multiple_missing_deps_all_reported() {
    let exts = vec![ext("ext-a", "1.0.0", &[("dep-1", "^1.0.0"), ("dep-2", "~2.0.0")])];
    let result = validate_dependencies(&exts);
    assert!(!result.valid);
    let missing: Vec<_> = result
        .issues
        .iter()
        .filter(|i| matches!(i, DependencyIssue::Missing { .. }))
        .collect();
    assert_eq!(missing.len(), 2);
}
