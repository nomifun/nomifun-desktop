//! Reliable app / URL / file launch via the OS shell.
//!
//! On Windows this does two things `cmd /c start` and a naive `ShellExecute`
//! cannot:
//!   1. **Resolve an application by name.** A bare display name like "QQ音乐" or
//!      "notepad" is not a file/URL/registered-app, so `ShellExecute("QQ音乐")`
//!      fails with ERROR_CANCELLED (1223) after popping the "Windows can't find"
//!      chooser. We instead resolve the name through the Start Menu
//!      (`Get-StartApps`, fuzzy-matched), turn the resulting AppID into a real
//!      launch target (a resolved `.exe` path, or `shell:AppsFolder\<AUMID>` for
//!      packaged apps), and ShellExecute THAT — which works.
//!   2. **Open URLs/files** via `ShellExecuteExW` (the `open` crate with the
//!      workspace's `shellexecute-on-windows` feature), the same path a
//!      double-click uses — no `cmd /c start` window-title quirk, no dialog.
//!
//! macOS/Linux fall through to `open` / `xdg-open` (which already resolve apps by
//! name well enough); the Start-Menu resolution is Windows-only.

/// Reject the degenerate targets that make the Windows shell pop a "cannot find"
/// dialog: empty/whitespace, or a string that is nothing but path separators
/// (`\`, `\\`, `//`) — the exact shape behind the `\` dialog.
pub fn validate_launch_target(target: &str) -> Result<(), String> {
    let t = target.trim();
    if t.is_empty() {
        return Err("launch target is empty".to_string());
    }
    if t.chars().all(|c| c == '\\' || c == '/') {
        return Err(format!(
            "launch target {target:?} is just path separators — give a URL (https://…), a file or \
             folder path, or an application name"
        ));
    }
    Ok(())
}

/// True if `target` looks like a URL or registered protocol (`https://…`,
/// `mailto:…`, `microsoft-edge:…`) — those ShellExecute directly and must not be
/// run through Start-Menu app resolution. A drive-letter path (`C:\…`) is NOT a
/// URL (single-letter scheme is rejected).
#[cfg(any(target_os = "windows", test))]
fn is_url(target: &str) -> bool {
    if target.contains("://") {
        return true;
    }
    match target.find(':') {
        Some(i) if i > 1 => target[..i]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.'),
        _ => false,
    }
}

/// Launch `target` (a URL, file/folder path, or application name) reliably via
/// the OS shell, optionally opening it WITH a specific `app`. Detached. Returns
/// a human-readable success message.
pub async fn launch(target: &str, app: Option<&str>) -> Result<String, String> {
    validate_launch_target(target)?;
    let target = target.to_string();
    let app = app.map(|s| s.to_string());
    tokio::task::spawn_blocking(move || launch_blocking(&target, app.as_deref()))
        .await
        .unwrap_or_else(|e| Err(format!("launch task failed: {e}")))
}

/// Blocking launch (runs on a `spawn_blocking` thread): resolve the target if it
/// is a bare Windows app name, then ShellExecute it.
fn launch_blocking(target: &str, app: Option<&str>) -> Result<String, String> {
    // Opening a target WITH a specific app (e.g. a URL in a chosen browser):
    // the app handler resolves it; no Start-Menu lookup.
    if let Some(a) = app {
        return open::with_detached(target, a)
            .map(|()| format!("Opened {target:?} with {a:?}."))
            .map_err(|e| format!("failed to open {target:?} with {a:?}: {e}"));
    }

    // Windows: resolve a bare application NAME through the Start Menu before
    // ShellExecuting it. URLs and existing paths skip resolution (they open
    // directly).
    #[cfg(target_os = "windows")]
    {
        if !is_url(target) && !std::path::Path::new(target).exists() {
            if let Some(resolved) = resolve_start_app(target) {
                return open::that_detached(&resolved)
                    .map(|()| format!("Launched {target:?} (resolved via the Start Menu to {resolved:?})."))
                    .map_err(|e| {
                        format!("found {target:?} in the Start Menu ({resolved:?}) but failed to launch it: {e}")
                    });
            }
            // Not in the Start Menu — best-effort raw open, with a clear error so
            // the model knows to use the exact name / a full path instead of
            // retrying the same string.
            return open::that_detached(target)
                .map(|()| format!("Opened {target:?}."))
                .map_err(|e| {
                    format!(
                        "could not find an application named {target:?} in the Start Menu, and the \
                         OS could not open it directly ({e}). Use the app's exact Start-menu name, \
                         or a full path to its .exe."
                    )
                });
        }
    }

    // URL / existing path (and all non-Windows launches).
    open::that_detached(target)
        .map(|()| format!("Opened {target:?}."))
        .map_err(|e| format!("failed to open {target:?}: {e}"))
}

