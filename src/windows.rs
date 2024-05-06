use windows::Win32::{
    Devices::DeviceAndDriverInstallation::{CM_MapCrToWin32Err, CONFIGRET},
    Foundation::WIN32_ERROR,
};

pub mod vhci {
    use std::{
        ffi::{c_char, OsString}, fs::File, io::Read, marker::{PhantomData, PhantomPinned}, net::SocketAddr, os::windows::{
            ffi::OsStringExt,
            fs::OpenOptionsExt,
            io::{AsHandle, AsRawHandle, BorrowedHandle},
        }, path::PathBuf, pin::Pin, ptr::NonNull
    };

    use windows::{
        core::{GUID, PCWSTR},
        Win32::{
            Devices::DeviceAndDriverInstallation::CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
            Foundation::{ERROR_INSUFFICIENT_BUFFER, HANDLE, WIN32_ERROR},
            Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE},
            System::IO::DeviceIoControl,
        },
    };

    use crate::{
        ioctl_read,
        vhci::{base, VhciDriver},
        windows::vhci::utils::ioctl,
    };

    use self::utils::ioctl::DeviceType;

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
        port: i32,
        base: base::PortRecord,
    }

    #[derive(Debug)]
    pub struct WindowsImportedDevice {
        base: base::ImportedDevice,
        record: PortRecord,
        speed: crate::DeviceSpeed,
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

    #[repr(C)]
    struct IoCtlIdev {
        port: i32,
        busid: [c_char; crate::BUS_ID_SIZE],
        service: [c_char; 32],
        host: [c_char; 1025],
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
        fn as_raw_handle(&self) -> HANDLE {
            HANDLE(self.handle.as_raw_handle() as isize)
        }

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

        fn imported_devices(&self) -> std::io::Result<Box<[WindowsImportedDevice]>> {
            let mut buf = Vec::<u8>::new();
            let mut reader = ioctl::Reader::new(
                self.as_handle(),
                DeviceType::Unknown,
                IoctlFunction::GetImportedDevices.as_u32(),
            );

            reader.read_to_end(&mut buf)?;

            todo!("Calculate number of idevs and cast safely")
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
            Ok(self.inner.imported_devices().map(WindowsImportedDevices)?)
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
