//! HTTP faces: the unauthenticated device face (`/robot/*`) and the
//! owner-authenticated management face (`/api/robots*`).

pub mod device;

pub use device::{RobotDeviceState, build_ota_response, device_router};
