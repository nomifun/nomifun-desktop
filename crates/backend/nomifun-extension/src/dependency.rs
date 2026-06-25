use std::collections::{HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};

use crate::types::LoadedExtension;
#[cfg(test)]
use crate::types::{ExtensionManifest, ExtensionSource, ExtensionState};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single dependency issue found during validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DependencyIssue {
    /// A required dependency is not installed.
    Missing {
        #[serde(rename = "ext")]
        extension: String,
        #[serde(rename = "dep")]
        dependency: String,
        required: String,
    },
    /// A dependency exists but its version does not satisfy the requirement.
    VersionMismatch {
        #[serde(rename = "ext")]
        extension: String,
        #[serde(rename = "dep")]
        dependency: String,
        required: String,
        actual: String,
    },
    /// A cycle was detected in the dependency graph.
    Circular { cycle: Vec<String> },
}

/// Outcome of dependency validation across a set of extensions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyValidationResult {
    /// `true` when no issues were found.
    pub valid: bool,
    /// All detected issues (missing, version mismatch, circular).
    pub issues: Vec<DependencyIssue>,
    /// Topological load order — dependencies before dependents.
    /// Cyclic extensions are appended at the end in alphabetical order.
    pub load_order: Vec<String>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Validate all inter-extension dependencies: missing, version mismatch,
/// circular.  Returns a [`DependencyValidationResult`] containing the issues
/// found and a topological load order.
pub fn validate_dependencies(extensions: &[LoadedExtension]) -> DependencyValidationResult {
    let versions: HashMap<&str, &str> = extensions
        .iter()
        .map(|ext| (ext.manifest.name.as_str(), ext.manifest.version.as_str()))
        .collect();

    let mut issues = Vec::new();

    // 1. Check missing and version mismatches.
    for ext in extensions {
        for (dep_name, dep_req) in &ext.manifest.dependencies {
            match versions.get(dep_name.as_str()) {
                None => {
                    issues.push(DependencyIssue::Missing {
                        extension: ext.manifest.name.clone(),
                        dependency: dep_name.clone(),
                        required: dep_req.clone(),
                    });
                }
                Some(actual_version) => {
                    if !version_matches(dep_req, actual_version) {
                        issues.push(DependencyIssue::VersionMismatch {
                            extension: ext.manifest.name.clone(),
                            dependency: dep_name.clone(),
                            required: dep_req.clone(),
                            actual: actual_version.to_string(),
                        });
                    }
                }
            }
        }
    }

    // 2. Topological sort + cycle detection.
    let (load_order, cycles) = compute_topological_order(extensions);
    for cycle in cycles {
        issues.push(DependencyIssue::Circular { cycle });
    }

    DependencyValidationResult {
        valid: issues.is_empty(),
        issues,
        load_order,
    }
}

// ---------------------------------------------------------------------------
// Version matching
// ---------------------------------------------------------------------------

/// Check whether `actual` satisfies `requirement`.
///
/// - Bare version (`"1.2.3"`) → **exact** match.
/// - Caret (`"^1.2.3"`)       → `>=1.2.3, <2.0.0`.
/// - Tilde (`"~1.2.3"`)       → `>=1.2.3, <1.3.0`.
fn version_matches(requirement: &str, actual: &str) -> bool {
    let Ok(version) = semver::Version::parse(actual) else {
        return false;
    };

    let req_str = if requirement.starts_with(|c: char| c.is_ascii_digit()) {
        // Bare version → exact match via '=' operator.
        format!("={requirement}")
    } else {
        requirement.to_string()
    };

    let Ok(req) = semver::VersionReq::parse(&req_str) else {
        return false;
    };

    req.matches(&version)
}

// ---------------------------------------------------------------------------
// Topological sort internals
// ---------------------------------------------------------------------------

