use super::*;

#[test]
fn relative_owner_labels_preserve_components_and_round_trip_the_wire() {
    let label = SessionPathLabel::parse("项目/多层目录/AGENTS.override.md").unwrap();
    let native = Path::new("项目").join("多层目录").join("AGENTS.override.md");
    assert_eq!(label.native_path(), native);
    assert_eq!(label.native_path().parent().unwrap(), Path::new("项目").join("多层目录"));
    let json = serde_json::to_string(&label).unwrap();
    assert_eq!(serde_json::from_str::<SessionPathLabel>(&json).unwrap(), label);
    assert_eq!(serde_json::from_str::<String>(&json).unwrap(), "项目/多层目录/AGENTS.override.md");
    assert!(SessionPathLabel::parse("").unwrap().native_path().as_os_str().is_empty());
}

#[test]
fn owner_labels_do_not_accept_native_request_traversal_or_separator_aliases() {
    for value in [".", "./nested", "nested/../other", "nested//file", "nested/", "nested\\file", "../outside", "bad\0path"] {
        assert!(SessionPathLabel::parse(value).is_err(), "{value:?}");
    }
}

#[test]
fn absolute_owner_labels_keep_an_independent_native_root() {
    let root = if cfg!(windows) { Path::new(r"C:\outside") } else { Path::new("/outside") };
    let native = root.join("项目").join("AGENTS.md");
    let label = SessionPathLabel::parse(native.to_str().unwrap()).unwrap();
    assert!(label.native_path().is_absolute());
    assert_eq!(label.native_path(), native);
    assert!(!label.native_path().starts_with(Path::new("项目")));
    #[cfg(windows)]
    for ambiguous in [r"C:relative", r"\rooted", "/rooted", "C:/outside/项目"] {
        assert!(SessionPathLabel::parse(ambiguous).is_err(), "{ambiguous}");
    }
}
