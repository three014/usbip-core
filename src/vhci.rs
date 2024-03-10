use std::{io, net::TcpStream, os::fd::AsRawFd, sync::atomic::AtomicUsize};

pub use error::Error;
use libusbip_sys::{
    usbip_names_free, usbip_vhci_attach_device2, usbip_vhci_driver_open, usbip_vhci_get_free_port,
};

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
pub struct VhciDriver;

impl VhciDriver {
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
            usbip_vhci_attach_device2(port, socket.as_raw_fd(), udev.devid(), udev.speed())
        };
        if rc != 0 {
            Err(Error::IoError(io::Error::last_os_error()))
        } else {
            Ok(port)
        }
    }
}

impl Drop for VhciDriver {
    fn drop(&mut self) {
        singleton::terminate(&STATE, || unsafe { usbip_names_free() });
    }
}
