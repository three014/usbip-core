use std::io::Read;
use std::os::windows::io::{AsRawHandle, BorrowedHandle};

use bitflags::bitflags;
use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_MORE_DATA, HANDLE, WIN32_ERROR};
use windows::Win32::Storage::FileSystem::{FILE_ACCESS_RIGHTS, FILE_READ_DATA, FILE_WRITE_DATA};
use windows::Win32::System::Ioctl::{
    FILE_ANY_ACCESS, METHOD_BUFFERED, METHOD_IN_DIRECT, METHOD_NEITHER, METHOD_OUT_DIRECT,
};
use windows::Win32::System::IO::DeviceIoControl;

pub struct Reader<'a> {
    handle: BorrowedHandle<'a>,
    dev_type: DeviceType,
    ctl_read_num: u32,
    end_of_req: bool,
}

impl<'a> Reader<'a> {
    pub const fn new(handle: BorrowedHandle<'a>, dev_type: DeviceType, ctl_read_num: u32) -> Self {
        Self {
            handle,
            dev_type,
            ctl_read_num,
            end_of_req: false
        }
    }
}

impl<'a> Read for Reader<'a> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.end_of_req {
            return Ok(0);
        }

        let code = ControlCode(
            self.dev_type,
            RequiredAccess::READ_DATA,
            self.ctl_read_num,
            TransferMethod::Buffered,
        )
        .into_u32();
        let handle = HANDLE(self.handle.as_raw_handle() as isize);
        let mut bytes_returned = 0;

        // SAFETY: `buf` is a valid mutable slice
        if let Err(err) = unsafe {
            DeviceIoControl(
                handle,
                code,
                Some(buf.as_ptr().cast()),
                buf.len().try_into().unwrap(),
                Some(buf.as_mut_ptr().cast()),
                buf.len().try_into().unwrap(),
                Some(std::ptr::addr_of_mut!(bytes_returned)),
                None,
            )
        } {
            let win32_err =
                WIN32_ERROR::from_error(&err).expect("Converting error from DeviceIoControl");
            match win32_err {
                ERROR_MORE_DATA => Ok(bytes_returned.try_into().unwrap()),
                _ => Err(std::io::Error::last_os_error()),
            }
        } else {
            self.end_of_req = true;
            Ok(bytes_returned.try_into().unwrap())
        }
    }
}

#[repr(u32)]
enum Method {
    Buffered = METHOD_BUFFERED,
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

#[allow(dead_code)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DeviceType {
    Port8042,
    Acpi,
    Battery,
    Beep,
    BusExtender,
    Cdrom,
    CdromFileSystem,
    Changer,
    Controller,
    DataLink,
    Dfs,
    DfsFileSystem,
    DfsVolume,
    Disk,
    DiskFileSystem,
    Dvd,
    FileSystem,
    Fips,
    FullscreenVideo,
    InportPort,
    Keyboard,
    Ks,
    Ksec,
    Mailslot,
    MassStorage,
    MidiIn,
    MidiOut,
    Modem,
    Mouse,
    MultiUncProvider,
    NamedPipe,
    Network,
    NetworkBrowser,
    NetworkFileSystem,
    NetworkRedirector,
    Null,
    ParallelPort,
    PhysicalNetcard,
    Printer,
    Scanner,
    Screen,
    Serenum,
    SerialPort,
    SerialMousePort,
    Smartcard,
    Smb,
    Sound,
    Streams,
    Tape,
    TapeFileSystem,
    Termsrv,
    Transport,
    Unknown,
    Vdm,
    Video,
    VirtualDisk,
    WaveIn,
    WaveOut,
}

