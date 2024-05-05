use windows::Win32::{
    Devices::DeviceAndDriverInstallation::{CM_MapCrToWin32Err, CONFIGRET},
    Foundation::WIN32_ERROR,
};

pub mod vhci {
    mod utils {
        pub mod ioctl {
            use windows::Win32::Storage::FileSystem::{
                FILE_ACCESS_RIGHTS, FILE_READ_DATA, FILE_WRITE_DATA,
            };

            #[repr(u32)]
            enum DeviceType {
                Unknown = ::windows::Win32::System::Ioctl::FILE_DEVICE_UNKNOWN,
            }

            #[repr(u32)]
            enum Method {
                Buffered = ::windows::Win32::System::Ioctl::METHOD_BUFFERED,
            }

            const fn ctl_code(
                dev_type: DeviceType,
                function: u32,
                method: Method,
                access: FILE_ACCESS_RIGHTS,
            ) -> u32 {
                // Taken from CTL_CODE macro from d4drvif.h
                ((dev_type as u32) << 16) | ((access.0) << 14) | ((function) << 2) | (method as u32)
            }

            const fn make(pre_function: PreFunction) -> u32 {
                ctl_code(
                    DeviceType::Unknown,
                    pre_function as u32,
                    Method::Buffered,
                    FILE_ACCESS_RIGHTS(FILE_READ_DATA.0 | FILE_WRITE_DATA.0),
                )
            }

            #[repr(u32)]
            enum PreFunction {
                PluginHardware = 0x800,
                PlugoutHardware,
                GetImportedDevices,
                SetPersistent,
                GetPersistent,
            }

            pub struct PluginHardware {}

            impl PluginHardware {
                pub const FUNCTION: u32 = make(PreFunction::PluginHardware);
            }

            pub struct PlugoutHardware {}

            impl PlugoutHardware {
                pub const FUNCTION: u32 = make(PreFunction::PlugoutHardware);
            }

            #[repr(C)]
            pub struct GetImportedDevices {}

            impl GetImportedDevices {
                pub const FUNCTION: u32 = make(PreFunction::GetImportedDevices);
            }

            pub struct SetPersistent;

            impl SetPersistent {
                pub const FUNCTION: u32 = make(PreFunction::SetPersistent);
            }

            pub struct GetPersistent;

            impl GetPersistent {
                pub const FUNCTION: u32 = make(PreFunction::GetPersistent);
            }
        }
        use windows::{
            core::{GUID, PCWSTR},
            Win32::{
                Devices::DeviceAndDriverInstallation::{
                    CM_Get_Device_Interface_ListW, CM_Get_Device_Interface_List_SizeW,
                    CM_GET_DEVICE_INTERFACE_LIST_FLAGS, CR_BUFFER_SMALL, CR_SUCCESS,
                },
                Foundation::{ERROR_INVALID_PARAMETER, ERROR_NOT_ENOUGH_MEMORY},
            },
        };

        use super::Win32Error;

        pub fn get_device_interface_list<P>(
            guid: GUID,
            pdeviceid: P,
            flags: CM_GET_DEVICE_INTERFACE_LIST_FLAGS,
        ) -> Result<Vec<u16>, Win32Error>
        where
            P: ::windows::core::IntoParam<PCWSTR> + Copy,
        {
            let mut v = Vec::<u16>::new();
            loop {
                let mut cch = 0;
                let ret = unsafe {
                    CM_Get_Device_Interface_List_SizeW(
                        std::ptr::addr_of_mut!(cch),
                        std::ptr::addr_of!(guid),
                        pdeviceid,
                        flags,
                    )
                };
                if ret != CR_SUCCESS {
                    break Err(Win32Error::from_cmret(ret, ERROR_INVALID_PARAMETER));
                }

                v.resize(cch as usize, 0);

                let ret = unsafe {
                    CM_Get_Device_Interface_ListW(
                        std::ptr::addr_of!(guid),
                        pdeviceid,
                        &mut v,
                        flags,
                    )
                };
                match ret {
                    CR_BUFFER_SMALL => continue,
                    CR_SUCCESS => break Ok(v),
                    err => break Err(Win32Error::from_cmret(err, ERROR_NOT_ENOUGH_MEMORY)),
                }
            }
        }
    }
    use std::{
        ffi::{c_char, OsString},
        fs::File,
        marker::{PhantomData, PhantomPinned},
        net::SocketAddr,
        os::windows::{ffi::OsStringExt, fs::OpenOptionsExt, io::AsRawHandle},
        path::PathBuf, pin::Pin, ptr::NonNull,
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
        vhci::{base, VhciDriver},
        windows::vhci::utils::ioctl,
    };

    use super::Win32Error;

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

        fn try_open() -> crate::vhci::Result<Self> {
            let file = File::options()
                .create(true)
                .read(true)
                .write(true)
                .attributes((FILE_SHARE_READ | FILE_SHARE_WRITE).0)
                .open(Self::path()?)?;

            Ok(Self { handle: file })
        }

        fn imported_devices(&self) -> ::windows::core::Result<Box<[WindowsImportedDevice]>> {
            let mut result = Vec::<IoCtlIdev>::new();
            let mut additional = 4;

            loop {
                result.reserve(additional);
                let result_size = result.capacity() * std::mem::size_of::<IoCtlIdev>();
                let mut bytes_returned = 0;

                if let Err(err) = unsafe {
                    DeviceIoControl(
                        self.as_raw_handle(),
                        ioctl::GetImportedDevices::FUNCTION,
                        None,
                        0,
                        Some(result.as_mut_ptr().cast()),
                        result_size.try_into().unwrap(),
                        Some(std::ptr::addr_of_mut!(bytes_returned)),
                        None,
                    )
                } {
                    match WIN32_ERROR::from_error(&err)
                        .expect("Unwrapping error from DeviceIoControl")
                    {
                        ERROR_INSUFFICIENT_BUFFER => {
                            additional <<= 1;
                        }
                        _ => Err(err)?,
                    }
                }
            }

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
