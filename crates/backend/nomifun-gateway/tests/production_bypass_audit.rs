//! Static guard for the SL-S3-06 Browser/Computer bypass migration.
//!
//! This test does not claim the migration is complete. The compatibility
//! Gateway and the standalone computer stdio bridge still contain known
//! concrete implementation references while the central composition is being
//! migrated. The passing guard keeps those references confined to the
//! explicitly named migration surfaces. The ignored test is the eventual
//! clean gate and is intentionally expected to fail until that migration lands.

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy)]
struct Marker {
    name: &'static str,
    needle: &'static str,
}

const DIRECT_BYPASS_MARKERS: &[Marker] = &[
    Marker {
        name: "direct ComputerTool construction",
        needle: "ComputerTool::new(",
    },
    Marker {
        name: "direct ComputerTool execution",
        needle: ".tool.execute(",
    },
    Marker {
        name: "direct ComputerRegistry execution",
        needle: "reg.execute(input).await",
    },
    Marker {
        name: "direct ManagedBrowserFacade construction",
        needle: "ManagedBrowserFacade::new(",
    },
    Marker {
        name: "direct BrowserRegistry dispatch",
        needle: "dispatch_managed(&ctx",
    },
    Marker {
        name: "concrete BrowserSessionHub reference",
        needle: "BrowserSessionHub",
    },
    Marker {
        name: "concrete computer dependency declaration",
        needle: "nomi-computer",
    },
    Marker {
        name: "concrete browser dependency declaration",
        needle: "nomi-browser",
    },
];

const DECLARED_MIGRATION_SURFACES: &[&str] = &[
    "gateway/Cargo.toml",
    "gateway/browser_registry.rs",
    "gateway/caps_browser.rs",
    "gateway/caps_computer.rs",
    "gateway/computer_registry.rs",
    "gateway/deps.rs",
    "gateway/lib.rs",
    "gateway/server.rs",
    "app/src/commands/computer_stdio.rs",
];

fn gateway_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn normalized(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn collect_rust_sources(
    root: &Path,
    directory: &Path,
    output: &mut Vec<(String, String)>,
) {
    let mut entries = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("read source directory {}: {error}", directory.display()))
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_else(|error| panic!("read source entry under {}: {error}", directory.display()));
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_rust_sources(root, &path, output);
            continue;
        }
        if path.extension().and_then(|value| value.to_str()) != Some("rs") {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .unwrap_or_else(|_| panic!("source path escaped {}", root.display()));
        let label = format!("gateway/{}", normalized(relative));
        let contents = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read source file {}: {error}", path.display()));
        output.push((label, contents));
    }
}

fn audited_sources() -> Vec<(String, String)> {
    let root = gateway_root();
    let mut sources = Vec::new();
    collect_rust_sources(&root.join("src"), &root.join("src"), &mut sources);

    let cargo_toml = root.join("Cargo.toml");
    sources.push((
        "gateway/Cargo.toml".to_owned(),
        fs::read_to_string(&cargo_toml)
            .unwrap_or_else(|error| panic!("read {}: {error}", cargo_toml.display())),
    ));

    let app_source = root.join("../nomifun-app/src/commands/computer_stdio.rs");
    sources.push((
        "app/src/commands/computer_stdio.rs".to_owned(),
        fs::read_to_string(&app_source)
            .unwrap_or_else(|error| panic!("read {}: {error}", app_source.display())),
    ));

    sources.sort_by(|left, right| left.0.cmp(&right.0));
    sources
}

fn bypass_hits() -> Vec<String> {
    let mut hits = Vec::new();
    for (path, source) in audited_sources() {
        for marker in DIRECT_BYPASS_MARKERS {
            if let Some((line_number, _)) = source
                .lines()
                .enumerate()
                .find(|(_, line)| line.contains(marker.needle))
            {
                hits.push(format!(
                    "{path}:{}: {}",
                    line_number + 1,
                    marker.name
                ));
            }
        }
    }
    hits.sort();
    hits.dedup();
    hits
}

fn is_declared_migration_surface(path: &str) -> bool {
    DECLARED_MIGRATION_SURFACES
        .iter()
        .any(|allowed| *allowed == path)
}

#[test]
fn known_bypass_inventory_is_confined_to_declared_migration_surfaces() {
    let unexpected = bypass_hits()
        .into_iter()
        .filter(|hit| {
            let path = hit.split(':').next().unwrap_or_default();
            !is_declared_migration_surface(path)
        })
        .collect::<Vec<_>>();

    assert!(
        unexpected.is_empty(),
        "SL-S3-06 direct implementation references escaped the declared migration surfaces:\n{}",
        unexpected.join("\n")
    );
}

#[test]
#[ignore = "SL-S3-06 blocked until central composition wires canonical Browser/Computer routes"]
fn production_bypass_scan_is_clean() {
    let hits = bypass_hits();
    assert!(
        hits.is_empty(),
        "SL-S3-06 is not closed; concrete Browser/Computer bypasses remain:\n{}",
        hits.join("\n")
    );
}
