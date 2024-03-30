pub(crate) mod error;
mod platform {
    #[cfg(unix)]
    pub use crate::unix::vhci2::{
        UnixDriver as Driver, UnixImportedDevice as ImportedDevice, UsbId, STATE_PATH,
    };

    #[cfg(windows)]
    pub use crate::windows::vhci::{
        UsbId, WindowsImportedDevice as ImportedDevice, WindowsVhciDriver as Driver, STATE_PATH
    };
}

use crate::DeviceStatus;
use crate::UsbDevice;
use core::fmt;
use std::net::TcpStream;
use std::str::FromStr;

pub use error::Error;
pub use platform::{Driver, ImportedDevice, UsbId, STATE_PATH};

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
        match self {
            ParseHubSpeedError::Empty => write!(f, "Invalid hub speed"),
            ParseHubSpeedError::Invalid => write!(f, "Empty string"),
        }
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
