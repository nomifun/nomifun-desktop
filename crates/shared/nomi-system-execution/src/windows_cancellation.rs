//! One original RPC's native cancellation capability. No execution registry or
//! remembered cancellations: the manual-reset Event is retained through ACK.

use super::*;
use windows_sys::Win32::{Foundation::{ERROR_ALREADY_EXISTS, GetLastError},
    System::Threading::{OpenEventW, SYNCHRONIZATION_SYNCHRONIZE}};

const PREFIX: &str = "Local\\NomiFun.SystemExecution.Cancel.";

/// The authenticated helper owns only a SYNCHRONIZE handle. It cannot signal,
/// reset or reopen a successor request through this capability.
#[derive(Clone, Debug, Default)]
pub struct WindowsCancellation(Option<Arc<Handle>>);

impl WindowsCancellation {
    pub fn require_open(&self) -> Result<(), SystemExecutionError> {
        match self.status() {
            Ok(true) => Err(cancelled_before_dispatch()), Ok(false) => Ok(()),
            Err(error) => Err(before_io("SYSTEM_EXECUTION_NOT_ADMITTED", "Reading the original cancellation Event", error)),
        }
    }
    pub fn is_cancelled(&self) -> bool {
        self.status().unwrap_or(true)
    }
    fn status(&self) -> io::Result<bool> {
        let Some(event) = self.0.as_ref() else { return Ok(false); };
        match unsafe { WaitForSingleObject(event.0, 0) } {
            WAIT_OBJECT_0 => Ok(true), WAIT_TIMEOUT => Ok(false), _ => Err(io::Error::last_os_error()),
        }
    }

    pub async fn cancelled(&self) {
        if self.0.is_none() { std::future::pending::<()>().await; }
        while !self.is_cancelled() { tokio::time::sleep(Duration::from_millis(20)).await; }
    }

    pub(super) fn open(name: &str, connection: Uuid, request_id: Uuid) -> io::Result<Self> {
        if name != format!("{PREFIX}{connection}.{request_id}") {
            return Err(io::Error::other("Native cancellation does not match the original request identity"));
        }
        let event = Handle::new(unsafe { OpenEventW(SYNCHRONIZATION_SYNCHRONIZE, 0, wide(name).as_ptr()) })?;
        Ok(Self(Some(Arc::new(event))))
    }
}

pub(crate) struct CancellationSource {
    pub(super) connection: Uuid,
    pub(super) id: Uuid,
    pub(super) name: String,
    event: Arc<Handle>,
}

impl CancellationSource {
    pub(super) fn new(connection: Uuid) -> Result<Self, SystemExecutionError> {
        Self::create(connection, Uuid::now_v7())
    }

