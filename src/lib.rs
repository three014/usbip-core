#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;
mod platform {
    #[cfg(unix)]
    pub use crate::unix::USB_IDS;
    #[cfg(windows)]
    pub use crate::windows::USB_IDS;
}
pub mod names;
pub mod vhci;
pub mod containers {
    pub mod beef;
    pub mod singleton;
    pub mod stacktools;
}
mod util;
pub mod net {
    use core::fmt;

    use bincode::config::{BigEndian, Configuration, Fixint};

    /// The result of a USB/IP network request.
    #[derive(Debug, Clone, Copy, bincode::Encode, bincode::Decode)]
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

    /// Returns the [`bincode::Configuration`] used
    /// for network communication.
    ///
    /// The current config is no limit on transfers, big endian, and fixed int encoding.
    ///
    /// [`bincode::Configuration`]: bincode::config::Configuration
    pub const fn bincode_config() -> Configuration<BigEndian, Fixint> {
        bincode::config::standard()
            .with_no_limit()
            .with_big_endian()
            .with_fixed_int_encoding()
    }
}

use core::fmt;
use std::{num::ParseIntError, path::Path, str::FromStr};

use bincode::de::read::Reader;
use containers::stacktools::StackStr;

pub use platform::USB_IDS;

pub const USBIP_VERSION: usize = 0x111;
pub const DEV_PATH_MAX: usize = 256;
pub const BUS_ID_SIZE: usize = 32;

#[derive(Debug, bincode::Encode, bincode::Decode)]
pub struct UsbDevice {
    path: StackStr<DEV_PATH_MAX>,
    busid: StackStr<BUS_ID_SIZE>,
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
        self.path.as_path()
    }

    pub fn bus_id(&self) -> &str {
        &*self.busid
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

/// The state of a [`vhci`] device port.
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
        match self {
            ParseDeviceStatusError::Invalid => write!(f, "Invalid device status"),
            ParseDeviceStatusError::Parse(p) => write!(f, "{p}"),
        }
    }
}

impl std::error::Error for ParseDeviceStatusError {}

impl FromStr for DeviceStatus {
    type Err = ParseDeviceStatusError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let status = match s.parse::<u8>().map_err(Self::Err::Parse)? {
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

#[derive(Debug, Clone, Copy)]
pub struct UsbInterface {
    b_interface_class: u8,
    b_interface_subclass: u8,
    b_interface_protocol: u8,
}

impl bincode::Encode for UsbInterface {
    fn encode<E: bincode::enc::Encoder>(
        &self,
        encoder: &mut E,
    ) -> Result<(), bincode::error::EncodeError> {
        bincode::Encode::encode(&self.b_interface_class, encoder)?;
        bincode::Encode::encode(&self.b_interface_subclass, encoder)?;
        bincode::Encode::encode(&self.b_interface_protocol, encoder)?;
        bincode::Encode::encode(&0u8, encoder)?;
        Ok(())
    }
}

impl bincode::Decode for UsbInterface {
    fn decode<D: bincode::de::Decoder>(
        decoder: &mut D,
    ) -> Result<Self, bincode::error::DecodeError> {
        let b_interface_class = u8::decode(decoder)?;
        let b_interface_subclass = u8::decode(decoder)?;
        let b_interface_protocol = u8::decode(decoder)?;
        decoder.claim_bytes_read(core::mem::size_of::<u8>())?;
        decoder.reader().consume(core::mem::size_of::<u8>());

        Ok(UsbInterface {
            b_interface_class,
            b_interface_subclass,
            b_interface_protocol,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, bincode::Decode, bincode::Encode)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_speed_sizeof_i32() {
        assert_eq!(
            std::mem::size_of::<DeviceSpeed>(),
            std::mem::size_of::<i32>()
        );
    }
}
