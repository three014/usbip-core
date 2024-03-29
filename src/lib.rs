use core::fmt;
use std::{
    ffi::{c_char, OsStr},
    num::ParseIntError,
    path::Path,
    str::FromStr,
};

use containers::buffer::Buffer;
use serde::{Deserialize, Serialize};

#[cfg(unix)]
mod unix;

pub mod names;
pub mod containers {
    pub mod beef;
    pub mod buffer;
    pub mod singleton;
}

mod util {
    pub mod __padding;
    pub mod __private {
        pub trait Sealed {}
    }
    use std::str::FromStr;

    fn cast_u8_to_i8_slice(a: &[u8]) -> &[i8] {
        unsafe { std::slice::from_raw_parts(a.as_ptr().cast::<i8>(), a.len()) }
    }

    fn cast_i8_to_u8_slice(a: &[i8]) -> &[u8] {
        unsafe { std::slice::from_raw_parts(a.as_ptr().cast::<u8>(), a.len()) }
    }

    pub fn parse_token<'a, 'b: 'a, T>(tokens: &'a mut impl Iterator<Item = &'b str>) -> T
    where
        T: FromStr,
        T::Err: std::error::Error,
    {
        tokens
            .next()
            .expect("There should be another item in the string stream")
            .trim()
            .parse()
            .expect("Token should be valid")
    }

    pub fn cast_i8_to_u8_mut_slice(a: &mut [i8]) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(a.as_mut_ptr().cast::<u8>(), a.len()) }
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
    path: Buffer<DEV_PATH_MAX, c_char>,
    busid: Buffer<BUS_ID_SIZE, c_char>,
    busnum: u32,
    devnum: u32,
    speed: DeviceSpeed,
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

impl UsbDevice {
    pub fn path(&self) -> &Path {
        let s = self.path.to_str().unwrap().trim();
        Path::new(OsStr::new(s))
    }

    pub fn bus_id(&self) -> &str {
        self.busid.to_str().unwrap().trim()
    }

    pub const fn dev_id(&self) -> u32 {
        (self.bus_num() << 16) | self.dev_num()
    }

    pub const fn speed(&self) -> DeviceSpeed {
        self.speed
    }

    pub const fn bus_num(&self) -> u32 {
        self.busnum
    }

    pub const fn dev_num(&self) -> u32 {
        self.devnum
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
            DeviceStatus::DevAvailable => write!(f, "Device Available"),
            DeviceStatus::DevInUse => write!(f, "Device in Use"),
            DeviceStatus::DevError => write!(f, "Device Unusable Due To Fatal Error"),
            DeviceStatus::PortAvailable => write!(f, "Port Available"),
            DeviceStatus::PortInitializing => write!(f, "Port Initializing"),
            DeviceStatus::PortInUse => write!(f, "Port in Use"),
            DeviceStatus::PortError => write!(f, "Port Error"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseDeviceStatusError {
    Invalid,
    Parse(ParseIntError),
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
        let status = match s.parse::<usize>().map_err(Self::Err::Parse)? {
            1 => Self::DevAvailable,
            2 => Self::DevInUse,
            3 => Self::DevError,
            4 => Self::PortAvailable,
            5 => Self::PortInitializing,
            6 => Self::PortInUse,
            7 => Self::PortError,
            _ => return Err(ParseDeviceStatusError::Invalid),
        };
        Ok(status)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u32)]
pub enum DeviceSpeed {
    Unknown = 0,
    Low,
    Full,
    High,
    Wireless,
    Super,
    SuperPlus,
}

impl fmt::Display for DeviceSpeed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeviceSpeed::Unknown => write!(f, "Unknown Speed"),
            DeviceSpeed::Low => write!(f, "Low Speed (1.5 Mbit/s)"),
            DeviceSpeed::Full => write!(f, "Full Speed (12 Mbit/s)"),
            DeviceSpeed::High => write!(f, "High Speed (480 Mbit/s)"),
            DeviceSpeed::Wireless => write!(f, "Wireless Speed (??)"),
            DeviceSpeed::Super => write!(f, "Super Speed (5 Gbit/s)"),
            DeviceSpeed::SuperPlus => write!(f, "Super Speed Plus (10 Gbit/s)"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum TryFromDeviceSpeedError {
    Invalid,
}

impl fmt::Display for TryFromDeviceSpeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TryFromDeviceSpeedError::Invalid => write!(f, "Invalid Device Speed"),
        }
    }
}