// ---- Windows Start-Menu application resolution --------------------------

/// Resolve a bare application name to a ShellExecute-able launch target via the
/// Start Menu. `None` when no Start-Menu app matches.
#[cfg(target_os = "windows")]
fn resolve_start_app(name: &str) -> Option<String> {
    let apps = get_start_apps();
    let app_id = match_app(&apps, name)?;
    Some(app_id_to_launch_target(&app_id))
}

/// Diagnostic: resolve an application name to its launch target WITHOUT launching
/// it (Windows only). Used by the `appresolve` example to verify Start-Menu
/// resolution end-to-end on a real machine. `None` if nothing matches.
#[cfg(target_os = "windows")]
pub fn resolve_app_for_diagnostics(name: &str) -> Option<String> {
    resolve_start_app(name)
}

/// Enumerate Start-Menu apps as `(display_name, app_id)` via PowerShell
/// `Get-StartApps`. Uses `-EncodedCommand` (base64 UTF-16LE) to avoid all
/// argument-quoting pitfalls, and `CREATE_NO_WINDOW` so no console flashes.
/// Returns empty on any failure (caller falls back to a raw open).
#[cfg(target_os = "windows")]
fn get_start_apps() -> Vec<(String, String)> {
    use base64::Engine;
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    // Tab-separated Name<TAB>AppID per line (names/AppIDs don't contain tabs).
    let script = "[Console]::OutputEncoding=[Text.Encoding]::UTF8; \
                  Get-StartApps | ForEach-Object { \"$($_.Name)`t$($_.AppID)\" }";
    let encoded = {
        let utf16: Vec<u8> = script.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        base64::engine::general_purpose::STANDARD.encode(utf16)
    };

    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &encoded])
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    let Ok(out) = output else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    parse_start_apps(&text)
}

/// Parse `Get-StartApps` tab-separated output into `(name, app_id)` pairs (pure).
#[cfg(target_os = "windows")]
fn parse_start_apps(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let mut it = line.splitn(2, '\t');
            let name = it.next()?.trim();
            let app_id = it.next()?.trim();
            if name.is_empty() || app_id.is_empty() {
                None
            } else {
                Some((name.to_string(), app_id.to_string()))
            }
        })
        .collect()
}

/// Normalize for matching: lowercase, drop all whitespace.
#[cfg(target_os = "windows")]
fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// The exe basename of an AppID path (without `.exe`), for matching a query like
/// "qqmusic" against `…\QQMusic.exe`. Empty for AUMIDs (no `\`).
#[cfg(target_os = "windows")]
fn exe_basename(app_id: &str) -> String {
    if !app_id.contains('\\') {
        return String::new();
    }
    let base = app_id.rsplit('\\').next().unwrap_or(app_id);
    base.strip_suffix(".exe")
        .or_else(|| base.strip_suffix(".EXE"))
        .unwrap_or(base)
        .to_string()
}

/// Pick the best-matching AppID for `query` from the Start-Menu list (pure).
/// Priority: exact (normalized) match on the display name or the exe basename;
/// then a containment match (shortest name = most specific). `None` if nothing
/// reasonable matches.
#[cfg(target_os = "windows")]
fn match_app(apps: &[(String, String)], query: &str) -> Option<String> {
    let q = norm(query);
    if q.is_empty() {
        return None;
    }
    let mut best_contains: Option<(&str, usize)> = None;
    for (name, app_id) in apps {
        let nn = norm(name);
        let bn = norm(&exe_basename(app_id));
        if nn == q || (!bn.is_empty() && bn == q) {
            return Some(app_id.clone()); // exact wins immediately
        }
        if q.len() >= 2 && (nn.contains(&q) || (!bn.is_empty() && bn.contains(&q))) {
            let score = name.chars().count();
            if best_contains.map(|(_, s)| score < s).unwrap_or(true) {
                best_contains = Some((app_id, score));
            }
        }
    }
    best_contains.map(|(a, _)| a.to_string())
}

/// Split a `{KNOWN-FOLDER-GUID}\rest` AppID into the GUID and the remainder
/// (pure). `None` when there is no leading `{…}` brace group.
#[cfg(target_os = "windows")]
fn split_guid_prefix(app_id: &str) -> Option<(&str, &str)> {
    if !app_id.starts_with('{') {
        return None;
    }
    let end = app_id.find('}')?;
    let guid = &app_id[..=end];
    let rest = app_id[end + 1..].trim_start_matches('\\');
    Some((guid, rest))
}

