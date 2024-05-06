use windows::Win32::{
    Devices::DeviceAndDriverInstallation::{CM_MapCrToWin32Err, CONFIGRET},
    Foundation::WIN32_ERROR,
};

pub mod vhci {
    use std::{
        ffi::OsString,
        fs::File,
        net::SocketAddr,
        os::windows::{
            ffi::OsStringExt,
            fs::OpenOptionsExt,
            io::{AsHandle, BorrowedHandle},
        },
        path::PathBuf,
    };

    use bincode::error::DecodeError;
    use windows::{
        core::{GUID, PCWSTR},
        Win32::{
            Devices::DeviceAndDriverInstallation::CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
            Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE},
        },
    };

    use crate::{
        containers::stacktools::StackStr,
        vhci::{base, VhciDriver},
        windows::vhci::utils::ioctl,
        BUS_ID_SIZE,
    };

    use self::utils::{
        consts::{NI_MAXHOST, NI_MAXSERV},
        ioctl::DeviceType,
    };

    use super::Win32Error;

    mod utils;

    pub static STATE_PATH: &str = "";
    const GUID_DEVINTERFACE_USB_HOST_CONTROLLER: GUID = GUID::from_values(
        0xB4030C06,
        0xDC5F,
        0x4FCC,
        [0x87, 0xEB, 0xE5, 0x51, 0x5A, 0x09, 0x35, 0xC0],
    );

    #[derive(Debug)]
    pub struct AttachArgs<'a> {
        pub host: SocketAddr,
        pub bus_id: &'a str,
    }

    #[derive(Debug)]
    pub struct PortRecord {
        base: base::PortRecord,
    }

    #[derive(Debug)]
    pub struct WindowsImportedDevice {
        base: base::ImportedDevice,
        record: PortRecord,
        speed: crate::DeviceSpeed,
    }

    impl From<IoCtlIdev> for WindowsImportedDevice {
        fn from(value: IoCtlIdev) -> Self {
            Self {
                base: base::ImportedDevice {
                    port: value.port as u16,
                    vendor: value.vendor,
                    product: value.product,
                    devid: value.devid,
                },
                record: PortRecord {
                    base: base::PortRecord {
                        busid: value.busid,
                        host: SocketAddr::new(
                            value.host.parse().unwrap(),
                            value.service.parse().unwrap(),
                        ),
                    },
                },
                speed: value.speed,
            }
        }
    }

    #[derive(Debug, Clone, Copy)]
    enum IoctlFunction {
        PluginHardware = 0x800,
        PlugoutHardware,
        GetImportedDevices,
        SetPersistent,
        GetPersistent,
    }

    impl IoctlFunction {
        const fn as_u32(&self) -> u32 {
            *self as u32
        }
    }

    #[derive(bincode::Decode, bincode::Encode)]
    struct IoCtlIdev {
        port: i32,
        busid: StackStr<BUS_ID_SIZE>,
        service: StackStr<NI_MAXSERV>,
        host: StackStr<NI_MAXHOST>,
        devid: u32,
        speed: crate::DeviceSpeed,
        vendor: u16,
        product: u16,
    }

    #[derive(Debug)]
    pub struct WindowsImportedDevices(Box<[WindowsImportedDevice]>);

    impl WindowsImportedDevices {
        pub fn iter(&self) -> core::slice::Iter<'_, WindowsImportedDevice> {
            self.get().iter()
        }

        pub fn get(&self) -> &[WindowsImportedDevice] {
            &self.0
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

        fn imported_devices(
            &self,
        ) -> Result<Box<[WindowsImportedDevice]>, bincode::error::DecodeError> {
            let mut reader = ioctl::Reader::new(
                self.as_handle(),
                DeviceType::Unknown,
                IoctlFunction::GetImportedDevices.as_u32(),
            );

            let idevs: Vec<IoCtlIdev> =
                bincode::decode_from_std_read(&mut reader, crate::net::bincode_config())?;
            Ok(idevs.into_iter().map(From::from).collect())
        }

        fn path() -> crate::vhci::Result<PathBuf> {
            let v = utils::get_device_interface_list(
                GUID_DEVINTERFACE_USB_HOST_CONTROLLER,
                PCWSTR::null(),
                CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
            )?;
            let mut p = v.split(|&elm| elm == 0).filter(|slice| !slice.is_empty());
            if let Some(path) = p.next() {
                if p.next().is_some() {
                    // We add 2 because of the first slice and
                    // this second slice we just found.
                    Err(crate::vhci::Error::MultipleDevInterfaces(2 + p.count()))
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

    impl VhciDriver for WindowsVhciDriver {
        fn open() -> crate::vhci::Result<Self> {
            Ok(Self {
                inner: InnerDriver::try_open()?,
            })
        }

        fn attach(&mut self, args: AttachArgs) -> Result<u16, crate::vhci::error::AttachError> {
            todo!()
        }

        fn detach(&mut self, port: u16) -> crate::vhci::Result<()> {
            todo!()
        }

        fn imported_devices(&self) -> crate::vhci::Result<WindowsImportedDevices> {
            Ok(self
                .inner
                .imported_devices()
                .map(WindowsImportedDevices)
                .map_err(|err| match err {
                    DecodeError::Io { inner, .. } => inner,
                    _ => std::io::Error::other(err),
                })?)
        }
    }

    impl From<Win32Error> for crate::vhci::Error {
        fn from(value: Win32Error) -> Self {
            Self::Windows(value.into())
        }
    }

    impl crate::util::__private::Sealed for WindowsVhciDriver {}
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
