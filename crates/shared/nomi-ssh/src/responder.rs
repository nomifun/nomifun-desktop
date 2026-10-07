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
    /// Match one host-generated literal prompt exactly at the end of the
    /// current command output. The prompt must be unguessable to any later
    /// untrusted command; callers remove this rule before running that command.
    pub fn exact_once(
        prompt: &str,
        answer: Zeroizing<String>,
    ) -> Result<Self, regex::Error> {
        let prompt = Regex::new(&format!(r"{}\s*$", regex::escape(prompt)))?;
        Ok(AnswerRule {
            prompt,
            answer,
            once: true,
        })
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

    const PROMPT: &str = "__NOMIFUN_SUDO_AUTH_0190f5fe__:";

    fn exact_rule() -> AnswerRule {
        AnswerRule::exact_once(PROMPT, Zeroizing::new("s3cret".to_string())).unwrap()
    }

    /// The prompt is matched against the *accumulated* output of the running
    /// command, so every case below is written the way `shell.rs` sees it.
    fn matches(sink: &str) -> bool {
        exact_rule().prompt.is_match(sink)
    }

    #[test]
    fn matches_only_the_exact_host_generated_prompt() {
        assert!(matches(PROMPT));
        assert!(matches(&format!("updating\n{PROMPT}")));
    }

    /// A command that merely *prints* the prompt text is not a command waiting
    /// for a password. Answering it writes the password into a shell that is not
    /// reading stdin, so it becomes the next command line and surfaces in the
    /// next command's captured output — the one place the password must never
    /// reach.
    #[test]
    fn never_matches_a_prompt_the_command_only_printed() {
        for sink in [
            // `cat /var/log/auth.log` — the log records past prompts verbatim
            "Aug  5 09:12:01 host sudo: __NOMIFUN_SUDO_AUTH_0190f5fe__:\nAug  5 09:12:02 host sudo: rika : TTY=pts/3\n",
            // `grep -r sudo /etc`
            "/etc/sudoers.d/note:# __NOMIFUN_SUDO_AUTH_0190f5fe__:\n/etc/pam.d/sudo:@include common-auth\n",
            // the prompt scrolled past, then the command kept working
            "__NOMIFUN_SUDO_AUTH_0190f5fe__: \nReading package lists...\n",
        ] {
            assert!(
                !matches(sink),
                "a printed prompt is not a waiting prompt: {sink:?}"
            );
        }
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
            "[sudo] password for rika: ",
            "Password: ",
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
