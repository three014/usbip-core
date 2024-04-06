#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows {
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
                    ((dev_type as u32) << 16)
                        | ((access.0) << 14)
                        | ((function) << 2)
                        | (method as u32)
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
            ffi::OsString,
            fs::File,
            ops::Deref,
            os::windows::{
                ffi::OsStringExt,
                fs::OpenOptionsExt,
                io::{AsHandle, AsRawHandle},
            },
            path::PathBuf,
        };

        use windows::{
            core::{GUID, PCWSTR},
            Win32::{
                Devices::DeviceAndDriverInstallation::CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
                Foundation::HANDLE,
                Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE},
                System::IO::DeviceIoControl,
            },
        };

        use crate::{
            vhci::{inner, VhciDriver},
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

        pub struct PortRecord {
            port: i32,
            inner: inner::PortRecord,
        }

        pub struct WindowsImportedDevice {
            inner: inner::ImportedDevice,
            record: PortRecord,
            speed: crate::DeviceSpeed,
        }

        pub struct UsbId<'a> {
            inner: inner::UsbId<'a>,
        }

        impl<'a> Deref for UsbId<'a> {
            type Target = inner::UsbId<'a>;

            fn deref(&self) -> &Self::Target {
                &self.inner
            }
        }

        struct DriverInner {
            handle: File,
        }

        impl DriverInner {
            fn as_raw_handle(&self) -> HANDLE {
                HANDLE(self.handle.as_raw_handle() as isize)
            }

            fn try_open() -> crate::vhci::Result<Self> {
                let file = File::options()
                    .create(true)
                    .read(true)
                    .write(true)
                    .attributes((FILE_SHARE_READ | FILE_SHARE_WRITE).0)
                    .open(get_path()?)?;

                Ok(Self { handle: file })
            }

            fn imported_devices(&self) -> ::windows::core::Result<Box<[WindowsImportedDevice]>> {
                let mut result = Vec::<WindowsImportedDevice>::new();

                unsafe {
                    DeviceIoControl(
                        self.as_raw_handle(),
                        ioctl::GetImportedDevices::FUNCTION,
                        None,
                        0,
                        None,
                        0,
                        None,
                        None,
                    )?;
                };
                todo!()
            }
        }

        fn get_path() -> crate::vhci::Result<PathBuf> {
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

        pub struct WindowsVhciDriver {
            inner: DriverInner,
        }

        impl VhciDriver for WindowsVhciDriver {
            fn open() -> crate::vhci::Result<Self> {
                Ok(Self {
                    inner: DriverInner::try_open()?,
                })
            }

            fn attach(
                &mut self,
                socket: std::net::TcpStream,
                usb_id: UsbId,
            ) -> crate::vhci::Result<u16> {
                todo!()
            }

            fn detach(&mut self, port: u16) -> crate::vhci::Result<()> {
                todo!()
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
}
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
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Copy, Serialize, Deserialize)]
    #[repr(u32)]
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
}

use core::fmt;
use std::{num::ParseIntError, path::Path, str::FromStr};

use containers::stacktools::StackStr;
use serde::{Deserialize, Serialize};

pub use platform::USB_IDS;

pub const USBIP_VERSION: usize = 0x111;
pub const DEV_PATH_MAX: usize = 256;
pub const BUS_ID_SIZE: usize = 32;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
        &self.busid
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
        let status = match s.parse::<usize>().map_err(Self::Err::Parse)? {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbInterface {
    b_interface_class: u8,
    b_interface_subclass: u8,
    b_interface_protocol: u8,
    padding: util::__padding::Padding<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