/// Convert a `Get-StartApps` AppID into a ShellExecute-able launch target.
///   * `{GUID}\rest`  → resolve the known-folder GUID and join `rest` (a real path).
///   * other `…\…`    → a plain absolute path, used as-is.
///   * no backslash   → an AUMID (packaged app) → `shell:AppsFolder\<AUMID>`.
/// An unresolvable GUID falls back to the raw AppID (best effort).
#[cfg(target_os = "windows")]
fn app_id_to_launch_target(app_id: &str) -> String {
    if !app_id.contains('\\') {
        return format!("shell:AppsFolder\\{app_id}");
    }
    if let Some((guid, rest)) = split_guid_prefix(app_id) {
        if let Some(base) = known_folder_path(guid) {
            return std::path::Path::new(&base)
                .join(rest)
                .to_string_lossy()
                .into_owned();
        }
    }
    app_id.to_string()
}

/// Resolve the common known-folder GUIDs that `Get-StartApps` prefixes desktop
/// app paths with, via their environment variables (no COM/FFI needed). Covers
/// ProgramFiles(x86/x64), Windows/System32, LocalAppData/AppData, UserProfile.
#[cfg(target_os = "windows")]
fn known_folder_path(guid: &str) -> Option<String> {
    let g = guid.to_ascii_uppercase();
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let win = || env("SystemRoot").or_else(|| env("windir"));
    match g.as_str() {
        // FOLDERID_ProgramFilesX86
        "{7C5A40EF-A0FB-4BFC-874A-C0F2E0B9FA8E}" => env("ProgramFiles(x86)").or_else(|| env("ProgramFiles")),
        // FOLDERID_ProgramFilesX64 / FOLDERID_ProgramFiles
        "{6D809377-6AF0-444B-8957-A3773F02200E}" => env("ProgramW6432").or_else(|| env("ProgramFiles")),
        "{905E63B6-C1BF-494E-B29C-65B732D3D21A}" => env("ProgramFiles"),
        // FOLDERID_Windows
        "{F38BF404-1D43-42F2-9305-67DE0B28FC23}" => win(),
        // FOLDERID_System (System32) / FOLDERID_SystemX86
        "{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}" | "{D65231B0-B2F1-4857-A4CE-A8E7C6EA7D27}" => {
            win().map(|w| format!("{w}\\System32"))
        }
        // FOLDERID_LocalAppData
        "{F1B32785-6FBA-4FCF-9D55-7B8E7F157091}" => env("LOCALAPPDATA"),
        // FOLDERID_RoamingAppData
        "{3EB685DB-65F9-4CF6-A03A-E3EF65729F3D}" => env("APPDATA"),
        // FOLDERID_Profile
        "{5E6C858F-0E22-4760-9AFE-EA3317B67173}" => env("USERPROFILE"),
        // FOLDERID_Programs (user Start Menu\Programs)
        "{A77F5D77-2E2B-44C3-A6A2-ABA601054A51}" => {
            env("APPDATA").map(|a| format!("{a}\\Microsoft\\Windows\\Start Menu\\Programs"))
        }
        // FOLDERID_CommonPrograms (all-users Start Menu\Programs)
        "{0139D44E-6AFE-49F2-8690-3DAFCAE6FFB8}" => {
            env("ProgramData").map(|a| format!("{a}\\Microsoft\\Windows\\Start Menu\\Programs"))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_whitespace_and_bare_separators() {
        assert!(validate_launch_target("").is_err());
        assert!(validate_launch_target("   ").is_err());
        assert!(validate_launch_target("\\").is_err());
        assert!(validate_launch_target("//").is_err());
        assert!(validate_launch_target("\\\\").is_err());
        assert!(validate_launch_target("/\\/").is_err());
    }

    #[test]
    fn accepts_url_path_and_app_name() {
        assert!(validate_launch_target("https://www.example.com").is_ok());
        assert!(validate_launch_target("C:\\Windows\\notepad.exe").is_ok());
        assert!(validate_launch_target("msedge").is_ok());
        assert!(validate_launch_target("/home/user/file.txt").is_ok());
    }

    #[test]
    fn is_url_classifies_schemes_not_drive_paths() {
        assert!(is_url("https://example.com"));
        assert!(is_url("http://x"));
        assert!(is_url("mailto:a@b.com"));
        assert!(is_url("microsoft-edge:https://x"));
        assert!(!is_url("C:\\Program Files\\x.exe")); // drive letter, not a scheme
        assert!(!is_url("QQ音乐"));
        assert!(!is_url("notepad"));
    }

    // ---- Windows Start-Menu resolution (pure logic) ----
    #[cfg(target_os = "windows")]
    mod windows_resolution {
        use super::super::*;

        /// Real `Get-StartApps` rows from this machine (QQ Music is a
        /// GUID-prefixed desktop path; QQ is an AUMID-like bare id).
        fn sample() -> Vec<(String, String)> {
            vec![
                ("QQ".to_string(), "QQ".to_string()),
                (
                    "QQ音乐".to_string(),
                    "{7C5A40EF-A0FB-4BFC-874A-C0F2E0B9FA8E}\\Tencent\\QQMusic\\QQMusic.exe".to_string(),
                ),
                (
                    "网易云音乐".to_string(),
                    "{6D809377-6AF0-444B-8957-A3773F02200E}\\NetEase\\CloudMusic\\cloudmusic.exe".to_string(),
                ),
                ("Notepad".to_string(), "Microsoft.Windows.Notepad_8wekyb3d8bbwe!App".to_string()),
            ]
        }

        #[test]
        fn parse_start_apps_splits_tab_separated() {
            let out = "QQ音乐\t{7C5A40EF}\\a\\b.exe\nNotepad\tNotepad.AUMID\n\n";
            let v = parse_start_apps(out);
            assert_eq!(v.len(), 2);
            assert_eq!(v[0].0, "QQ音乐");
            assert_eq!(v[0].1, "{7C5A40EF}\\a\\b.exe");
            assert_eq!(v[1], ("Notepad".to_string(), "Notepad.AUMID".to_string()));
        }

        #[test]
        fn match_app_exact_chinese_name() {
            let m = match_app(&sample(), "QQ音乐").unwrap();
            assert!(m.contains("QQMusic.exe"), "got {m}");
        }

        #[test]
        fn match_app_matches_exe_basename_for_qqmusic() {
            // "qqmusic" matches the exe basename of QQ音乐's AppID, not its name.
            let m = match_app(&sample(), "qqmusic").unwrap();
            assert!(m.contains("QQMusic.exe"), "got {m}");
        }

        #[test]
        fn match_app_contains_for_partial_chinese() {
            let m = match_app(&sample(), "网易云").unwrap();
            assert!(m.contains("cloudmusic.exe"), "got {m}");
        }

        #[test]
        fn match_app_aumid_exact() {
            let m = match_app(&sample(), "notepad").unwrap();
            assert!(m.contains("Microsoft.Windows.Notepad"), "got {m}");
        }

        #[test]
        fn match_app_none_for_unknown() {
            assert!(match_app(&sample(), "definitely-not-an-app-xyz").is_none());
        }

        #[test]
        fn split_guid_prefix_extracts_guid_and_rest() {
            let (g, r) = split_guid_prefix("{7C5A40EF-A0FB-4BFC-874A-C0F2E0B9FA8E}\\Tencent\\QQMusic\\QQMusic.exe").unwrap();
            assert_eq!(g, "{7C5A40EF-A0FB-4BFC-874A-C0F2E0B9FA8E}");
            assert_eq!(r, "Tencent\\QQMusic\\QQMusic.exe");
            assert!(split_guid_prefix("C:\\plain\\path.exe").is_none());
            assert!(split_guid_prefix("Some.AUMID!App").is_none());
        }

        #[test]
        fn app_id_to_launch_target_aumid_and_plain_path() {
            assert_eq!(
                app_id_to_launch_target("Microsoft.Windows.Notepad_8wekyb3d8bbwe!App"),
                "shell:AppsFolder\\Microsoft.Windows.Notepad_8wekyb3d8bbwe!App"
            );
            assert_eq!(app_id_to_launch_target("C:\\plain\\x.exe"), "C:\\plain\\x.exe");
        }

        #[test]
        fn app_id_to_launch_target_resolves_program_files_x86_guid() {
            // {7C5A40EF…} = ProgramFilesX86 → real path under %ProgramFiles(x86)%.
            let t = app_id_to_launch_target(
                "{7C5A40EF-A0FB-4BFC-874A-C0F2E0B9FA8E}\\Tencent\\QQMusic\\QQMusic.exe",
            );
            assert!(t.ends_with("\\Tencent\\QQMusic\\QQMusic.exe"), "got {t}");
            assert!(!t.starts_with('{'), "GUID must be resolved away: {t}");
        }
    }
}
