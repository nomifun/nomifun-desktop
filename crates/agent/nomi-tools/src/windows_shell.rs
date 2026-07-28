use nomi_process_runtime::Transport;

pub(crate) const SHELL_PTY_COLS: u16 = 120;
pub(crate) const SHELL_PTY_ROWS: u16 = 30;

#[cfg(windows)]
const WINDOW_LAUNCH_ERROR: &str =
    "Windows shell commands cannot open a separate console or application window; use the dedicated launch tool";

const AGENT_WEB_OPEN_ERROR: &str =
    "shell commands must not open web URLs in the operating-system browser; use the managed \
     Browser tool (browser navigate) to read or interact with web pages";

pub(crate) fn shell_transport(requested_tty: bool) -> Transport {
    if cfg!(windows) || requested_tty {
        Transport::Pty {
            cols: SHELL_PTY_COLS,
            rows: SHELL_PTY_ROWS,
        }
    } else {
        Transport::Pipe
    }
}

pub(crate) fn validate_shell_script(script: &str) -> Result<(), String> {
    #[cfg(windows)]
    if contains_explicit_window_launch(script) {
        return Err(WINDOW_LAUNCH_ERROR.to_owned());
    }

    if contains_agent_web_open(script) {
        return Err(AGENT_WEB_OPEN_ERROR.to_owned());
    }

    Ok(())
}

/// OS opener and browser executables that would hand an `http/https` URL to a
/// visible operating-system browser. Invoking one of these with a web URL from
/// the Agent shell bypasses the managed Browser Hub's approval, egress and
/// lifecycle policies, so it fails closed on every platform. Local files and
/// plain application launches (no web URL argument) stay allowed.
const WEB_OPENER_PROGRAMS: &[&str] = &[
    "xdg-open",
    "open",
    "gio",
    "kde-open",
    "kde-open5",
    "gnome-open",
    "sensible-browser",
    "x-www-browser",
    "explorer",
    "start",
    "start-process",
    "saps",
    "rundll32",
    "google-chrome",
    "google-chrome-stable",
    "chrome",
    "chromium",
    "chromium-browser",
    "firefox",
    "msedge",
    "microsoft-edge",
    "brave",
    "brave-browser",
    "opera",
    "vivaldi",
    "safari",
];

fn contains_agent_web_open(script: &str) -> bool {
    command_tokens(script)
        .into_iter()
        .any(|command| command_opens_web_url(&command))
}

fn command_opens_web_url(command: &[String]) -> bool {
    let Some(program) = command.first().map(|word| program_basename(word)) else {
        return false;
    };
    if !WEB_OPENER_PROGRAMS.contains(&program.as_str()) {
        return false;
    }
    command.iter().skip(1).any(|argument| {
        // Scheme-prefix match: browsers normalize scheme-only forms such as
        // `https:example.com` back to a real web navigation.
        let lower = argument.to_ascii_lowercase();
        lower.contains("http:") || lower.contains("https:")
    })
}

/// Lowercased executable basename: strips any path prefix and a trailing
/// `.exe`, so `/usr/bin/xdg-open` and `C:\...\msedge.exe` match their entries.
fn program_basename(word: &str) -> String {
    let base = word
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(word)
        .to_ascii_lowercase();
    base.strip_suffix(".exe").unwrap_or(&base).to_owned()
}

#[cfg(windows)]
fn contains_explicit_window_launch(script: &str) -> bool {
    command_tokens(script)
        .into_iter()
        .any(|command| command_requests_window(&command))
}

#[cfg(windows)]
fn command_requests_window(command: &[String]) -> bool {
    let Some(program) = command.first().map(|word| word.to_ascii_lowercase()) else {
        return false;
    };
    if matches!(program.as_str(), "start" | "start-process" | "saps") {
        return true;
    }
    if !matches!(program.as_str(), "cmd" | "cmd.exe") {
        return false;
    }

    let Some((switch_index, switch)) = command
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, word)| matches!(word.to_ascii_lowercase().as_str(), "/c" | "/k"))
    else {
        return false;
    };
    if switch.eq_ignore_ascii_case("/k") {
        return true;
    }
    command
        .get(switch_index + 1)
        .is_some_and(|command| starts_with_command(command, "start"))
}

#[cfg(windows)]
fn starts_with_command(command: &str, program: &str) -> bool {
    let Some(rest) = command.trim_start().strip_prefix(program) else {
        return false;
    };
    rest.is_empty() || rest.chars().next().is_some_and(char::is_whitespace)
}

fn command_tokens(script: &str) -> Vec<Vec<String>> {
    let mut commands = Vec::new();
    let mut command = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut escaped = false;

    let finish_word = |word: &mut String, command: &mut Vec<String>| {
        if !word.is_empty() {
            command.push(std::mem::take(word));
        }
    };
    let finish_command = |word: &mut String, command: &mut Vec<String>, commands: &mut Vec<Vec<String>>| {
        finish_word(word, command);
        if !command.is_empty() {
            commands.push(std::mem::take(command));
        }
    };

    for character in script.chars() {
        if escaped {
            word.push(character);
            escaped = false;
            continue;
        }
        if character == '`' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(active_quote) = quote {
            if character == active_quote {
                quote = None;
            } else {
                word.push(character);
            }
            continue;
        }
        match character {
            '\'' | '"' => quote = Some(character),
            character if character.is_whitespace() => finish_word(&mut word, &mut command),
            ';' | '|' | '&' | '\r' | '\n' => {
                finish_command(&mut word, &mut command, &mut commands)
            }
            _ => word.push(character),
        }
    }
    finish_command(&mut word, &mut command, &mut commands);
    commands
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    #[test]
    fn launch_policy_recognizes_command_boundaries() {
        assert!(contains_explicit_window_launch("start cmd"));
        assert!(contains_explicit_window_launch("cmd /c \"start notepad\""));
        assert!(!contains_explicit_window_launch("Write-Output 'cmd /k is data'"));
        assert!(!contains_explicit_window_launch("cmd /c echo start"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_web_opens_fail_closed_on_every_platform() {
        for script in [
            "xdg-open https://example.com",
            "open https://example.com",
            "open -a Safari 'https://example.com'",
            "/usr/bin/xdg-open http://example.com",
            "gio open https://example.com",
            "google-chrome https://example.com",
            "firefox \"https://example.com\"",
            "ls; xdg-open https://example.com",
            "\"C:\\Program Files\\msedge.exe\" https://example.com",
            // Scheme-only forms are normalized to web navigations by browsers.
            "xdg-open https:example.com",
            "firefox http:example.com",
        ] {
            assert!(
                validate_shell_script(script).is_err(),
                "must fail closed for {script:?}"
            );
        }
    }

    #[test]
    fn shell_local_opens_and_non_opener_web_commands_stay_allowed() {
        for script in [
            // Local files and app launches carry no web URL.
            "xdg-open ./report.html",
            "open -R /tmp/file.txt",
            "open .",
            // Network clients without an OS window are egress, not openers.
            "curl https://example.com",
            "wget https://example.com/file.tar.gz",
            "git clone https://example.com/repo.git",
            // A URL as plain data for a non-opener program.
            "echo https://example.com",
            "firefox --version",
        ] {
            assert!(
                validate_shell_script(script).is_ok(),
                "must stay allowed for {script:?}"
            );
        }
    }

    #[test]
    fn program_basename_strips_paths_and_exe() {
        assert_eq!(program_basename("/usr/bin/xdg-open"), "xdg-open");
        assert_eq!(program_basename("C:\\apps\\MSEdge.EXE"), "msedge");
        assert_eq!(program_basename("open"), "open");
    }
}
