use std::{
    fs,
    path::{Path, PathBuf},
};

fn runtime_src_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn production_source(path: &Path) -> String {
    let source =
        fs::read_to_string(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    source
        .split_once("#[cfg(test)]")
        .map_or(source.as_str(), |(production, _)| production)
        .to_owned()
}

fn runtime_production_source() -> String {
    fs::read_dir(runtime_src_dir())
        .expect("read nomifun-runtime source directory")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|extension| extension == "rs"))
        .map(|entry| production_source(&entry.path()))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn bundled_runtime_contains_no_process_ownership_primitives() {
    let source = runtime_production_source();
    let forbidden = [
        "struct CleanupJob",
        "fn install_pdeathsig",
        "fn install_macos_pdeath_watch",
        "PR_SET_PDEATHSIG",
        "libc::kqueue",
        "libc::fork",
        "Builder::clean_cli(\"taskkill\")",
    ];
    let remaining = forbidden
        .into_iter()
        .filter(|primitive| source.contains(primitive))
        .collect::<Vec<_>>();

    assert!(
        remaining.is_empty(),
        "nomifun-runtime must remain limited to bundled-toolchain and PATH support: {remaining:?}"
    );
}

#[test]
fn path_merge_is_delegated_to_nomi_process_runtime() {
    let source = production_source(&runtime_src_dir().join("shell_env.rs"));
    assert!(
        source.contains("nomi_process_runtime::merge_process_path"),
        "shell_env must delegate ordered PATH merging to nomi_process_runtime::merge_process_path"
    );
    assert!(
        !source.contains("std::env::join_paths"),
        "shell_env must not retain a second PATH merge implementation"
    );
}
