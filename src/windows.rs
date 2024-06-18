use windows::Win32::{
    Devices::DeviceAndDriverInstallation::{CM_MapCrToWin32Err, CONFIGRET},
    Foundation::WIN32_ERROR,
};

mod util;
pub mod vhci {
    mod ioctl;
    pub mod ioctl2;
    use std::{
        ffi::OsString,
        fs::File,
        net::{SocketAddr, ToSocketAddrs},
        os::windows::{
            ffi::OsStringExt,
            fs::OpenOptionsExt,
            io::{AsHandle, BorrowedHandle},
        },
        path::PathBuf,
    };

    use ioctl2::DriverError;
    use windows::{
        core::{GUID, PCWSTR},
        Win32::{
            Devices::DeviceAndDriverInstallation::CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
            Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE},
        },
    };

    use crate::{
        vhci::{base, error2::Error, AttachArgs},
        BusId, BUS_ID_SIZE,
    };

    use super::util;

    pub static STATE_PATH: &str = "";
    const GUID_DEVINTERFACE_USB_HOST_CONTROLLER: GUID = GUID::from_values(
        0xB4030C06,
        0xDC5F,
        0x4FCC,
        [0x87, 0xEB, 0xE5, 0x51, 0x5A, 0x09, 0x35, 0xC0],
    );

    pub struct DeviceLocation {
        host: SocketAddr,
        busid: BusId<'static>,
    }

    impl From<ioctl2::DeviceLocation<'static>> for DeviceLocation {
        fn from(value: ioctl2::DeviceLocation<'static>) -> Self {
            let ioctl2::DeviceLocation { host, busid } = value;
            Self { host, busid }
        }
    }

    #[derive(Debug)]
    pub struct PortRecord {
        base: base::PortRecord,
        port: u16,
    }

    impl From<ioctl2::PortRecord<'_>> for PortRecord {
        fn from(value: ioctl2::PortRecord) -> Self {
            let host = (value.host.as_str(), value.service.as_str().parse().unwrap());
            Self {
                base: base::PortRecord {
                    host: host.to_socket_addrs().unwrap().next().unwrap(),
                    busid: value.busid.to_owned(),
                },
                port: value.port as u16,
            }
        }
    }

    #[derive(Debug)]
    pub struct WindowsImportedDevice {
        base: base::ImportedDevice,
        record: PortRecord,
        speed: crate::DeviceSpeed,
    }

    impl From<ioctl2::ImportedDevice<'_>> for WindowsImportedDevice {
        fn from(value: ioctl2::ImportedDevice) -> Self {
            Self {
                base: base::ImportedDevice {
                    vendor: value.vendor,
                    product: value.product,
                    devid: value.devid,
                },
                record: PortRecord::from(value.record),
                speed: value.speed,
            }
        }
    }

    #[derive(Debug)]
    pub struct WindowsImportedDevices(Box<[WindowsImportedDevice]>);

    impl WindowsImportedDevices {
        pub fn get(&self) -> &[WindowsImportedDevice] {
            &self.0
        }
    }

    #[derive(Debug, Clone, Copy)]
    pub struct TryFromAttachArgsErr;

    impl std::fmt::Display for TryFromAttachArgsErr {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(
                f,
                "the given busid was greater than BUS_ID_SIZE ({})",
                BUS_ID_SIZE - 1
            )
        }
    }

    impl std::error::Error for TryFromAttachArgsErr {}

    impl<'a> TryFrom<AttachArgs<'a>> for ioctl2::DeviceLocation<'a> {
        type Error = TryFromAttachArgsErr;

        fn try_from(value: AttachArgs<'a>) -> Result<Self, Self::Error> {
            Self::new(value.host, value.bus_id).ok_or(TryFromAttachArgsErr)
        }
    }

    impl From<win_deviceioctl::Error<DriverError>> for Error {
        fn from(err: win_deviceioctl::Error<DriverError>) -> Self {
            match err {
                win_deviceioctl::Error::Driver(DriverError::DevNotConnected) => {
                    Error::WriteSys(std::io::ErrorKind::NotConnected.into())
                }
                win_deviceioctl::Error::Driver(DriverError::IncompatibleProtocolVersion)
                | win_deviceioctl::Error::Driver(DriverError::InvalidAbi) => {
                    Error::WriteSys(std::io::ErrorKind::InvalidData.into())
                }
                win_deviceioctl::Error::Io(io) => Error::WriteSys(io),
                win_deviceioctl::Error::Driver(DriverError::FileNotFound) => {
                    Error::WriteSys(std::io::ErrorKind::NotFound.into())
                }
                _ => unreachable!("Dev error in parsing data"),
            }
        }
    }

    struct InnerDriver {
        handle: File,
    }

    impl InnerDriver {
        fn as_handle(&self) -> BorrowedHandle {
            self.handle.as_handle()
        }

        fn try_open() -> crate::vhci::Result<Self> {
            let file = File::options()
                .create(true)
                .read(true)
                .write(true)
                .attributes((FILE_SHARE_READ | FILE_SHARE_WRITE).0)
                .open(Self::path()?)?;

            Ok(Self { handle: file })
        }

        fn attach(&mut self, args: AttachArgs) -> crate::vhci::Result<u16> {
            let device_location = ioctl2::DeviceLocation::try_from(args)
                .map_err(|err| Error::UserInput(Box::from(err)))?;
            let port =
                win_deviceioctl::send_recv(self.as_handle(), ioctl2::Attach::new(device_location))
                    .map_err(Error::from)?;

            Ok(port)
        }

        fn detach(&mut self, port: u16) -> crate::vhci::Result<()> {
            win_deviceioctl::send(self.as_handle(), ioctl2::Detach::new(port)).map_err(Error::from)
        }

        fn imported_devices(&self) -> crate::vhci::Result<WindowsImportedDevices> {
            win_deviceioctl::send_recv(self.as_handle(), ioctl2::GetImportedDevices)
                .map_err(Error::from)
                .map(|vec| WindowsImportedDevices(vec.into_boxed_slice()))
        }

        fn persistent_devices(&self) -> crate::vhci::Result<Box<[DeviceLocation]>> {
            let devs = match win_deviceioctl::recv(self.as_handle(), ioctl2::GetPersistentDevices) {
                Ok(devs) => devs,
                Err(win_deviceioctl::Error::Driver(DriverError::FileNotFound)) => Vec::new(),
                Err(err) => Err(Error::from(err))?,
            };
            Ok(devs.into_iter().map(DeviceLocation::from).collect())
        }

        fn path() -> crate::vhci::Result<PathBuf> {
            let v = util::get_device_interface_list(
                GUID_DEVINTERFACE_USB_HOST_CONTROLLER,
                PCWSTR::null(),
                CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
            )
            .map_err(|err| std::io::Error::from_raw_os_error(err.get().to_hresult().0))?;
            let mut p = v.split(|&elm| elm == 0).filter(|slice| !slice.is_empty());
            if let Some(path) = p.next() {
                if p.next().is_some() {
                    // We add 2 because of the first slice and
                    // this second slice we just found.
                    Err(Error::MultipleDevInterfaces(2 + p.count()))
                } else {
                    Ok(PathBuf::from(OsString::from_wide(path)))
                }
            } else {
                Err(std::io::Error::from(std::io::ErrorKind::NotFound).into())
            }
        }
    }

    pub struct WindowsVhciDriver {
        inner: InnerDriver,
    }

    impl WindowsVhciDriver {
        #[inline(always)]
        pub fn open() -> crate::vhci::Result<Self> {
            Ok(Self {
                inner: InnerDriver::try_open()?,
            })
        }

        #[inline(always)]
        pub fn attach(&mut self, args: AttachArgs) -> crate::vhci::Result<u16> {
            self.inner.attach(args)
        }

        #[inline(always)]
        pub fn detach(&mut self, port: u16) -> crate::vhci::Result<()> {
            self.inner.detach(port)
        }

        #[inline(always)]
        pub fn imported_devices(&self) -> crate::vhci::Result<WindowsImportedDevices> {
            self.inner.imported_devices()
        }
    }

    pub trait WindowsVhciDriverExt {
        fn persistent_devices(&self) -> crate::vhci::Result<Box<[DeviceLocation]>>;
    }

    impl WindowsVhciDriverExt for WindowsVhciDriver {
        fn persistent_devices(&self) -> crate::vhci::Result<Box<[DeviceLocation]>> {
            self.inner.persistent_devices()
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn driver_can_open() {
            WindowsVhciDriver::open().unwrap();
        }

        #[test]
        fn imported_devices_doesnt_die() {
            let driver = WindowsVhciDriver::open().unwrap();
            driver.imported_devices().unwrap();
        }

        #[test]
        fn get_persistent_doesnt_die() {
            let driver = WindowsVhciDriver::open().unwrap();
            driver.persistent_devices().unwrap();
        }

        #[test]
        fn detach_port_one() {
            let mut driver = WindowsVhciDriver::open().unwrap();
            if let Err(err) = driver.detach(1) {
                match err {
                    Error::WriteSys(io) if io.kind() == std::io::ErrorKind::NotConnected => {}
                    err => Err(err).unwrap(),
                }
            }
        }
    }
}

pub static USB_IDS: &str = "";

struct Win32Error(WIN32_ERROR);

impl Win32Error {
    pub fn get(self) -> WIN32_ERROR {
        self.0
    }

    pub fn from_cmret(cm_ret: CONFIGRET, default_err: WIN32_ERROR) -> Self {
        let code = unsafe { CM_MapCrToWin32Err(cm_ret, default_err.0) };
        Self(WIN32_ERROR(code))
    }
}

impl From<Win32Error> for ::windows::core::Error {
    fn from(value: Win32Error) -> Self {
        ::windows::core::Error::from(value.get())
    }
}
