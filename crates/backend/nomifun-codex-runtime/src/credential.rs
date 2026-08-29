use std::fmt;
use std::pin::Pin;

use async_trait::async_trait;
use nomi_process_runtime::{ChildProcessBuilder, ManagedChildProcess};
use nomifun_agent_contracts::{AgentSessionId, RuntimeBindingId};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncWrite, AsyncWriteExt};

use crate::error::RuntimeError;

pub const CREDENTIAL_PROTOCOL: &str = "nomifun-inherited-handle-v1";
pub const CREDENTIAL_FRAME_MAGIC: &[u8] = b"NOMIFUN-CODEX-CREDENTIAL-V1\0";
pub const MAX_CREDENTIAL_BYTES: usize = 64 * 1024;

type CredentialWriter = Pin<Box<dyn AsyncWrite + Send + Unpin>>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CredentialHandleDescriptor {
    UnixFd { fd: u32 },
    WindowsHandle { value: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeHelloRequest {
    pub credential_protocol: String,
    pub credential_handle: CredentialHandleDescriptor,
}

impl RuntimeHelloRequest {
    pub fn new(credential_handle: CredentialHandleDescriptor) -> Self {
        Self {
            credential_protocol: CREDENTIAL_PROTOCOL.to_owned(),
            credential_handle,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeCredentialScope {
    pub agent_session_id: AgentSessionId,
    pub runtime_binding_id: RuntimeBindingId,
}

#[async_trait]
pub trait RuntimeCredentialProvider: Send + Sync {
    async fn issue(
        &self,
        scope: &RuntimeCredentialScope,
    ) -> Result<InheritedHandleCredential, RuntimeError>;
}

pub struct InheritedHandleCredential {
    bytes: Vec<u8>,
    consumed: bool,
}

impl InheritedHandleCredential {
    pub fn new(bytes: impl Into<Vec<u8>>) -> Result<Self, RuntimeError> {
        let bytes = bytes.into();
        if bytes.is_empty() {
            return Err(RuntimeError::Credential(
                "runtime credential cannot be empty".to_owned(),
            ));
        }
        if bytes.len() > MAX_CREDENTIAL_BYTES {
            return Err(RuntimeError::Credential(format!(
                "runtime credential exceeds {MAX_CREDENTIAL_BYTES} bytes"
            )));
        }
        Ok(Self {
            bytes,
            consumed: false,
        })
    }

    pub fn is_consumed(&self) -> bool {
        self.consumed
    }

    async fn write_once(
        &mut self,
        writer: &mut (dyn AsyncWrite + Send + Unpin),
    ) -> Result<(), RuntimeError> {
        if self.consumed {
            return Err(RuntimeError::Credential(
                "runtime credential is one-shot".to_owned(),
            ));
        }

        let length = u32::try_from(self.bytes.len())
            .map_err(|_| RuntimeError::Credential("credential length overflow".to_owned()))?;
        let result = async {
            writer.write_all(CREDENTIAL_FRAME_MAGIC).await?;
            writer.write_all(&length.to_be_bytes()).await?;
            writer.write_all(&self.bytes).await?;
            writer.flush().await?;
            writer.shutdown().await
        }
        .await;

        self.bytes.fill(0);
        self.bytes.clear();
        self.consumed = true;
        result.map_err(|error| RuntimeError::Credential(error.to_string()))
    }
}

impl fmt::Debug for InheritedHandleCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InheritedHandleCredential")
            .field("bytes", &"<redacted>")
            .field("consumed", &self.consumed)
            .finish()
    }
}

impl Drop for InheritedHandleCredential {
    fn drop(&mut self) {
        self.bytes.fill(0);
    }
}

pub struct PreparedCredentialChannel {
    writer: Option<CredentialWriter>,
    #[cfg(windows)]
    child_read_handle: Option<std::os::windows::io::OwnedHandle>,
}

impl fmt::Debug for PreparedCredentialChannel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedCredentialChannel")
            .field("writer", &self.writer.as_ref().map(|_| "<anonymous-pipe>"))
            .finish_non_exhaustive()
    }
}

impl PreparedCredentialChannel {
    pub fn prepare(builder: &mut ChildProcessBuilder) -> Result<Self, RuntimeError> {
        prepare_platform_channel(builder).map_err(RuntimeError::Process)
    }

    pub fn bind_child(
        &mut self,
        process: &ManagedChildProcess,
    ) -> Result<CredentialHandleDescriptor, RuntimeError> {
        bind_platform_child(self, process).map_err(RuntimeError::Process)
    }

    pub async fn transmit_and_close(
        mut self,
        mut credential: InheritedHandleCredential,
    ) -> Result<(), RuntimeError> {
        let mut writer = self.writer.take().ok_or_else(|| {
            RuntimeError::Credential("credential pipe writer is unavailable".to_owned())
        })?;
        credential.write_once(writer.as_mut().get_mut()).await
    }

    #[cfg(test)]
    pub(crate) fn from_test_writer(writer: impl AsyncWrite + Send + Unpin + 'static) -> Self {
        Self {
            writer: Some(Box::pin(writer)),
            #[cfg(windows)]
            child_read_handle: None,
        }
    }
}

#[cfg(unix)]
fn prepare_platform_channel(
    builder: &mut ChildProcessBuilder,
) -> std::io::Result<PreparedCredentialChannel> {
    use std::os::fd::OwnedFd;
    use std::os::unix::net::UnixStream;

    const CHILD_CREDENTIAL_FD: i32 = 3;

    let (host, child) = UnixStream::pair()?;
    host.set_nonblocking(true)?;
    let child: OwnedFd = child.into();
    builder.inherit_fds(vec![(CHILD_CREDENTIAL_FD, child)]);
    let host = tokio::net::UnixStream::from_std(host)?;
    Ok(PreparedCredentialChannel {
        writer: Some(Box::pin(host)),
    })
}

#[cfg(unix)]
fn bind_platform_child(
    _channel: &mut PreparedCredentialChannel,
    _process: &ManagedChildProcess,
) -> std::io::Result<CredentialHandleDescriptor> {
    Ok(CredentialHandleDescriptor::UnixFd { fd: 3 })
}

#[cfg(windows)]
fn prepare_platform_channel(
    _builder: &mut ChildProcessBuilder,
) -> std::io::Result<PreparedCredentialChannel> {
    use std::fs::File;
    use std::mem::MaybeUninit;
    use std::os::windows::io::{FromRawHandle, OwnedHandle};

    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    use windows_sys::Win32::System::Pipes::CreatePipe;

    let mut read = MaybeUninit::uninit();
    let mut write = MaybeUninit::uninit();
    let mut security = SECURITY_ATTRIBUTES {
        nLength: u32::try_from(std::mem::size_of::<SECURITY_ATTRIBUTES>())
            .expect("SECURITY_ATTRIBUTES fits in u32"),
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 0,
    };
    // SAFETY: both output pointers are valid, security has the documented
    // size, and successful handles are immediately transferred to RAII types.
    if unsafe { CreatePipe(read.as_mut_ptr(), write.as_mut_ptr(), &mut security, 0) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: CreatePipe succeeded and returned two fresh owned handles.
    let read = unsafe { OwnedHandle::from_raw_handle(read.assume_init().cast()) };
    // SAFETY: CreatePipe succeeded and returned one fresh write handle.
    let write = unsafe { File::from_raw_handle(write.assume_init().cast()) };
    Ok(PreparedCredentialChannel {
        writer: Some(Box::pin(tokio::fs::File::from_std(write))),
        child_read_handle: Some(read),
    })
}

#[cfg(windows)]
fn bind_platform_child(
    channel: &mut PreparedCredentialChannel,
    process: &ManagedChildProcess,
) -> std::io::Result<CredentialHandleDescriptor> {
    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::Foundation::{DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    let source = channel.child_read_handle.take().ok_or_else(|| {
        std::io::Error::other("credential child read handle is unavailable")
    })?;
    let target_process = process
        .child()
        .raw_handle()
        .ok_or_else(|| std::io::Error::other("runtime process handle is unavailable"))?;
    let mut target: HANDLE = std::ptr::null_mut();
    // SAFETY: source and target process handles remain live for the call.
    // DuplicateHandle writes one handle value owned by the child process.
    if unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            source.as_raw_handle().cast(),
            target_process.cast(),
            &mut target,
            0,
            0,
            DUPLICATE_SAME_ACCESS,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    drop(source);
    let value = u64::try_from(target as usize)
        .map_err(|_| std::io::Error::other("credential handle value overflow"))?;
    Ok(CredentialHandleDescriptor::WindowsHandle { value })
}

#[cfg(not(any(unix, windows)))]
fn prepare_platform_channel(
    _builder: &mut ChildProcessBuilder,
) -> std::io::Result<PreparedCredentialChannel> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "runtime credential handles require Unix or Windows",
    ))
}

