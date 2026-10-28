//! Local test metadata shares the Actor's native owner, without a Document or ASR.
use super::{Identity, audio};
use serde::Serialize;
use std::time::Instant;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MicrophoneTestState {
    pub id: Option<String>,
    pub device_id: Option<String>,
    pub state: String,
    pub callback_observed: bool,
    pub samples: u64,
    pub nonzero: u64,
    pub peak: f64,
    pub rms: f64,
    pub elapsed_ms: u64,
    pub stopped: bool,
    pub detail: Option<String>,
}
impl Default for MicrophoneTestState {
    fn default() -> Self {
        Self { id: None, device_id: None, state: "idle".into(), callback_observed: false,
            samples: 0, nonzero: 0, peak: 0.0, rms: 0.0, elapsed_ms: 0,
            stopped: true, detail: None }
    }
}
pub(super) struct MicrophoneCheck {
    pub test_id: String,
    pub owner: Identity,
    pub meter: audio::MeterCapture,
    pub started_at: Instant,
    pub listening_at: Option<Instant>,
    pub timer: CancellationToken,
    pub outcome: &'static str,
}
impl Drop for MicrophoneCheck {
    fn drop(&mut self) {
        self.timer.cancel();
        self.meter.close();
    }
}
