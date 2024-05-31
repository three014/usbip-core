use windows::Win32::{
    Devices::DeviceAndDriverInstallation::{CM_MapCrToWin32Err, CONFIGRET},
    Foundation::WIN32_ERROR,
};

mod util;
pub mod vhci {
    pub mod ioctl;
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

    use windows::{
        core::{GUID, PCWSTR},
        Win32::{
            Devices::DeviceAndDriverInstallation::CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
            Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE},
        },
    };

    use crate::{
        containers::stacktools::StackStr,
        vhci::{base, error2::Error, AttachArgs},
        BUS_ID_SIZE,
    };

    pub use ioctl::DoorError;

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
        bus_id: StackStr<BUS_ID_SIZE>,
    }

    impl From<ioctl::OwnedDeviceLocation> for DeviceLocation {
        fn from(value: ioctl::OwnedDeviceLocation) -> Self {
            Self {
                host: value.host,
                bus_id: value.bus_id,
            }
        }
    }

    #[derive(Debug)]
    pub struct PortRecord {
        base: base::PortRecord,
        port: u16
    }

    impl From<ioctl::PortRecord> for PortRecord {
        fn from(value: ioctl::PortRecord) -> Self {
            let host = (&*value.host, value.service.parse().unwrap());
            Self {
                base: base::PortRecord {
                    host: host.to_socket_addrs().unwrap().next().unwrap(),
                    busid: value.busid,
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

    impl From<ioctl::ImportedDevice> for WindowsImportedDevice {
        fn from(value: ioctl::ImportedDevice) -> Self {
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

    impl<'a> From<AttachArgs<'a>> for ioctl::DeviceLocation<'a> {
        fn from(value: AttachArgs<'a>) -> Self {
            Self::new(value.host, value.bus_id)
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
            let port =
                ioctl::relay(self.as_handle(), ioctl::Attach::new(args.into())).map_err(|err| {
                    match err {
                        DoorError::Io(io) => Error::WriteSys(io),
                        _ => unreachable!("Developer error in parsing data"),
                    }
                })?;

            Ok(port)
        }

        fn detach(&mut self, port: u16) -> crate::vhci::Result<()> {
            ioctl::relay(self.as_handle(), ioctl::Detach::new(port)).map_err(|err| match err {
                DoorError::Io(io) => Error::WriteSys(io),
                _ => unreachable!("Developer error in parsing data"),
            })
        }

        fn imported_devices(&self) -> crate::vhci::Result<WindowsImportedDevices> {
            let idevs = ioctl::relay(self.as_handle(), ioctl::GetImportedDevices)
                .map_err(|err| match err {
                    DoorError::Io(io) => Error::WriteSys(io),
                    err => Err(err).unwrap(),
                })?
                .into_iter()
                .map(From::from)
                .collect::<Vec<_>>();

            Ok(WindowsImportedDevices(idevs.into_boxed_slice()))
        }

        fn persistent_devices(&self) -> crate::vhci::Result<Vec<DeviceLocation>> {
            let dev_locs = ioctl::relay(self.as_handle(), ioctl::GetPersistentDevices)
                .map_err(|err| match err {
                    DoorError::Io(io) => Error::WriteSys(io),
                    _ => unreachable!("Developer error in parsing data"),
                })?
                .into_iter()
                .map(From::from)
                .collect();

            Ok(dev_locs)
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
        fn detach_port_one() {
            let mut driver = WindowsVhciDriver::open().unwrap();

            if let Err(err) = driver.detach(1) {
                match err {
                    Error::WriteSys(io) if io.kind() == std::io::ErrorKind::NotConnected => {}
                    _ => panic!(),
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
