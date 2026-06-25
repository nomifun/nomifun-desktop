//! Shared ANSI/OSC escape stripping + incremental line scanning for consumers
//! that watch a PTY's raw output byte-stream (AutoWork completion marker,
//! IDMM stall detection). Chunks arrive split at arbitrary byte boundaries —
//! possibly mid-escape-sequence — so the state machine is incremental and the
//! line scanner buffers across `feed` calls.

#[derive(Clone, Copy)]
enum EscState {
    Normal,
    /// Saw ESC, awaiting the sequence introducer.
    Esc,
    /// Inside a CSI sequence (`ESC [ … final`).
    Csi,
    /// Inside an OSC sequence (`ESC ] … BEL|ST`).
    Osc,
    /// Saw ESC while inside OSC — the next byte (`\`) completes the ST terminator.
    OscEsc,
}

/// Strip ANSI/OSC escape sequences and C0 controls (except newline) from a
/// buffer, returning lossy UTF-8 text.
pub fn strip_ansi(bytes: &[u8]) -> String {
    let mut state = EscState::Normal;
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    for &b in bytes {
        match state {
            EscState::Normal => match b {
                0x1b => state = EscState::Esc,
                b'\n' => out.push(b'\n'),
                b'\r' => {}
                0x00..=0x08 | 0x0b..=0x1f | 0x7f => {}
                _ => out.push(b),
            },
            EscState::Esc => {
                state = match b {
                    b'[' => EscState::Csi,
                    b']' => EscState::Osc,
                    _ => EscState::Normal,
                };
            }
            EscState::Csi => {
                if (0x40..=0x7e).contains(&b) {
                    state = EscState::Normal;
                }
            }
            EscState::Osc => match b {
                0x07 => state = EscState::Normal,
                0x1b => state = EscState::OscEsc,
                _ => {}
            },
            EscState::OscEsc => state = EscState::Normal,
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Incremental, escape-stripping, line-oriented scanner. Feed raw PTY chunks;
/// get back the completed (newline-terminated) lines, already ANSI-stripped and
/// with the trailing newline removed. The in-progress final line is retained
/// across `feed` calls until its newline arrives.
pub struct AnsiLineScanner {
    state: EscState,
    line: Vec<u8>,
}

impl Default for AnsiLineScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl AnsiLineScanner {
    pub fn new() -> Self {
        Self {
            state: EscState::Normal,
            line: Vec::new(),
        }
    }

    /// Feed a raw output chunk; return any completed lines found within it.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut lines = Vec::new();
        for &b in bytes {
            match self.state {
                EscState::Normal => match b {
                    0x1b => self.state = EscState::Esc,
                    b'\n' => {
                        lines.push(String::from_utf8_lossy(&self.line).into_owned());
                        self.line.clear();
                    }
                    b'\r' => {}
                    0x00..=0x08 | 0x0b..=0x1f | 0x7f => {}
                    _ => self.line.push(b),
                },
                EscState::Esc => {
                    self.state = match b {
                        b'[' => EscState::Csi,
                        b']' => EscState::Osc,
                        _ => EscState::Normal,
                    };
                }
                EscState::Csi => {
                    if (0x40..=0x7e).contains(&b) {
                        self.state = EscState::Normal;
                    }
                }
                EscState::Osc => match b {
                    0x07 => self.state = EscState::Normal,
                    0x1b => self.state = EscState::OscEsc,
                    _ => {}
                },
                EscState::OscEsc => self.state = EscState::Normal,
            }
        }
        lines
    }

    /// The current (not-yet-newline-terminated) partial line, ANSI-stripped.
    pub fn partial(&self) -> String {
        String::from_utf8_lossy(&self.line).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_csi_and_osc_sequences() {
        let raw = b"\x1b[1;32mhello\x1b[0m \x1b]0;title\x07world\r\n";
        assert_eq!(strip_ansi(raw), "hello world\n");
    }

    #[test]
    fn scanner_emits_complete_lines_only() {
        let mut s = AnsiLineScanner::new();
        assert_eq!(s.feed(b"line one\nline t"), vec!["line one".to_string()]);
        assert_eq!(s.partial(), "line t");
        assert_eq!(s.feed(b"wo\n"), vec!["line two".to_string()]);
    }

    #[test]
    fn scanner_strips_ansi_within_lines() {
        let mut s = AnsiLineScanner::new();
        let lines = s.feed(b"\x1b[32mgreen\x1b[0m text\n");
        assert_eq!(lines, vec!["green text".to_string()]);
    }

    #[test]
    fn scanner_handles_escape_split_across_chunks() {
        let mut s = AnsiLineScanner::new();
        assert!(s.feed(b"\x1b").is_empty());
        assert!(s.feed(b"[32m").is_empty());
        assert_eq!(s.feed(b"ok\n"), vec!["ok".to_string()]);
    }

    #[test]
    fn scanner_drops_cr_and_c0_controls() {
        let mut s = AnsiLineScanner::new();
        assert_eq!(s.feed(b"a\x07b\r\n"), vec!["ab".to_string()]);
    }
}
