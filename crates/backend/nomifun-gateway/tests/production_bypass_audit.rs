//! Static guard for the SL-S3-06 Browser/Computer bypass migration.
//!
//! This test is the production bypass gate for SL-S3-06. Browser/Computer
//! capability modules and the standalone computer stdio bridge are deleted;
//! the scan remains to prevent concrete implementation dependencies from
//! returning to the Gateway.

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

#[test]
fn production_bypass_scan_is_clean() {
    let hits = bypass_hits();
    assert!(
        hits.is_empty(),
        "SL-S3-06 is open; concrete Browser/Computer bypasses returned:\n{}",
        hits.join("\n")
    );
}