impl DeviceType {
    pub const fn into_u32(self) -> u32 {
        use windows::Win32::System::Ioctl::*;
        match self {
            DeviceType::Port8042 => FILE_DEVICE_8042_PORT,
            DeviceType::Acpi => FILE_DEVICE_ACPI,
            DeviceType::Battery => FILE_DEVICE_BATTERY,
            DeviceType::Beep => FILE_DEVICE_BEEP,
            DeviceType::BusExtender => FILE_DEVICE_BUS_EXTENDER,
            //DeviceType::Cdrom => FILE_DEVICE_CD_ROM,
            DeviceType::CdromFileSystem => FILE_DEVICE_CD_ROM_FILE_SYSTEM,
            DeviceType::Changer => FILE_DEVICE_CHANGER,
            DeviceType::Controller => FILE_DEVICE_CONTROLLER,
            DeviceType::DataLink => FILE_DEVICE_DATALINK,
            DeviceType::Dfs => FILE_DEVICE_DFS,
            DeviceType::DfsFileSystem => FILE_DEVICE_DFS_FILE_SYSTEM,
            DeviceType::DfsVolume => FILE_DEVICE_DFS_VOLUME,
            //DeviceType::Disk => FILE_DEVICE_DISK,
            DeviceType::DiskFileSystem => FILE_DEVICE_DISK_FILE_SYSTEM,
            //DeviceType::Dvd => FILE_DEVICE_DVD,
            DeviceType::FileSystem => FILE_DEVICE_FILE_SYSTEM,
            DeviceType::Fips => FILE_DEVICE_FIPS,
            DeviceType::FullscreenVideo => FILE_DEVICE_FULLSCREEN_VIDEO,
            DeviceType::InportPort => FILE_DEVICE_INPORT_PORT,
            DeviceType::Keyboard => FILE_DEVICE_KEYBOARD,
            DeviceType::Ks => FILE_DEVICE_KS,
            DeviceType::Ksec => FILE_DEVICE_KSEC,
            DeviceType::Mailslot => FILE_DEVICE_MAILSLOT,
            DeviceType::MassStorage => FILE_DEVICE_MASS_STORAGE,
            DeviceType::MidiIn => FILE_DEVICE_MIDI_IN,
            DeviceType::MidiOut => FILE_DEVICE_MIDI_OUT,
            DeviceType::Modem => FILE_DEVICE_MODEM,
            DeviceType::Mouse => FILE_DEVICE_MOUSE,
            DeviceType::MultiUncProvider => FILE_DEVICE_MULTI_UNC_PROVIDER,
            DeviceType::NamedPipe => FILE_DEVICE_NAMED_PIPE,
            DeviceType::Network => FILE_DEVICE_NETWORK,
            DeviceType::NetworkBrowser => FILE_DEVICE_NETWORK_BROWSER,
            DeviceType::NetworkFileSystem => FILE_DEVICE_NETWORK_FILE_SYSTEM,
            DeviceType::NetworkRedirector => FILE_DEVICE_NETWORK_REDIRECTOR,
            DeviceType::Null => FILE_DEVICE_NULL,
            DeviceType::ParallelPort => FILE_DEVICE_PARALLEL_PORT,
            DeviceType::PhysicalNetcard => FILE_DEVICE_PHYSICAL_NETCARD,
            DeviceType::Printer => FILE_DEVICE_PRINTER,
            DeviceType::Scanner => FILE_DEVICE_SCANNER,
            DeviceType::Screen => FILE_DEVICE_SCREEN,
            DeviceType::Serenum => FILE_DEVICE_SERENUM,
            DeviceType::SerialMousePort => FILE_DEVICE_SERIAL_MOUSE_PORT,
            DeviceType::SerialPort => FILE_DEVICE_SERIAL_PORT,
            //DeviceType::Smartcard => FILE_DEVICE_SMARTCARD,
            DeviceType::Smb => FILE_DEVICE_SMB,
            DeviceType::Sound => FILE_DEVICE_SOUND,
            DeviceType::Streams => FILE_DEVICE_STREAMS,
            //DeviceType::Tape => FILE_DEVICE_TAPE,
            DeviceType::TapeFileSystem => FILE_DEVICE_TAPE_FILE_SYSTEM,
            DeviceType::Termsrv => FILE_DEVICE_TERMSRV,
            DeviceType::Transport => FILE_DEVICE_TRANSPORT,
            DeviceType::Unknown => FILE_DEVICE_UNKNOWN,
            DeviceType::Vdm => FILE_DEVICE_VDM,
            DeviceType::Video => FILE_DEVICE_VIDEO,
            DeviceType::VirtualDisk => FILE_DEVICE_VIRTUAL_DISK,
            DeviceType::WaveIn => FILE_DEVICE_WAVE_IN,
            DeviceType::WaveOut => FILE_DEVICE_WAVE_OUT,
            _ => unimplemented!(),
        }
    }

