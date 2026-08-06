//! `AnswerRule`: a prompt-driven auto-answer table for the remote shell. The
//! canonical use is sudo — the backend injects the host's sudo password when it
//! sees the sudo prompt, so the model never sees or types the password (it never
//! enters captured output, the transcript, or the provider request). The same
//! table handles other blocking prompts (apt y/n, git credential prompts).
//!
//! Injection is one-shot per command by default: after answering once we stop,
//! so a rejected sudo password is not retried (three failures trigger PAM
//! lockout). The answer bytes are written to the channel only — never appended
//! to captured output.
use regex::Regex;
use zeroize::Zeroizing;

/// One auto-answer rule: when `prompt` matches freshly-arrived output, write
/// `answer` (followed by a newline) to the shell's input.
pub struct AnswerRule {
    pub prompt: Regex,
    pub answer: Zeroizing<String>,
    /// Answer at most once per command (the default for passwords).
    pub once: bool,
}

impl AnswerRule {
    /// Build a sudo password rule matching OpenSSH/sudo's default prompt.
    ///
    /// Two forms, and no more: sudo's own `[sudo] password for <user>: `
    /// (classic sudo, whose `passprompt` replaces PAM's prompt), and a bare
    /// `Password: ` that is the *entire* output so far (sudo-rs — the default
    /// `sudo` on current Ubuntu — has no prompt of its own and lets libpam ask).
    ///
    /// Deliberately **not** matched: `password:` at the end of a longer buffer.
    /// The prompt is tested against everything the command has printed so far,
    /// so a trailing-`password:` pattern also matches `mysql -p`, a nested
    /// `ssh`, `git push` over https and every other program that asks for a
    /// secret — and answering those writes *this host's sudo password* into
    /// their stdin. The cost of the narrow form is that a PAM-prompt sudo is not
    /// auto-answered when the submission printed something before sudo ran; a
    /// leaked sudo password is not recoverable, a re-run is.
    pub fn sudo(password: Zeroizing<String>) -> Self {
        let prompt = Regex::new(r"(?i)\[sudo\] password for .*:|^password:\s*$")
            .expect("static sudo prompt regex");
        AnswerRule {
            prompt,
            answer: password,
            once: true,
        }
    }
}

impl std::fmt::Debug for AnswerRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnswerRule")
            .field("prompt", &self.prompt.as_str())
            .field("answer", &"<redacted>")
            .field("once", &self.once)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::AnswerRule;
    use zeroize::Zeroizing;

    fn sudo_rule() -> AnswerRule {
        AnswerRule::sudo(Zeroizing::new("s3cret".to_string()))
    }

    /// The prompt is matched against the *accumulated* output of the running
    /// command, so every case below is written the way `shell.rs` sees it.
    fn matches(sink: &str) -> bool {
        sudo_rule().prompt.is_match(sink)
    }

    #[test]
    fn matches_sudos_own_prompt_anywhere_in_the_output() {
        assert!(matches("[sudo] password for rika: "));
        assert!(matches("updating\n[sudo] password for rika: "));
    }

    /// sudo implementations with no prompt of their own (sudo-rs, the default
    /// `sudo` on current Ubuntu) let PAM ask, and libpam's built-in prompt is a
    /// bare `Password: `. That is the *whole* of the command's output when sudo
    /// is the command being run.
    #[test]
    fn matches_the_bare_pam_prompt_when_it_is_the_whole_output() {
        assert!(matches("Password: "));
        assert!(matches("password:"));
    }

    /// The regression this test exists for: these prompts all end in
    /// `password:`, and answering them writes *this host's sudo password* into
    /// some other program's stdin — a database server, another SSH host, a git
    /// remote. None of them may match.
    #[test]
    fn never_matches_another_programs_password_prompt() {
        for sink in [
            // mysql -u root -p
            "Enter password: ",
            // a nested ssh to a third host
            "rika@other-host's password: ",
            // git push over https
            "Password for 'https://github.com': ",
            // psql
            "Password for user postgres: ",
            // a prompt that arrives after the command printed something
            "connecting...\nEnter password: ",
        ] {
            assert!(
                !matches(sink),
                "the sudo password must never be offered to {sink:?}"
            );
        }
    }
}