#[cfg(not(any(unix, windows)))]
fn bind_platform_child(
    _channel: &mut PreparedCredentialChannel,
    _process: &ManagedChildProcess,
) -> std::io::Result<CredentialHandleDescriptor> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "runtime credential handles require Unix or Windows",
    ))
}

#[cfg(test)]
mod tests {
    use tokio::io::AsyncReadExt;

    use super::*;

    #[test]
    fn credential_debug_never_contains_secret_material() {
        let credential = InheritedHandleCredential::new(b"do-not-log-me".to_vec()).unwrap();
        let debug = format!("{credential:?}");
        assert!(!debug.contains("do-not-log-me"));
        assert!(debug.contains("<redacted>"));
    }

    #[tokio::test]
    async fn credential_frame_is_bounded_one_shot_and_closes() {
        let (mut reader, writer) = tokio::io::duplex(1024);
        let channel = PreparedCredentialChannel {
            writer: Some(Box::pin(writer)),
            #[cfg(windows)]
            child_read_handle: None,
        };
        let credential = InheritedHandleCredential::new(b"runtime-secret".to_vec()).unwrap();

        channel.transmit_and_close(credential).await.unwrap();

        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await.unwrap();
        assert_eq!(&bytes[..CREDENTIAL_FRAME_MAGIC.len()], CREDENTIAL_FRAME_MAGIC);
        let length_start = CREDENTIAL_FRAME_MAGIC.len();
        let length = u32::from_be_bytes(
            bytes[length_start..length_start + 4]
                .try_into()
                .unwrap(),
        );
        assert_eq!(length, 14);
        assert_eq!(&bytes[length_start + 4..], b"runtime-secret");
    }

    #[test]
    fn descriptor_contains_only_handle_metadata() {
        let hello = RuntimeHelloRequest::new(CredentialHandleDescriptor::WindowsHandle {
            value: 42,
        });
        let json = serde_json::to_string(&hello).unwrap();
        assert_eq!(
            json,
            "{\"credential_protocol\":\"nomifun-inherited-handle-v1\",\"credential_handle\":{\"kind\":\"windows_handle\",\"value\":42}}"
        );
    }
}
