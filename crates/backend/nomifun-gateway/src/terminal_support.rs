//! Terminal-domain shared helper: the launch-preset resolver.
//!
//! The terminal CAPABILITIES live in `caps_terminal`; this module retains only
//! `preset_launch`, the backend mirror of the frontend launch presets, reused
//! by the terminal capability handlers (and unit-tested there).

/// Backend mirror of the frontend launch presets
/// (`ui/src/renderer/pages/terminal/launchPresets.ts`) — keep the two in sync.
/// Agent CLI presets always use their fixed FullAuto flags. Returns
/// `(command, args, backend)`; the `$SHELL` sentinel is resolved to the
/// platform shell by `TerminalService`.
pub(crate) fn preset_launch(
    preset: &str,
) -> Result<(String, Vec<String>, Option<String>), String> {
    match preset {
        "shell" => Ok((nomifun_terminal::types::SHELL_SENTINEL.to_owned(), vec![], None)),
        "claude" => Ok((
            "claude".to_owned(),
            vec!["--dangerously-skip-permissions".to_owned()],
            Some("claude".to_owned()),
        )),
        "codex" => Ok((
            "codex".to_owned(),
            vec!["--dangerously-bypass-approvals-and-sandbox".to_owned()],
            Some("codex".to_owned()),
        )),
        "gemini" => Ok((
            "gemini".to_owned(),
            vec!["--yolo".to_owned()],
            Some("gemini".to_owned()),
        )),
        other => Err(format!("unknown preset '{other}' (expected shell | claude | codex | gemini)")),
    }
}
