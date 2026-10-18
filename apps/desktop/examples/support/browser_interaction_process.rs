//! Read-only witness of the exact native browser process. The owned OS handle
//! prevents PID reuse from being mistaken for exit and retains no COM lifetime.
use std::{
    ffi::c_void,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    time::Duration,
};

#[link(name = "kernel32")]
unsafe extern "system" {
    fn OpenProcess(access: u32, inherit: i32, process_id: u32) -> *mut c_void;
    fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
}

pub(super) struct Witness(OwnedHandle);

impl Witness {
    pub async fn observe(view: &tauri::Webview) -> Result<Self, String> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        view.with_webview(move |platform| {
            let result = (|| {
                let core = unsafe { platform.controller().CoreWebView2() }
                    .map_err(|error| error.to_string())?;
                let mut process_id = 0;
                unsafe { core.BrowserProcessId(&mut process_id) }
                    .map_err(|error| error.to_string())?;
                if process_id == 0 {
                    return Err("Native browser process identity missing".into());
                }
                // SYNCHRONIZE is observation only: no termination or write right.
                let raw = unsafe { OpenProcess(0x0010_0000, 0, process_id) };
                if raw.is_null() {
                    return Err(format!("Native browser process observation: {}", std::io::Error::last_os_error()));
                }
                let witness = Self(unsafe { OwnedHandle::from_raw_handle(raw) });
                if witness.exited()? {
                    return Err("Native browser process exited before the close witness".into());
                }
                Ok(witness)
            })();
            let _ = sender.send(result);
        }).map_err(|error| error.to_string())?;
        receiver.await.map_err(|_| "Native browser process witness callback lost".to_owned())?
    }

    fn exited(&self) -> Result<bool, String> {
        match unsafe { WaitForSingleObject(self.0.as_raw_handle(), 0) } {
            0 => Ok(true),
            0x102 => Ok(false), // WAIT_TIMEOUT: this exact process is still alive.
            _ => Err(format!("Native browser process observation failed: {}", std::io::Error::last_os_error())),
        }
    }

    pub async fn wait_for_exit(&self, duration: Duration) -> Result<bool, String> {
        let deadline = tokio::time::Instant::now() + duration;
        loop {
            if self.exited()? { return Ok(true); }
            if tokio::time::Instant::now() >= deadline { return Ok(false); }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
}
