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
    pub fn sudo(password: Zeroizing<String>) -> Self {
        // sudo's default prompt is "[sudo] password for <user>: "; also match
        // the generic "Password:" some PAM stacks emit.
        let prompt = Regex::new(r"(?i)\[sudo\] password for .*:|^password:\s*$|\bpassword:\s*$")
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
