//! CPAL 0.16 keeps IMMDeviceEnumerator in a process-global OnceLock, while its
//! COM guard is thread-local. Seed that enumerator on a long-lived MTA owner;
//! a short-lived recording or settings task must never own its apartment.
use cpal::traits::HostTrait;
use std::{
    marker::PhantomData,
    sync::{OnceLock, mpsc},
    time::Duration,
};
use windows::Win32::{
    Foundation::RPC_E_CHANGED_MODE,
    System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize},
};

pub(super) struct ThreadCom {
    owns: bool,
    _thread_bound: PhantomData<*mut ()>,
}
impl ThreadCom {
    fn initialize() -> Result<Self, String> {
        // SAFETY: COM initialization and balanced cleanup stay on this thread.
        let status = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if status.is_ok() {
            Ok(Self {
                owns: true,
                _thread_bound: PhantomData,
            })
        } else if status == RPC_E_CHANGED_MODE {
            Ok(Self {
                owns: false,
                _thread_bound: PhantomData,
            })
        } else {
            Err(format!("Windows 音频 COM 初始化失败：{status}"))
        }
    }
}
impl Drop for ThreadCom {
    fn drop(&mut self) {
        if self.owns {
            unsafe { CoUninitialize() };
        }
    }
}
struct Lifetime {
    // Static process lifetime: this is a COM apartment keeper, never a stream.
    // No microphone is opened or played by the keeper.
    _keepalive: mpsc::SyncSender<()>,
    _worker: std::thread::JoinHandle<()>,
}
static LIFETIME: OnceLock<Result<Lifetime, String>> = OnceLock::new();
fn initialize_lifetime() -> Result<Lifetime, String> {
    let (keepalive, receiver) = mpsc::sync_channel(1);
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let worker = std::thread::Builder::new()
        .name("nomifun-audio-com-mta".into())
        .spawn(move || {
            let com = match ThreadCom::initialize() {
                Ok(com) => com,
                Err(error) => {
                    let _ = ready_tx.send(Err(error));
                    return;
                }
            };
            if !com.owns {
                let _ = ready_tx.send(Err("专用 Windows 音频线程未能建立 MTA".into()));
                return;
            }
            let result = cpal::default_host()
                .devices()
                .map(|devices| {
                    let _ = devices.count();
                })
                .map_err(|error| format!("Windows 音频枚举器初始化失败：{error}"));
            let ok = result.is_ok();
            if ready_tx.send(result).is_err() || !ok {
                return;
            }
            let _ = receiver.recv();
            drop(com);
        })
        .map_err(|error| error.to_string())?;
    ready_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| "Windows 音频枚举器初始化超时".to_string())??;
    Ok(Lifetime {
        _keepalive: keepalive,
        _worker: worker,
    })
}
pub(super) fn prepare_lifetime() -> Result<(), String> {
    LIFETIME
        .get_or_init(initialize_lifetime)
        .as_ref()
        .map(|_| ())
        .map_err(Clone::clone)
}
pub(super) fn thread_guard() -> Result<ThreadCom, String> {
    prepare_lifetime()?;
    ThreadCom::initialize()
}
