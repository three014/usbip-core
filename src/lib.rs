use core::fmt;
pub use util::buffer;

#[cfg(target_family = "unix")]
pub mod unix;

pub mod vhci;
pub mod names;

mod util {
    pub mod buffer;
    pub mod singleton;

    pub fn cast_u8_to_i8_slice(a: &[u8]) -> &[i8] {
        unsafe { std::slice::from_raw_parts(a.as_ptr().cast::<i8>(), a.len()) }
    }

    pub fn _cast_i8_to_u8_slice(a: &[i8]) -> &[u8] {
        unsafe { std::slice::from_raw_parts(a.as_ptr().cast::<u8>(), a.len()) }
    }
}

pub mod net {
    use core::fmt;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Copy, Serialize, Deserialize)]
    #[repr(u32)]
    pub enum Status {
        Success = 0x00,
        Failed = 0x01,
        DevBusy = 0x02,
        DevErr = 0x03,
        NoDev = 0x04,
        Unexpected = 0x05,
    }

    impl fmt::Display for Status {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Status::Success => write!(f, "Request succeeded"),
                Status::Failed => write!(f, "Request failed"),
                Status::DevBusy => write!(f, "Device busy (exported)"),
                Status::DevErr => write!(f, "Device in error state"),
                Status::NoDev => write!(f, "Device not found"),
                Status::Unexpected => write!(f, "Unexpected response"),
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum DeviceStatus {
    DevAvailable = 0x01,
    DevInUse,
    DevError,
    PortAvailable,
    PortInitializing,
    PortInUse,
    PortError,
}

impl fmt::Display for DeviceStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeviceStatus::DevAvailable => write!(f, "device is available"),
            DeviceStatus::DevInUse => write!(f, "device is in use"),
            DeviceStatus::DevError => write!(f, "device is unusable because of a fatal error"),
            DeviceStatus::PortAvailable => write!(f, "port is available"),
            DeviceStatus::PortInitializing => write!(f, "port is initializing"),
            DeviceStatus::PortInUse => write!(f, "port is in use"),
            DeviceStatus::PortError => write!(f, "port error"),
        }
    }
}
