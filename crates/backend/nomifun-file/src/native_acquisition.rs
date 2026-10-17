//! Narrow error adapter; native acquisition itself has no backend dependency.
use nomi_native_files::NativeFileError;
use nomifun_common::AppError;

pub fn native_file_acquisition_error(error: NativeFileError) -> AppError {
    match error {
        NativeFileError::Io { operation, source } => crate::path_safety::native_io_error(operation, source),
        NativeFileError::InvalidInput(message) => AppError::BadRequest(message),
        NativeFileError::Changed(message) => AppError::Conflict(message),
        NativeFileError::Unavailable(message) => AppError::UnprocessableEntity(message),
    }
}

pub(crate) fn owner_error_to_native(error: AppError) -> NativeFileError {
    match error {
        AppError::NativeSystemAccessDenied { operation, source } => NativeFileError::Io { operation, source },
        AppError::NotFound(message) => NativeFileError::io("reading native metadata", std::io::Error::new(std::io::ErrorKind::NotFound, message)),
        AppError::BadRequest(message) => NativeFileError::InvalidInput(message),
        AppError::Conflict(message) => NativeFileError::Changed(message),
        AppError::Forbidden(message) => NativeFileError::Changed(message),
        error => NativeFileError::Unavailable(error.to_string()),
    }
}
