//! The wire contract is also documented in keyboard/docs/product/usb-protocol.md.
use anyhow::{Result, bail, ensure};
use serde_json::Value;

pub const MAX_FRAME: usize = 4096;
#[derive(Default)]
pub struct Parser {
    buffer: Vec<u8>,
    pub discarded: u64,
}
impl Parser {
    pub fn append(&mut self, bytes: &[u8]) -> Result<Vec<Value>> {
        ensure!(
            bytes.len() <= 8192 && self.buffer.len() + bytes.len() <= 16384,
            "USB receive buffer overflow"
        );
        self.buffer.extend_from_slice(bytes);
        let mut frames = Vec::new();
        while self.buffer.len() >= 4 {
            if self.buffer[0] != 1 || !matches!(self.buffer[1], 1 | 0x10) {
                self.buffer.remove(0);
                self.discarded += 1;
                continue;
            }
            ensure!(
                self.buffer[1] != 1,
                "unexpected USB audio: product requires keyboard NONE and native OS capture"
            );
            let n = 4 + u16::from_le_bytes([self.buffer[2], self.buffer[3]]) as usize;
            if n > MAX_FRAME {
                self.buffer.remove(0);
                self.discarded += 1;
                continue;
            }
            if self.buffer.len() < n {
                break;
            }
            let raw: Vec<_> = self.buffer.drain(..n).collect();
            frames.push(serde_json::from_slice(&raw[4..])?);
        }
        Ok(frames)
    }
    pub fn finish(&self) -> Result<()> {
        ensure!(self.buffer.is_empty(), "truncated USB frame");
        Ok(())
    }
}
pub fn command(value: Value) -> Result<Vec<u8>> {
    let body = serde_json::to_vec(&value)?;
    ensure!(body.len() + 4 <= MAX_FRAME, "command too large");
    let mut out = vec![1, 0x10];
    out.extend((body.len() as u16).to_le_bytes());
    out.extend(body);
    Ok(out)
}

pub fn key(value: &str) -> Result<usize> {
    match value {
        "k1" => Ok(0),
        "k2" => Ok(1),
        "k3" => Ok(2),
        "k4" => Ok(3),
        _ => bail!("unknown button"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fragmented_json_matches_contiguous() {
        let bytes = command(serde_json::json!({"event":"get_device_info"})).unwrap();
        for split in 0..bytes.len() {
            let mut parser = Parser::default();
            let mut frames = parser.append(&bytes[..split]).unwrap();
            frames.extend(parser.append(&bytes[split..]).unwrap());
            assert_eq!(frames.len(), 1);
            parser.finish().unwrap();
        }
    }
    #[test]
    fn resynchronization_and_truncation_are_visible() {
        let mut parser = Parser::default();
        let mut bytes = vec![9, 8, 7];
        bytes.extend(command(serde_json::json!({"event":"ping"})).unwrap());
        assert_eq!(parser.append(&bytes).unwrap().len(), 1);
        assert_eq!(parser.discarded, 3);
        parser.append(&[1, 16, 8, 0, 1]).unwrap();
        assert!(parser.finish().is_err());
    }
    #[test]
    fn usb_audio_is_rejected_at_its_header_without_buffering_a_payload() {
        for split in 0..4 {
            let mut parser = Parser::default();
            let header = [1, 1, 16, 0];
            assert!(parser.append(&header[..split]).unwrap().is_empty());
            assert!(
                parser.append(&header[split..]).unwrap_err().to_string()
                    .contains("unexpected USB audio")
            );
        }
    }
    #[test]
    fn receive_growth_and_unknown_keys_are_rejected() {
        assert!(Parser::default().append(&vec![0; 8193]).is_err());
        assert_eq!(key("k4").unwrap(), 3);
        assert!(key("k5").is_err());
    }
}