/// Kahn's algorithm with cycle detection.
///
/// Returns `(load_order, detected_cycles)`.  Cyclic nodes are appended to
/// `load_order` in alphabetical order so that the caller can still attempt
/// loading them (API Spec requirement).
fn compute_topological_order(extensions: &[LoadedExtension]) -> (Vec<String>, Vec<Vec<String>>) {
    let known: HashSet<&str> = extensions.iter().map(|e| e.manifest.name.as_str()).collect();

    // adjacency: dependency → vec of dependents
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut in_degree: HashMap<&str, usize> = HashMap::new();

    for ext in extensions {
        in_degree.entry(ext.manifest.name.as_str()).or_insert(0);
        for dep_name in ext.manifest.dependencies.keys() {
            if known.contains(dep_name.as_str()) {
                adj.entry(dep_name.as_str())
                    .or_default()
                    .push(ext.manifest.name.as_str());
                *in_degree.entry(ext.manifest.name.as_str()).or_insert(0) += 1;
            }
        }
    }

    // Seed the queue with zero-in-degree nodes (sorted for determinism).
    let mut queue: VecDeque<&str> = {
        let mut seeds: Vec<&str> = in_degree
            .iter()
            .filter(|(_, deg)| **deg == 0)
            .map(|(&name, _)| name)
            .collect();
        seeds.sort_unstable();
        seeds.into_iter().collect()
    };

    let mut order: Vec<String> = Vec::with_capacity(extensions.len());

    while let Some(node) = queue.pop_front() {
        order.push(node.to_string());
        if let Some(dependents) = adj.get(node) {
            let mut ready = Vec::new();
            for &dep in dependents {
                let deg = in_degree.get_mut(dep).expect("in_degree entry");
                *deg -= 1;
                if *deg == 0 {
                    ready.push(dep);
                }
            }
            ready.sort_unstable();
            for n in ready {
                queue.push_back(n);
            }
        }
    }

    // Nodes not emitted are part of cycles.
    let emitted: HashSet<&str> = order.iter().map(|s| s.as_str()).collect();
    let remaining: HashSet<&str> = known.difference(&emitted).copied().collect();

    let cycles = if remaining.is_empty() {
        Vec::new()
    } else {
        find_cycles(extensions, &remaining)
    };

    // Append cyclic nodes alphabetically (best-effort load).
    let mut remaining_sorted: Vec<String> = remaining.iter().map(|s| s.to_string()).collect();
    remaining_sorted.sort_unstable();
    order.extend(remaining_sorted);

    (order, cycles)
}

/// Find distinct cycles in the subgraph induced by `involved` nodes.
fn find_cycles(extensions: &[LoadedExtension], involved: &HashSet<&str>) -> Vec<Vec<String>> {
    // Build adjacency: node → its dependencies (restricted to involved set).
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
    for ext in extensions {
        let name = ext.manifest.name.as_str();
        if involved.contains(name) {
            let mut deps: Vec<&str> = ext
                .manifest
                .dependencies
                .keys()
                .map(|s| s.as_str())
                .filter(|s| involved.contains(s))
                .collect();
            deps.sort_unstable();
            adj.insert(name, deps);
        }
    }

    let mut visited: HashSet<&str> = HashSet::new();
    let mut on_stack: HashSet<&str> = HashSet::new();
    let mut path: Vec<&str> = Vec::new();
    let mut cycles: Vec<Vec<String>> = Vec::new();

    let mut sorted_nodes: Vec<&str> = involved.iter().copied().collect();
    sorted_nodes.sort_unstable();

    for node in sorted_nodes {
        if !visited.contains(node) {
            dfs_find_cycles(node, &adj, &mut visited, &mut on_stack, &mut path, &mut cycles);
        }
    }

    cycles
}

fn dfs_find_cycles<'a>(
    node: &'a str,
    adj: &HashMap<&'a str, Vec<&'a str>>,
    visited: &mut HashSet<&'a str>,
    on_stack: &mut HashSet<&'a str>,
    path: &mut Vec<&'a str>,
    cycles: &mut Vec<Vec<String>>,
) {
    visited.insert(node);
    on_stack.insert(node);
    path.push(node);

    if let Some(neighbors) = adj.get(node) {
        for &next in neighbors {
            if !visited.contains(next) {
                dfs_find_cycles(next, adj, visited, on_stack, path, cycles);
            } else if on_stack.contains(next) {
                // Extract cycle from path.
                if let Some(start) = path.iter().position(|&n| n == next) {
                    let mut cycle: Vec<String> = path[start..].iter().map(|s| s.to_string()).collect();
                    cycle.push(next.to_string()); // close the cycle
                    cycles.push(cycle);
                }
            }
        }
    }

    path.pop();
    on_stack.remove(node);
}

