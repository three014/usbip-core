use crate::{ffi, DeviceStatus, UsbDevice};
use std::{io, net::TcpStream, os::fd::AsRawFd, sync::atomic::AtomicUsize};

pub use error::Error;
use ffi::{
    usbip_imported_device, usbip_names_free, usbip_vhci_attach_device2, usbip_vhci_driver_open,
    usbip_vhci_get_free_port, vhci_driver, usbip_vhci_detach_device,
};

pub use ffi::VHCI_STATE_PATH as STATE_PATH;

use crate::{
    util::singleton::{self, UNINITIALIZED},
    Info,
};
mod error {
    use std::{fmt, io};

    #[derive(Debug)]
    pub enum Error {
        OpenFailed,
        NoFreePorts,
        ImportFailed,
        AlreadyOpen,
        IoError(io::Error),
    }

    impl fmt::Display for Error {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Error::OpenFailed => write!(f, "open vhci_driver failed (is vhci_hcd loaded?)"),
                Error::NoFreePorts => write!(f, "no free ports"),
                Error::ImportFailed => write!(f, "import device failed"),
                Error::AlreadyOpen => write!(f, "already opened for this process"),
                Error::IoError(io) => write!(f, "I/O error: {io}"),
            }
        }
    }

    impl std::error::Error for Error {}
}

static STATE: AtomicUsize = AtomicUsize::new(UNINITIALIZED);
#[derive(Debug)]
pub struct Driver;

impl Driver {
    pub fn try_open() -> Result<Self, Error> {
        let result = singleton::try_init(&STATE, || {
            let rc = unsafe { usbip_vhci_driver_open() };
            if rc < 0 {
                Err(Error::OpenFailed)
            } else {
                Ok(Self)
            }
        });

        result.map_err(|err| match err {
            singleton::Error::AlreadyInit => Error::AlreadyOpen,
            singleton::Error::UserSpecified(err) => err,
            singleton::Error::AlreadyFailed => Error::OpenFailed,
        })
    }

    fn get_free_port(&self, speed: u32) -> Result<u8, Error> {
        let _ = self;
        let port = unsafe { usbip_vhci_get_free_port(speed) };
        if port < 0 {
            Err(Error::NoFreePorts)
        } else {
            Ok(port as u8)
        }
    }

    pub fn try_attach_dev(&self, socket: &TcpStream, udev: &Info) -> Result<u8, Error> {
        let port = self.get_free_port(udev.speed())?;
        let rc = unsafe {
            usbip_vhci_attach_device2(port, socket.as_raw_fd(), udev.dev_id(), udev.speed())
        };
        if rc != 0 {
            Err(Error::IoError(io::Error::last_os_error()))
        } else {
            Ok(port)
        }
    }

    pub fn try_detach_dev(&self, port: u8) -> Result<(), Error> {
        let rc = unsafe {
            usbip_vhci_detach_device(port)
        };
        if rc < 0 {
            Err(Error::IoError(io::Error::last_os_error()))
        } else {
            Ok(())
        }
    }

    fn ffi_imported_devices(&self) -> &[usbip_imported_device] {
        let _ = self;
        // SAFETY: By entering this function, this thread is the
        // only thread that can access the vhci driver struct, therefore
        // the data cannot be mutated while we're working with it.
        // Furthermore, we ensured that the driver was allocated when
        // we initialized VhciDriver, so the data cannot be null.
        unsafe { (*vhci_driver).idev.as_slice((*vhci_driver).nports as usize) }
    }

    pub fn imported_devices(&self) -> impl ExactSizeIterator<Item = ImportedDevice> + '_ {
        self.ffi_imported_devices()
            .iter()
            .map(std::convert::Into::into)
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        singleton::terminate(&STATE, || unsafe { usbip_names_free() });
    }
}

#[derive(Debug, Clone, Copy)]
pub enum HubSpeed {
    High = 0,
    Super,
}

impl From<ffi::hub_speed> for HubSpeed {
    fn from(value: ffi::hub_speed) -> Self {
        match value {
            ffi::hub_speed::HUB_SPEED_HIGH => HubSpeed::High,
            ffi::hub_speed::HUB_SPEED_SUPER => HubSpeed::Super,
        }
    }
}

#[derive(Debug)]
pub struct ImportedDevice {
    hub: HubSpeed,
    port: u8,
    status: DeviceStatus,
    udev: UsbDevice,
}

impl ImportedDevice {
    pub fn as_udev(&self) -> &UsbDevice {
        &self.udev
    }

    pub fn hub(&self) -> HubSpeed {
        self.hub
    }

    pub fn port(&self) -> u8 {
        self.port
    }

    pub fn status(&self) -> DeviceStatus {
        self.status
    }
}

impl From<ffi::usbip_imported_device> for ImportedDevice {
    fn from(value: ffi::usbip_imported_device) -> Self {
        let udev: UsbDevice = value.udev.into();
        debug_assert_eq!(udev.info().dev_id(), value.devid);
        debug_assert_eq!(udev.busnum, u32::from(value.busnum));
        debug_assert_eq!(udev.devnum, u32::from(value.devnum));
        Self {
            hub: value.hub.into(),
            port: value.port,
            status: value.status.into(),
            udev,
        }
    }
}

impl From<&ffi::usbip_imported_device> for ImportedDevice {
    fn from(value: &ffi::usbip_imported_device) -> Self {
        value.clone().into()
    }
}

#[cfg(test)]
mod tests {
    use crate::DeviceStatus;

    use super::*;

    #[test]
    fn singleton_vhci_driver() {
        if let Ok(_x) = Driver::try_open() {
            Driver::try_open().expect_err("driver should've failed to open a second time");
        }
    }

    #[test]
    fn driver_is_allocated_on_success() {
        let Ok(_x) = Driver::try_open() else {
            return;
        };
        let ptr = unsafe { vhci_driver };
        assert!(!ptr.is_null())
    }

    #[test]
    fn iterate_imported_devices() {
        if let Ok(x) = Driver::try_open() {
            for idev in x.ffi_imported_devices() {
                println!(
                    "C Imported Device - port: {}, num: {}-{}, status: {}",
                    idev.port,
                    idev.busnum,
                    idev.devnum,
                    Into::<DeviceStatus>::into(idev.status)
                )
            }
        }
    }

    #[test]
    fn convert_ffi_idevs_into_rust_idevs() {
        if let Ok(x) = Driver::try_open() {
            for idev in x.ffi_imported_devices() {
                let rust_idev: ImportedDevice = idev.into();
                println!(
                    "Rust Imported Device - port: {}, num: {}-{}, status: {}",
                    rust_idev.port(),
                    rust_idev.as_udev().busnum,
                    rust_idev.as_udev().devnum,
                    rust_idev.status()
                )
            }
        }
    }
}
