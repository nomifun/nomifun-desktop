//! Narrow error adapter; native acquisition itself has no backend dependency.
use nomi_native_files::{NativeFileError, NativeFileFailure};
use nomifun_common::AppError;

pub fn native_file_acquisition_error(error: NativeFileError) -> AppError {
    let failure = error.observation();
    let (operation, source) = match error {
        NativeFileError::Io { operation, source } => (Some(operation), Some(source)),
        _ => (None, None),
    };
    native_file_failure_error_with_source(failure, operation, source)
}

/// A native worker's typed failure uses the same projection as a login-user
/// acquisition. An error receipt never supplies a Source absence observation.
pub fn native_file_failure_error(failure: NativeFileFailure) -> AppError {
    native_file_failure_error_with_source(failure, None, None)
}

fn native_file_failure_error_with_source(failure: NativeFileFailure, operation: Option<String>, source: Option<std::io::Error>) -> AppError {
    let code = match failure.code.as_str() {
        "INVALID_PAYLOAD" => "INVALID_PAYLOAD",
        "FILE_CHANGED_DURING_READ" => "FILE_CHANGED_DURING_READ",
        "NATIVE_FILE_ACQUIRE_UNAVAILABLE" => "NATIVE_FILE_ACQUIRE_UNAVAILABLE",
        "SYSTEM_ACCESS_DENIED" => "SYSTEM_ACCESS_DENIED",
        "RESOURCE_NOT_FOUND" => "RESOURCE_NOT_FOUND",
        "NATIVE_FILE_READ_FAILED" => "NATIVE_FILE_READ_FAILED",
        _ => return AppError::Internal("Native readonly owner returned an unsupported failure code".into()),
    };
    AppError::NativeFileAcquisition { code, message: failure.message, kind: failure.kind,
        raw_os_error: failure.raw_os_error, operation, source }
}

pub(crate) fn native_read_io_error(operation: impl Into<String>, source: std::io::Error) -> AppError {
    native_file_acquisition_error(NativeFileError::io(operation, source))
}

pub(crate) fn owner_error_to_native(error: AppError) -> NativeFileError {
    match error {
        AppError::NativeFileAcquisition { operation: Some(operation), source: Some(source), .. } => NativeFileError::Io { operation, source },
        AppError::NativeFileAcquisition { code: "INVALID_PAYLOAD", message, .. } => NativeFileError::InvalidInput(message),
        AppError::NativeFileAcquisition { code: "FILE_CHANGED_DURING_READ", message, .. } => NativeFileError::Changed(message),
        AppError::NativeFileAcquisition { message, .. } => NativeFileError::Unavailable(message),
        AppError::NativeSystemAccessDenied { operation, source } => NativeFileError::Io { operation, source },
        AppError::NotFound(message) => NativeFileError::io("reading native metadata", std::io::Error::new(std::io::ErrorKind::NotFound, message)),
        AppError::BadRequest(message) => NativeFileError::InvalidInput(message),
        AppError::Conflict(message) => NativeFileError::Changed(message),
        AppError::Forbidden(message) => NativeFileError::Changed(message),
        error => NativeFileError::Unavailable(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_native_io_round_trip_preserves_the_original_kind_raw_code_and_operation() {
        for raw in [2,5,67] {
            let original=NativeFileError::io("actual native read root",std::io::Error::from_raw_os_error(raw));let expected=original.observation();
            let mapped=native_file_acquisition_error(original);
            assert_eq!(mapped.error_details().unwrap()["native_file_error"],serde_json::to_value(&expected).unwrap());
            let roundtrip=owner_error_to_native(mapped).observation();
            assert_eq!(serde_json::to_value(roundtrip).unwrap(),serde_json::to_value(expected).unwrap());
        }
    }
    #[test]
    fn worker_failure_uses_the_same_error_payload_without_reconstructing_an_io_source() {
        let failure=NativeFileFailure{code:"RESOURCE_NOT_FOUND".into(),message:"root was unavailable".into(),kind:Some("NotFound".into()),raw_os_error:Some(67)};
        let mapped=native_file_failure_error(failure.clone());assert_eq!(mapped.error_code(),"RESOURCE_NOT_FOUND");
        assert_eq!(mapped.error_details().unwrap()["native_file_error"],serde_json::to_value(failure).unwrap());
        assert!(matches!(mapped,AppError::NativeFileAcquisition{source:None,..}));
    }
}