// ---------------------------------------------------------------------------
// Test helper
// ---------------------------------------------------------------------------

/// Build a minimal [`LoadedExtension`] for testing purposes.
#[cfg(test)]
fn make_extension(name: &str, version: &str, deps: &[(&str, &str)]) -> LoadedExtension {
    use std::collections::HashMap;

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
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- version_matches ---------------------------------------------------

    #[test]
    fn exact_match_same_version() {
        assert!(version_matches("1.2.3", "1.2.3"));
    }

    #[test]
    fn exact_match_different_version() {
        assert!(!version_matches("2.0.0", "1.5.0"));
    }

    #[test]
    fn exact_match_rejects_higher_patch() {
        assert!(!version_matches("1.2.3", "1.2.4"));
    }

    #[test]
    fn caret_match_within_range() {
        // ^1.2.3 allows >=1.2.3, <2.0.0
        assert!(version_matches("^1.2.3", "1.9.0"));
    }

    #[test]
    fn caret_match_at_lower_bound() {
        assert!(version_matches("^1.2.3", "1.2.3"));
    }

    #[test]
    fn caret_match_rejects_next_major() {
        assert!(!version_matches("^1.2.3", "2.0.0"));
    }

    #[test]
    fn caret_match_rejects_below_lower() {
        assert!(!version_matches("^1.2.3", "1.2.2"));
    }

    #[test]
    fn tilde_match_within_range() {
        // ~1.2.3 allows >=1.2.3, <1.3.0
        assert!(version_matches("~1.2.3", "1.2.9"));
    }

    #[test]
    fn tilde_match_at_lower_bound() {
        assert!(version_matches("~1.2.3", "1.2.3"));
    }

    #[test]
    fn tilde_match_rejects_next_minor() {
        assert!(!version_matches("~1.2.3", "1.3.0"));
    }

    #[test]
    fn tilde_match_rejects_below_lower() {
        assert!(!version_matches("~1.2.3", "1.2.0"));
    }

    #[test]
    fn invalid_requirement_returns_false() {
        assert!(!version_matches("not-a-version", "1.0.0"));
    }

    #[test]
    fn invalid_actual_returns_false() {
        assert!(!version_matches("^1.0.0", "not-semver"));
    }

    // -- topological_sort --------------------------------------------------

    #[test]
    fn sort_no_deps() {
        let exts = vec![
            make_extension("alpha", "1.0.0", &[]),
            make_extension("beta", "1.0.0", &[]),
            make_extension("gamma", "1.0.0", &[]),
        ];
        let order = compute_topological_order(&exts).0;
        assert_eq!(order.len(), 3);
        // Alphabetical when no constraints.
        assert_eq!(order, vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn sort_linear_chain() {
        // gamma → beta → alpha
        let exts = vec![
            make_extension("alpha", "1.0.0", &[]),
            make_extension("beta", "1.0.0", &[("alpha", "^1.0.0")]),
            make_extension("gamma", "1.0.0", &[("beta", "^1.0.0")]),
        ];
        let order = compute_topological_order(&exts).0;
        assert_eq!(order, vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn sort_diamond() {
        // d → b, d → c, b → a, c → a
        let exts = vec![
            make_extension("a", "1.0.0", &[]),
            make_extension("b", "1.0.0", &[("a", "^1.0.0")]),
            make_extension("c", "1.0.0", &[("a", "^1.0.0")]),
            make_extension("d", "1.0.0", &[("b", "^1.0.0"), ("c", "^1.0.0")]),
        ];
        let order = compute_topological_order(&exts).0;
        // a must come before b and c; b and c before d.
        let pos = |name: &str| order.iter().position(|n| n == name).unwrap();
        assert!(pos("a") < pos("b"));
        assert!(pos("a") < pos("c"));
        assert!(pos("b") < pos("d"));
        assert!(pos("c") < pos("d"));
    }

    #[test]
    fn sort_with_cycle_still_includes_all() {
        // a → b → c → a (cycle)
        let exts = vec![
            make_extension("a", "1.0.0", &[("c", "^1.0.0")]),
            make_extension("b", "1.0.0", &[("a", "^1.0.0")]),
            make_extension("c", "1.0.0", &[("b", "^1.0.0")]),
        ];
        let order = compute_topological_order(&exts).0;
        assert_eq!(order.len(), 3);
        // All three should be present.
        assert!(order.contains(&"a".to_string()));
        assert!(order.contains(&"b".to_string()));
        assert!(order.contains(&"c".to_string()));
    }

    #[test]
    fn sort_ignores_unknown_deps() {
        // ext-a depends on ext-missing (not in list).
        let exts = vec![make_extension("ext-a", "1.0.0", &[("ext-missing", "^1.0.0")])];
        let order = compute_topological_order(&exts).0;
        assert_eq!(order, vec!["ext-a"]);
    }

    // -- validate_dependencies ---------------------------------------------

    #[test]
    fn valid_satisfied_deps() {
        let exts = vec![
            make_extension("ext-b", "1.2.0", &[]),
            make_extension("ext-a", "1.0.0", &[("ext-b", "^1.0.0")]),
        ];
        let result = validate_dependencies(&exts);
        assert!(result.valid);
        assert!(result.issues.is_empty());
        assert_eq!(result.load_order, vec!["ext-b", "ext-a"]);
    }

    #[test]
    fn detect_missing_dependency() {
        let exts = vec![make_extension("ext-a", "1.0.0", &[("ext-missing", "^1.0.0")])];
        let result = validate_dependencies(&exts);
        assert!(!result.valid);
        assert_eq!(result.issues.len(), 1);
        assert!(matches!(
            &result.issues[0],
            DependencyIssue::Missing {
                extension,
                dependency,
                ..
            } if extension == "ext-a" && dependency == "ext-missing"
        ));
    }

    #[test]
    fn detect_version_mismatch_exact() {
        let exts = vec![
            make_extension("ext-b", "1.5.0", &[]),
            make_extension("ext-a", "1.0.0", &[("ext-b", "2.0.0")]),
        ];
        let result = validate_dependencies(&exts);
        assert!(!result.valid);
        assert!(result.issues.iter().any(|i| matches!(
            i,
            DependencyIssue::VersionMismatch {
                required,
                actual,
                ..
            } if required == "2.0.0" && actual == "1.5.0"
        )));
    }

    #[test]
    fn detect_circular_dependency() {
        let exts = vec![
            make_extension("ext-a", "1.0.0", &[("ext-c", "^1.0.0")]),
            make_extension("ext-b", "1.0.0", &[("ext-a", "^1.0.0")]),
            make_extension("ext-c", "1.0.0", &[("ext-b", "^1.0.0")]),
        ];
        let result = validate_dependencies(&exts);
        assert!(!result.valid);
        let circular = result
            .issues
            .iter()
            .filter(|i| matches!(i, DependencyIssue::Circular { .. }))
            .count();
        assert!(circular >= 1);
        // All nodes still in load_order.
        assert_eq!(result.load_order.len(), 3);
    }

    #[test]
    fn circular_cycle_path_closes() {
        let exts = vec![
            make_extension("a", "1.0.0", &[("c", "^1.0.0")]),
            make_extension("b", "1.0.0", &[("a", "^1.0.0")]),
            make_extension("c", "1.0.0", &[("b", "^1.0.0")]),
        ];
        let result = validate_dependencies(&exts);
        let cycles: Vec<&Vec<String>> = result
            .issues
            .iter()
            .filter_map(|i| match i {
                DependencyIssue::Circular { cycle } => Some(cycle),
                _ => None,
            })
            .collect();
        assert!(!cycles.is_empty());
        // Cycle path must close (first == last).
        for cycle in &cycles {
            assert!(cycle.len() >= 3);
            assert_eq!(cycle.first(), cycle.last());
        }
    }

    #[test]
    fn no_deps_all_valid() {
        let exts = vec![make_extension("x", "1.0.0", &[]), make_extension("y", "2.0.0", &[])];
        let result = validate_dependencies(&exts);
        assert!(result.valid);
        assert!(result.issues.is_empty());
        assert_eq!(result.load_order.len(), 2);
    }

    #[test]
    fn mixed_issues() {
        // ext-a → ext-missing (missing), ext-a → ext-b bad version, ext-c → ext-d → ext-c (cycle)
        let exts = vec![
            make_extension("ext-a", "1.0.0", &[("ext-missing", "^1.0.0"), ("ext-b", "^2.0.0")]),
            make_extension("ext-b", "1.0.0", &[]),
            make_extension("ext-c", "1.0.0", &[("ext-d", "^1.0.0")]),
            make_extension("ext-d", "1.0.0", &[("ext-c", "^1.0.0")]),
        ];
        let result = validate_dependencies(&exts);
        assert!(!result.valid);

        let missing_count = result
            .issues
            .iter()
            .filter(|i| matches!(i, DependencyIssue::Missing { .. }))
            .count();
        let mismatch_count = result
            .issues
            .iter()
            .filter(|i| matches!(i, DependencyIssue::VersionMismatch { .. }))
            .count();
        let circular_count = result
            .issues
            .iter()
            .filter(|i| matches!(i, DependencyIssue::Circular { .. }))
            .count();

        assert_eq!(missing_count, 1);
        assert_eq!(mismatch_count, 1);
        assert!(circular_count >= 1);
        // All 4 extensions still in load order.
        assert_eq!(result.load_order.len(), 4);
    }

    #[test]
    fn empty_extensions_list() {
        let result = validate_dependencies(&[]);
        assert!(result.valid);
        assert!(result.issues.is_empty());
        assert!(result.load_order.is_empty());
    }

    #[test]
    fn caret_match_success_via_validate() {
        let exts = vec![
            make_extension("base", "1.9.0", &[]),
            make_extension("consumer", "1.0.0", &[("base", "^1.2.3")]),
        ];
        let result = validate_dependencies(&exts);
        assert!(result.valid);
    }

    #[test]
    fn caret_match_failure_via_validate() {
        let exts = vec![
            make_extension("base", "2.0.0", &[]),
            make_extension("consumer", "1.0.0", &[("base", "^1.2.3")]),
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

    #[test]
    fn tilde_match_success_via_validate() {
        let exts = vec![
            make_extension("base", "1.2.9", &[]),
            make_extension("consumer", "1.0.0", &[("base", "~1.2.3")]),
        ];
        let result = validate_dependencies(&exts);
        assert!(result.valid);
    }

    #[test]
    fn tilde_match_failure_via_validate() {
        let exts = vec![
            make_extension("base", "1.3.0", &[]),
            make_extension("consumer", "1.0.0", &[("base", "~1.2.3")]),
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

    #[test]
    fn partial_cycle_does_not_block_acyclic_nodes() {
        // a and b form a cycle; c has no deps.
        let exts = vec![
            make_extension("a", "1.0.0", &[("b", "^1.0.0")]),
            make_extension("b", "1.0.0", &[("a", "^1.0.0")]),
            make_extension("c", "1.0.0", &[]),
        ];
        let result = validate_dependencies(&exts);
        // c should appear before the cyclic nodes in load_order.
        let pos_c = result.load_order.iter().position(|n| n == "c").unwrap();
        let pos_a = result.load_order.iter().position(|n| n == "a").unwrap();
        let pos_b = result.load_order.iter().position(|n| n == "b").unwrap();
        assert!(pos_c < pos_a);
        assert!(pos_c < pos_b);
    }
}
