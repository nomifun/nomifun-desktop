//! Terminal-domain shared helper: the launch-preset resolver.
//!
//! The terminal CAPABILITIES live in `caps_terminal`; this module retains only
//! `preset_launch`, the backend mirror of the frontend launch presets, reused
//! by the terminal capability handlers (and unit-tested there).

/// Backend mirror of the frontend launch presets
/// (`ui/src/renderer/pages/terminal/launchPresets.ts`) — keep the two in sync.
/// Returns `(command, args, backend)`; the `$SHELL` sentinel is resolved to the
/// platform shell by `TerminalService`.
pub(crate) fn preset_launch(preset: &str, full_auto: bool) -> Result<(String, Vec<String>, Option<String>), String> {
    let flag = |f: &str| {
        if full_auto {
            vec![f.to_owned()]
        } else {
            vec![]
        }
    };
    match preset {
        "shell" => Ok((nomifun_terminal::types::SHELL_SENTINEL.to_owned(), vec![], None)),
        "claude" => Ok((
            "claude".to_owned(),
            flag("--dangerously-skip-permissions"),
            Some("claude".to_owned()),
        )),
        "codex" => Ok((
            "codex".to_owned(),
            flag("--dangerously-bypass-approvals-and-sandbox"),
            Some("codex".to_owned()),
        )),
        "gemini" => Ok(("gemini".to_owned(), flag("--yolo"), Some("gemini".to_owned()))),
        other => Err(format!("unknown preset '{other}' (expected shell | claude | codex | gemini)")),
    }
}