    pub const fn from_u32(value: u32) -> Self {
        use windows::Win32::System::Ioctl::*;
        match value {
            FILE_DEVICE_8042_PORT => DeviceType::Port8042,
            FILE_DEVICE_ACPI => DeviceType::Acpi,
            FILE_DEVICE_BATTERY => DeviceType::Battery,
            FILE_DEVICE_BEEP => DeviceType::Beep,
            FILE_DEVICE_BUS_EXTENDER => DeviceType::BusExtender,
            //FILE_DEVICE_CD_ROM => DeviceType::Cdrom,
            FILE_DEVICE_CD_ROM_FILE_SYSTEM => DeviceType::CdromFileSystem,
            FILE_DEVICE_CHANGER => DeviceType::Changer,
            FILE_DEVICE_CONTROLLER => DeviceType::Controller,
            FILE_DEVICE_DATALINK => DeviceType::DataLink,
            FILE_DEVICE_DFS => DeviceType::Dfs,
            FILE_DEVICE_DFS_FILE_SYSTEM => DeviceType::DfsFileSystem,
            FILE_DEVICE_DFS_VOLUME => DeviceType::DfsVolume,
            //FILE_DEVICE_DISK => DeviceType::Disk,
            FILE_DEVICE_DISK_FILE_SYSTEM => DeviceType::DiskFileSystem,
            //FILE_DEVICE_DVD => DeviceType::Dvd,
            FILE_DEVICE_FILE_SYSTEM => DeviceType::FileSystem,
            FILE_DEVICE_FIPS => DeviceType::Fips,
            FILE_DEVICE_FULLSCREEN_VIDEO => DeviceType::FullscreenVideo,
            FILE_DEVICE_INPORT_PORT => DeviceType::InportPort,
            FILE_DEVICE_KEYBOARD => DeviceType::Keyboard,
            FILE_DEVICE_KS => DeviceType::Ks,
            FILE_DEVICE_KSEC => DeviceType::Ksec,
            FILE_DEVICE_MAILSLOT => DeviceType::Mailslot,
            FILE_DEVICE_MASS_STORAGE => DeviceType::MassStorage,
            FILE_DEVICE_MIDI_IN => DeviceType::MidiIn,
            FILE_DEVICE_MIDI_OUT => DeviceType::MidiOut,
            FILE_DEVICE_MODEM => DeviceType::Modem,
            FILE_DEVICE_MOUSE => DeviceType::Mouse,
            FILE_DEVICE_MULTI_UNC_PROVIDER => DeviceType::MultiUncProvider,
            FILE_DEVICE_NAMED_PIPE => DeviceType::NamedPipe,
            FILE_DEVICE_NETWORK => DeviceType::Network,
            FILE_DEVICE_NETWORK_BROWSER => DeviceType::NetworkBrowser,
            FILE_DEVICE_NETWORK_FILE_SYSTEM => DeviceType::NetworkFileSystem,
            FILE_DEVICE_NETWORK_REDIRECTOR => DeviceType::NetworkRedirector,
            FILE_DEVICE_NULL => DeviceType::Null,
            FILE_DEVICE_PARALLEL_PORT => DeviceType::ParallelPort,
            FILE_DEVICE_PHYSICAL_NETCARD => DeviceType::PhysicalNetcard,
            FILE_DEVICE_PRINTER => DeviceType::Printer,
            FILE_DEVICE_SCANNER => DeviceType::Scanner,
            FILE_DEVICE_SCREEN => DeviceType::Screen,
            FILE_DEVICE_SERENUM => DeviceType::Serenum,
            FILE_DEVICE_SERIAL_MOUSE_PORT => DeviceType::SerialMousePort,
            FILE_DEVICE_SERIAL_PORT => DeviceType::SerialPort,
            //FILE_DEVICE_SMARTCARD => DeviceType::Smartcard,
            FILE_DEVICE_SMB => DeviceType::Smb,
            FILE_DEVICE_SOUND => DeviceType::Sound,
            FILE_DEVICE_STREAMS => DeviceType::Streams,
            //FILE_DEVICE_TAPE => DeviceType::Tape,
            FILE_DEVICE_TAPE_FILE_SYSTEM => DeviceType::TapeFileSystem,
            FILE_DEVICE_TERMSRV => DeviceType::Termsrv,
            FILE_DEVICE_TRANSPORT => DeviceType::Transport,
            FILE_DEVICE_UNKNOWN => DeviceType::Unknown,
            FILE_DEVICE_VDM => DeviceType::Vdm,
            FILE_DEVICE_VIDEO => DeviceType::Video,
            FILE_DEVICE_VIRTUAL_DISK => DeviceType::VirtualDisk,
            FILE_DEVICE_WAVE_IN => DeviceType::WaveIn,
            FILE_DEVICE_WAVE_OUT => DeviceType::WaveOut,
            _ => DeviceType::Unknown,
        }
    }
}

