use core::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use util::buffer::Buffer;

#[cfg(unix)]
mod unix;

pub mod names;

mod util {
    use std::str::FromStr;

    pub mod __padding;
    pub mod buffer;
    pub mod singleton;
    pub mod __private {
        pub trait Sealed {}
    }

    pub fn cast_u8_to_i8_slice(a: &[u8]) -> &[i8] {
        unsafe { std::slice::from_raw_parts(a.as_ptr().cast::<i8>(), a.len()) }
    }

    pub fn _cast_i8_to_u8_slice(a: &[i8]) -> &[u8] {
        unsafe { std::slice::from_raw_parts(a.as_ptr().cast::<u8>(), a.len()) }
    }

    pub fn get_token<'a, 'b: 'a, T>(tokens: &'a mut impl Iterator<Item = &'b str>) -> T
    where
        T: FromStr,
        T::Err: std::error::Error,
    {
        tokens.next().unwrap().trim().parse().unwrap()
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

pub const USBIP_VERSION: usize = 0x111;
pub const DEV_PATH_MAX: usize = 256;
pub const BUS_ID_SIZE: usize = 32;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbDevice {
    path: Buffer<DEV_PATH_MAX, i8>,
    busid: Buffer<BUS_ID_SIZE, i8>,
    busnum: u32,
    devnum: u32,
    speed: u32,
    id_vendor: u16,
    id_product: u16,
    bcd_device: u16,
    b_device_class: u8,
    b_device_subclass: u8,
    b_device_protocol: u8,
    b_configuration_value: u8,
    b_num_configurations: u8,
    b_num_interfaces: u8,
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

#[derive(Debug, Clone, Copy)]
pub enum ParseDeviceStatusError {
    Empty,
    Invalid,
}

impl fmt::Display for ParseDeviceStatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!()
    }
}

impl std::error::Error for ParseDeviceStatusError {}

impl FromStr for DeviceStatus {
    type Err = ParseDeviceStatusError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        todo!()
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbInterface {
    b_interface_class: u8,
    b_interface_subclass: u8,
    b_interface_protocol: u8,
    padding: util::__padding::Padding<u8>,
}

pub mod vhci {
    pub(crate) mod error;
    mod platform {
        #[cfg(unix)]
        pub use crate::unix::vhci2::Driver;
    }

    use crate::DeviceStatus;
    use crate::UsbDevice;
    use core::fmt;
    use std::net::SocketAddr;
    use std::str::FromStr;

    pub use error::Error;
    pub use platform::Driver;

    pub type Result<T> = std::result::Result<T, Error>;

    #[derive(Debug, Clone, Copy)]
    pub enum HubSpeed {
        High = 0,
        Super,
    }

    impl FromStr for HubSpeed {
        type Err = ParseHubSpeedError;

        fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
            match s {
                "ss" => Ok(Self::Super),
                "hs" => Ok(Self::High),
                "" => Err(ParseHubSpeedError::Empty),
                _ => Err(ParseHubSpeedError::Invalid),
            }
        }
    }

    #[derive(Debug, Clone, Copy)]
    pub enum ParseHubSpeedError {
        Empty,
        Invalid,
    }

    impl fmt::Display for ParseHubSpeedError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            todo!()
        }
    }

    impl std::error::Error for ParseHubSpeedError {}

    #[derive(Debug)]
    pub struct ImportedDevice {
        hub: HubSpeed,
        port: u16,
        status: DeviceStatus,
        vendor: u16,
        product: u16,
        dev_id: u32,
        udev: UsbDevice,
    }

    impl ImportedDevice {
        pub const fn hub(&self) -> HubSpeed {
            self.hub
        }

        pub const fn port(&self) -> u16 {
            self.port
        }

        pub const fn status(&self) -> DeviceStatus {
            self.status
        }

        pub const fn vendor(&self) -> u16 {
            self.vendor
        }

        pub const fn dev_id(&self) -> u32 {
            self.dev_id
        }

        pub const fn product(&self) -> u16 {
            self.product
        }

        pub const fn usb_dev(&self) -> &UsbDevice {
            &self.udev
        }
    }

    pub trait VhciDriver: Sized + crate::util::__private::Sealed {
        fn open() -> Result<Self>;
        fn attach(&self, socket: SocketAddr, bus_id: &str) -> Result<u16>;
        fn detach(&self, port: u16) -> Result<()>;
        fn imported_devices(&self) -> Result<&[ImportedDevice]>;
    }
}
