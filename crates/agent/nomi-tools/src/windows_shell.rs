use nomi_process_runtime::Transport;

pub(crate) const SHELL_PTY_COLS: u16 = 120;
pub(crate) const SHELL_PTY_ROWS: u16 = 30;

#[cfg(windows)]
const WINDOW_LAUNCH_ERROR: &str =
    "Windows shell commands cannot open a separate console or application window; use the dedicated launch tool";

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

    #[cfg(not(windows))]
    let _ = script;

    Ok(())
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

#[cfg(windows)]
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
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn launch_policy_recognizes_command_boundaries() {
        assert!(contains_explicit_window_launch("start cmd"));
        assert!(contains_explicit_window_launch("cmd /c \"start notepad\""));
        assert!(!contains_explicit_window_launch("Write-Output 'cmd /k is data'"));
        assert!(!contains_explicit_window_launch("cmd /c echo start"));
    }
}