impl Into<u32> for DeviceType {
    fn into(self) -> u32 {
        self.into_u32()
    }
}

impl From<u32> for DeviceType {
    fn from(value: u32) -> Self {
        Self::from_u32(value)
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct RequiredAccess: u32 {
        const ANY_ACCESS = FILE_ANY_ACCESS;
        const READ_DATA = FILE_READ_DATA.0;
        const WRITE_DATA = FILE_WRITE_DATA.0;
        const READ_WRITE_DATA = RequiredAccess::READ_DATA.bits() | RequiredAccess::WRITE_DATA.bits();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum TransferMethod {
    Neither = METHOD_NEITHER,
    InputDirect = METHOD_IN_DIRECT,
    OutputDirect = METHOD_OUT_DIRECT,
    Buffered = METHOD_BUFFERED,
}

impl TransferMethod {
    pub const fn from_u32(value: u32) -> Self {
        match value & 0x3 {
            METHOD_NEITHER => Self::Neither,
            METHOD_IN_DIRECT => Self::InputDirect,
            METHOD_OUT_DIRECT => Self::OutputDirect,
            METHOD_BUFFERED => Self::Buffered,
            _ => unreachable!(),
        }
    }

    pub const fn into_u32(self) -> u32 {
        match self {
            Self::Neither => METHOD_NEITHER,
            Self::InputDirect => METHOD_IN_DIRECT,
            Self::OutputDirect => METHOD_OUT_DIRECT,
            Self::Buffered => METHOD_BUFFERED,
        }
    }
}

impl From<u32> for TransferMethod {
    fn from(value: u32) -> Self {
        Self::from_u32(value)
    }
}

impl Into<u32> for TransferMethod {
    fn into(self) -> u32 {
        self.into_u32()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlCode(DeviceType, RequiredAccess, u32, TransferMethod);

impl ControlCode {
    const METHOD_BITS: usize = 2;
    const NUM_BITS: usize = 12;
    const ACCESS_BITS: usize = 2;
    const TYPE_BITS: usize = 16;

    const METHOD_SHIFT: usize = 0;
    const NUM_SHIFT: usize = Self::METHOD_SHIFT + Self::METHOD_BITS;
    const ACCESS_SHIFT: usize = Self::NUM_SHIFT + Self::NUM_BITS;
    const TYPE_SHIFT: usize = Self::ACCESS_SHIFT + Self::ACCESS_BITS;

    const METHOD_MASK: u32 = (1 << Self::METHOD_BITS) - 1;
    const NUM_MASK: u32 = (1 << Self::NUM_BITS) - 1;
    const ACCESS_MASK: u32 = (1 << Self::ACCESS_BITS) - 1;
    const TYPE_MASK: u32 = (1 << Self::TYPE_BITS) - 1;

    pub const fn dev_type(&self) -> DeviceType {
        self.0
    }

    pub const fn required_access(&self) -> RequiredAccess {
        self.1
    }

    pub const fn num(&self) -> u32 {
        self.2
    }

    pub const fn transfer_method(&self) -> TransferMethod {
        self.3
    }

    pub const fn from_u32(value: u32) -> Self {
        let method = (value >> Self::METHOD_SHIFT) & Self::METHOD_MASK;
        let num = (value >> Self::NUM_SHIFT) & Self::NUM_MASK;
        let access = (value >> Self::ACCESS_SHIFT) & Self::ACCESS_MASK;
        let ty = (value >> Self::TYPE_SHIFT) & Self::TYPE_MASK;

        Self(
            DeviceType::from_u32(ty),
            if let Some(req_access) = RequiredAccess::from_bits(access) {
                req_access
            } else {
                RequiredAccess::READ_DATA
            },
            num,
            TransferMethod::from_u32(method),
        )
    }

    pub const fn into_u32(self) -> u32 {
        let method = self.transfer_method().into_u32() << Self::METHOD_SHIFT;
        let num = self.num() << Self::NUM_SHIFT;
        let access = self.required_access().bits() << Self::ACCESS_SHIFT;
        let ty = self.dev_type().into_u32() << Self::TYPE_SHIFT;

        ty | access | num | method
    }
}

impl From<u32> for ControlCode {
    fn from(value: u32) -> Self {
        Self::from_u32(value)
    }
}

impl Into<u32> for ControlCode {
    fn into(self) -> u32 {
        self.into_u32()
    }
}

#[macro_export]
macro_rules! ioctl_none {
    ($(#[$attr:meta])* $name:ident, $dev_ty:expr, $nr:expr) => {
        $(#[$attr])*
        pub unsafe fn $name(handle: *mut std::ffi::c_void) -> Result<u32, $crate::Error> {
            let code = $crate::ControlCode(
                $dev_ty,
                $crate::RequiredAccess::ANY_ACCESS,
                $nr,
                $crate::TransferMethod::Neither,
            ).into();
            let mut return_value = 0;

            let status = $crate::DeviceIoControl(
                handle as _,
                code,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                0,
                &mut return_value,
                std::ptr::null_mut(),
            ) != 0;

            match status {
                true => Ok(return_value),
                _ => Err(std::io::Error::last_os_error())?,
            }
        }
    }
}

#[macro_export]
macro_rules! ioctl_read {
    ($(#[$attr:meta])* $name:ident, $dev_ty:expr, $nr:expr, $ty:ty) => {
        $(#[$attr])*
        pub unsafe fn $name(handle: *mut std::ffi::c_void, data: *mut $ty) -> Result<u32, $crate::Error> {
            let code = $crate::ControlCode(
                $dev_ty,
                $crate::RequiredAccess::READ_DATA,
                $nr,
                $crate::TransferMethod::Buffered,
            ).into();
            let mut return_value = 0;

            let status = $crate::DeviceIoControl(
                handle as _,
                code,
                data as _,
                std::mem::size_of::<$ty>() as _,
                data as _,
                std::mem::size_of::<$ty>() as _,
                &mut return_value,
                std::ptr::null_mut(),
            ) != 0;

            match status {
                true => Ok(return_value),
                _ => Err(std::io::Error::last_os_error())?,
            }
        }
    }
}

#[macro_export]
macro_rules! ioctl_write {
    ($(#[$attr:meta])* $name:ident, $dev_ty:expr, $nr:expr, $ty:ty) => {
        $(#[$attr])*
        pub unsafe fn $name(handle: *mut std::ffi::c_void, data: *const $ty) -> Result<u32, $crate::Error> {
            let code = $crate::ControlCode(
                $dev_ty,
                $crate::RequiredAccess::WRITE_DATA,
                $nr,
                $crate::TransferMethod::Buffered,
            ).into();
            let mut return_value = 0;

            let status = $crate::DeviceIoControl(
                handle as _,
                code,
                data as _,
                std::mem::size_of::<$ty>() as _,
                std::ptr::null_mut(),
                0,
                &mut return_value,
                std::ptr::null_mut(),
            ) != 0;

            match status {
                true => Ok(return_value),
                _ => Err(std::io::Error::last_os_error())?,
            }
        }
    }
}

#[macro_export]
macro_rules! ioctl_readwrite {
    ($(#[$attr:meta])* $name:ident, $dev_ty:expr, $nr:expr, $ty:ty) => {
        $(#[$attr])*
        pub unsafe fn $name(handle: *mut std::ffi::c_void, data: *mut $ty) -> Result<u32, $crate::Error> {
            let code = $crate::ControlCode(
                $dev_ty,
                $crate::RequiredAccess::READ_WRITE_DATA,
                $nr,
                $crate::TransferMethod::Buffered,
            ).into();
            let mut return_value = 0;

            let status = $crate::DeviceIoControl(
                handle as _,
                code,
                data as _,
                std::mem::size_of::<$ty>() as _,
                data as _,
                std::mem::size_of::<$ty>() as _,
                &mut return_value,
                std::ptr::null_mut(),
            ) != 0;

            match status {
                true => Ok(return_value),
                _ => Err(std::io::Error::last_os_error())?,
            }
        }
    }
}