impl std::error::Error for TryFromDeviceSpeedError {}

impl FromStr for DeviceSpeed {
    type Err = ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "unknown" => Ok(Self::Unknown),
            "1.5" => Ok(Self::Low),
            "53.3-480" => Ok(Self::Wireless),
            num => Ok(Self::from(num.parse::<u32>()?)),
        }
    }
}

impl From<u32> for DeviceSpeed {
    fn from(value: u32) -> Self {
        match value {
            12 => Self::Full,
            480 => Self::High,
            5000 => Self::Super,
            10000 => Self::SuperPlus,
            _ => Self::Unknown,
        }
    }
}

pub mod vhci {
    pub(crate) mod error;
    mod platform {
        #[cfg(unix)]
        pub use crate::unix::vhci2::UnixDriver as Driver;
        #[cfg(unix)]
        pub use crate::unix::vhci2::UnixImportedDevice as ImportedDevice;
        #[cfg(unix)]
        pub use crate::unix::vhci2::UsbId;
    }

    use crate::DeviceStatus;
    use crate::UsbDevice;
    use core::fmt;
    use std::net::TcpStream;
    use std::str::FromStr;

    pub use error::Error;
    pub use platform::Driver;
    pub use platform::ImportedDevice;
    pub use platform::UsbId;

    pub type Result<T> = std::result::Result<T, Error>;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum HubSpeed {
        High = 0,
        Super,
    }

    impl From<HubSpeed> for crate::DeviceSpeed {
        fn from(value: HubSpeed) -> Self {
            match value {
                HubSpeed::High => crate::DeviceSpeed::High,
                HubSpeed::Super => crate::DeviceSpeed::Super,
            }
        }
    }

    impl TryFrom<crate::DeviceSpeed> for HubSpeed {
        type Error = crate::TryFromDeviceSpeedError;

        fn try_from(value: crate::DeviceSpeed) -> std::result::Result<Self, Self::Error> {
            match value {
                crate::DeviceSpeed::High => Ok(Self::High),
                crate::DeviceSpeed::Super => Ok(Self::Super),
                _ => Err(Self::Error::Invalid),
            }
        }
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
    pub(crate) struct ImportedDeviceInner {
        pub(crate) hub: HubSpeed,
        pub(crate) port: u16,
        pub(crate) status: DeviceStatus,
        pub(crate) vendor: u16,
        pub(crate) product: u16,
        pub(crate) devid: u32,
        pub(crate) udev: UsbDevice,
    }

    impl ImportedDeviceInner {
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
            self.devid
        }

        pub const fn product(&self) -> u16 {
            self.product
        }

        pub const fn usb_dev(&self) -> &UsbDevice {
            &self.udev
        }
    }

    #[derive(Debug, Clone)]
    pub(crate) struct UsbIdInner<'a> {
        pub(crate) bus_id: &'a str,
    }

    impl UsbIdInner<'_> {
        pub const fn bus_id(&self) -> &str {
            self.bus_id
        }
    }

    pub trait VhciDriver: Sized + crate::util::__private::Sealed {
        fn open() -> Result<Self>;
        fn attach(&mut self, socket: TcpStream, usb_id: UsbId) -> Result<u16>;
        fn detach(&mut self, port: u16) -> Result<()>;
        fn imported_devices(&self) -> impl ExactSizeIterator<Item = &'_ ImportedDevice> + '_;
    }
}