    fn create(connection: Uuid, id: Uuid) -> Result<Self, SystemExecutionError> {
        let name = format!("{PREFIX}{connection}.{id}");
        let user = sid(unsafe { GetCurrentProcess() })
            .map_err(|error| before_io("SYSTEM_EXECUTION_NOT_ADMITTED", "Reading cancellation owner", error))?;
        let sddl = wide(format!("D:P(A;;GA;;;{user})(A;;0x00100000;;;BA)(A;;0x00100000;;;SY)"));
        let mut descriptor = ptr::null_mut();
        if unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), 1, &mut descriptor, ptr::null_mut()) } == 0 {
            return Err(before_io("SYSTEM_EXECUTION_NOT_ADMITTED", "Creating cancellation security", io::Error::last_os_error()));
        }
        let security = PipeSecurity(descriptor);
        let attributes = SECURITY_ATTRIBUTES { nLength: mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: security.0, bInheritHandle: 0 };
        let raw = unsafe { CreateEventW(&attributes, 1, 0, wide(&name).as_ptr()) };
        let error = unsafe { GetLastError() };
        let event = Handle::new(raw).map_err(|error| before_io("SYSTEM_EXECUTION_NOT_ADMITTED", "Creating original cancellation Event", error))?;
        if error == ERROR_ALREADY_EXISTS {
            return Err(before("SYSTEM_EXECUTION_NOT_ADMITTED", "The original cancellation Event already exists"));
        }
        Ok(Self { connection, id, name, event: Arc::new(event) })
    }

    pub(crate) fn signal(&self) -> Result<(), SystemExecutionError> {
        // This exact owned handle stays live in the original RPC worker. No
        // name/PID lookup or successor target is used when Stop arrives.
        if unsafe { SetEvent(self.event.0) } == 0 {
            let mut error = unknown_io("Sending the original native cancellation signal", io::Error::last_os_error());
            error.code = "SYSTEM_EXECUTION_CONTROL_DELIVERY_FAILED".into();
            Err(error)
        } else { Ok(()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_event_is_latched_before_registration_readonly_and_cannot_target_a_successor() {
        let connection = Uuid::now_v7();
        let source = CancellationSource::new(connection).unwrap();
        source.signal().unwrap();
        let native = WindowsCancellation::open(&source.name, connection, source.id).unwrap();
        assert!(native.is_cancelled());
        assert_eq!(unsafe { SetEvent(native.0.as_ref().unwrap().0) }, 0, "native owns no EVENT_MODIFY_STATE right");
        assert_eq!(unsafe { GetLastError() }, 5);
        assert!(WindowsCancellation::open(&source.name, Uuid::now_v7(), source.id).is_err());
        assert!(WindowsCancellation::open(&source.name, connection, Uuid::now_v7()).is_err());
        let successor = CancellationSource::new(connection).unwrap();
        let fresh = WindowsCancellation::open(&successor.name, connection, successor.id).unwrap();
        source.signal().unwrap();
        drop(source);
        assert!(native.is_cancelled());
        assert!(!fresh.is_cancelled());
        successor.signal().unwrap();
        assert!(fresh.is_cancelled());
    }

    #[test]
    fn existing_event_collision_is_rejected_without_reusing_or_resetting_it() {
        let original = CancellationSource::new(Uuid::now_v7()).unwrap();
        original.signal().unwrap();
        let error = match CancellationSource::create(original.connection, original.id) {
            Ok(_) => panic!("reused an existing native Event"), Err(error) => error,
        };
        assert_eq!(error.code, "SYSTEM_EXECUTION_NOT_ADMITTED");
        assert!(WindowsCancellation::open(&original.name, original.connection, original.id).unwrap().is_cancelled());
    }

    #[test]
    fn actual_signal_failure_is_retained_without_reclassifying_the_original_native_result() {
        let owner = CancellationSource::new(Uuid::now_v7()).unwrap();
        let native = WindowsCancellation::open(&owner.name, owner.connection, owner.id).unwrap();
        let readonly_source = CancellationSource { connection:owner.connection, id:owner.id,
            name:owner.name.clone(), event:native.0.unwrap() };
        let delivery = readonly_source.signal().unwrap_err();
        assert_eq!(delivery.code, "SYSTEM_EXECUTION_CONTROL_DELIVERY_FAILED");
        assert_eq!(delivery.details["raw_os_error"], 5);
        let actual = super::super::retain_control_delivery_error(Ok(json!({"original_native_result":19})), Some(delivery.clone())).unwrap();
        assert_eq!(actual["original_native_result"], 19);
        assert!(actual["control_delivery_error"].as_str().unwrap().contains("CONTROL_DELIVERY_FAILED"));
        let original = unknown("actual reply lost after native dispatch");
        let actual = super::super::retain_control_delivery_error(Err(original), Some(delivery.clone())).unwrap_err();
        assert_eq!(actual.code, "SYSTEM_EXECUTION_RESULT_UNKNOWN");
        assert_eq!(actual.details["cleanup_proven"], false);
        assert_eq!(actual.details["user_code_not_started"], false);
        assert!(actual.details["control_delivery_error"].is_string());
        for original in [json!(7), json!(["original native value"]), json!(null), json!("complete")] {
            assert_eq!(super::super::retain_control_delivery_error(Ok(original.clone()), Some(delivery.clone())).unwrap(), original);
        }
        let original = SystemExecutionError { code:"original_result_unknown".into(), message:"original native error".into(), details:json!(["original detail"]) };
        let actual = super::super::retain_control_delivery_error(Err(original.clone()), Some(delivery)).unwrap_err();
        assert_eq!(actual.code,original.code); assert_eq!(actual.details,original.details);
        assert!(actual.message.contains("cancellation signal delivery failed"));
    }
}

impl Drop for CancellationSource {
    fn drop(&mut self) { let _ = self.signal(); }
}

pub(crate) fn cancelled_before_dispatch() -> SystemExecutionError {
    before("SYSTEM_EXECUTION_CANCELLED_BEFORE_DISPATCH", "The original native request was cancelled before device dispatch")
}
