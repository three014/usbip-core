//! Ahh, the silly vhci module. This is where everything begins.

pub mod error2 {
    /// The error type for VHCI operations.
    #[derive(Debug)]
    pub enum Error {
        UserInput(Box<dyn std::error::Error>),
        NoFreePorts,
        PortNotInUse,
        DriverNotFound,
        WriteSys(std::io::Error),
        Net(crate::net::Error),
        #[cfg(windows)]
        MultipleDevInterfaces(usize),
    }

    impl From<std::io::Error> for Error {
        fn from(value: std::io::Error) -> Self {
            Self::WriteSys(value)
        }
    }

    impl core::fmt::Display for Error {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Error::UserInput(err) => write!(f, "Invalid user input: {err}"),
                Error::NoFreePorts => write!(f, "No free port on USB/IP hub"),
                Error::PortNotInUse => write!(f, "Port not in use"),
                Error::DriverNotFound => write!(f, "VHCI device not found, is the driver loaded?"),
                Error::WriteSys(io) => write!(f, "Driver I/O error: {io}"),
                Error::Net(net) => write!(f, "Net error: {net}"),
                #[cfg(windows)]
                Error::MultipleDevInterfaces(num) => write!(
                    f,
                    "Multiple instances of VHCI device interface found ({num})"
                ),
            }
        }
    }

    impl std::error::Error for Error {}
}

mod platform {
    #[cfg(unix)]
    pub use crate::unix::vhci2::{
        PortRecord, UnixImportedDevice as ImportedDevice,
        UnixImportedDevices as ImportedDevices, Driver, STATE_PATH,
    };

    #[cfg(windows)]
    pub use crate::windows::vhci::{
        PortRecord, WindowsImportedDevice as ImportedDevice,
        WindowsImportedDevices as ImportedDevices, WindowsVhciDriver as Driver, STATE_PATH,
    };
}

pub mod base {
    use std::net::SocketAddr;

    use crate::{containers::stacktools::StackStr, BUS_ID_SIZE};

    #[derive(Debug)]
    pub struct ImportedDevice {
        pub(crate) vendor: u16,
        pub(crate) product: u16,
        pub(crate) devid: u32,
    }

    impl ImportedDevice {
        pub const fn vendor(&self) -> u16 {
            self.vendor
        }

        pub const fn dev_id(&self) -> u32 {
            self.devid
        }

        pub const fn product(&self) -> u16 {
            self.product
        }

        pub const fn bus_num(&self) -> u32 {
            self.dev_id() >> 16
        }

        pub const fn dev_num(&self) -> u32 {
            self.dev_id() & 0x0000ffff
        }
    }

    #[derive(Debug)]
    pub struct PortRecord {
        pub(crate) host: SocketAddr,
        pub(crate) busid: StackStr<BUS_ID_SIZE>,
    }

    impl PortRecord {
        pub const fn host(&self) -> &SocketAddr {
            &self.host
        }

        pub fn bus_id(&self) -> &str {
            &*self.busid
        }
    }
}

use core::fmt;
use std::{str::FromStr, net::SocketAddr};

pub use platform::{Driver, ImportedDevice, ImportedDevices, PortRecord, STATE_PATH};

pub type Result<T> = std::result::Result<T, error2::Error>;


pub struct AttachArgs<'a> {
    pub host: SocketAddr,
    pub bus_id: &'a str,
}

/// The VHCI driver's supported USB device speeds.
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

/// An object that provides an interface
/// to the vhci driver.
///
/// The platform's vhci driver needs to be installed
/// and loaded for the driver operations to succeed.
pub struct VhciDriver {
    inner: Driver,
}

impl VhciDriver {
    /// Creates a new [`VhciDriver2`] from
    /// a platform-specific driver implementation.
    #[inline(always)]
    const fn new(inner: Driver) -> Self {
        Self { inner }
    }

    #[inline(always)]
    const fn get(&self) -> &Driver {
        &self.inner
    }

    #[inline(always)]
    fn get_mut(&mut self) -> &mut Driver {
        &mut self.inner
    }

    /// Opens the vhci driver.
    ///
    /// # Errors
    /// This function will return an error if
    /// the underlying kernel driver was not loaded.
    #[inline(always)]
    pub fn open() -> Result<Self> {
        Ok(Self::new(Driver::open()?))
    }

    /// Attaches a host's USB device to this device.
    ///
    /// # Platform-specific behavior
    /// On unix, this function assumes that a connection
    /// has already been established with the host system.
    ///
    /// On windows, this function will first attempt to establish
    /// a connection with the host.
    #[inline(always)]
    pub fn attach(&mut self, args: AttachArgs) -> Result<u16> {
        self.get_mut().attach(args)
    }

    #[inline(always)]
    pub fn detach(&mut self, port: u16) -> Result<()> {
        self.get_mut().detach(port)
    }

    /// Returns a list of usb devices that are
    /// currently attached to this device.
    ///
    /// Because other programs can interface with the
    /// kernel driver and attach/detach usb devices,
    /// this list can become outdated. Developers that
    /// would like to use this function to maintain
    /// a list for a user to view should call this
    /// function regularly to get an updated view.
    ///
    /// # Errors
    /// This function will return an error if the
    /// kernel driver malfunctions. On platforms
    /// that store the records in files, this
    /// function can error if those files were
    /// incorrectly modified.
    ///
    /// # Platform-specific behavior
    /// On windows, this function always allocates
    /// memory, even if there are no attached
    /// usb devices.
    #[inline(always)]
    pub fn imported_devices(&self) -> Result<ImportedDevices> {
        self.get().imported_devices()
    }
}
