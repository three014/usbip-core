//! Ahh, the silly vhci module. This is where everything begins.

mod error2 {

    pub enum Error {
        NoFreePorts,
        PortNotInUse,
        DriverNotLoaded,
        Io(std::io::Error)
    }
}
pub(crate) mod error;
mod platform {
    #[cfg(unix)]
    pub use crate::unix::vhci2::{
        AttachArgs, PortRecord, UnixImportedDevice as ImportedDevice,
        UnixImportedDevices as ImportedDevices, UnixVhciDriver as Driver, STATE_PATH,
    };

    #[cfg(windows)]
    pub use crate::windows::vhci::{
        AttachArgs, PortRecord, WindowsImportedDevice as ImportedDevice,
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
            &self.busid
        }
    }
}

use core::fmt;
use std::str::FromStr;

pub use error::Error;
pub use platform::{AttachArgs, Driver, ImportedDevice, ImportedDevices, PortRecord, STATE_PATH};

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

/// An object that provides an interface
/// to the vhci driver.
///
/// # Platform-specific behavior
/// On Unix, the vhci_hcd kernel module needs to be loaded
/// to use the driver, and many actions require superuser
/// permissions.
///
/// On Windows, the usbip-win2 ude driver needs to be
/// installed.
pub struct VhciDriver2 {
    inner: Driver,
}

impl VhciDriver2 {

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
    pub fn attach(&mut self, args: AttachArgs) -> std::result::Result<u16, error::AttachError> {
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

