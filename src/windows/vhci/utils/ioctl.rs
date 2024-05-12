//! # Goals for this module
//! I want to define some sort of interface that specifies
//! an IoCtl function, control code, and a data layout suitable
//! for both the input/output.
//!
//! This data layout does not need to be in its final form,
//! but should avoid taking advantage of that fact through
//! "hacky" formats and tricks.
//!
//! # Motivation
//! Every function from the usbip-win2 userspace library
//! share a huge similarity in that they interact with
//! the DeviceIoControl windows syscall. It makes me
//! think of each function as a module that can be
//! added to the vhci driver to gain more functionality.
//! Therefore I want each module to be able to provide
//! DeviceIoControl with all the information it needs to
//! do its job, allow simultaneously presenting itself
//! as an opaque data type (until it is converted into
//! its usable form).
//!
//! # Current work
//! As of 5/8/2024, there is a [`IoControl`] trait that
//! defines a module's [`ControlCode`] based on the given
//! [`Function`]. Each module can also implement a separate
//! trait called [`EncodedSize`] which allows a type
//! to specify the amount of bytes they will take up in
//! their encoded format, which has proven useful since
//! the vhci driver requires the user to specify the size of
//! their data types for verification (I think).
//!
//! There's also a struct called [`Reader`], and its
//! job is to read data from the DeviceIoControl
//! based on input data that it currently calculates from
//! the [`IoControl`] and [`EncodedSize`] methods.
//!
//! Despite the existing traits, it feels clunky to
//! send data to the DeviceIoControl because the
//! existing data is not specific enough to
//! generate the right data. Also, while it felt right
//! at the time, it now feels weird to use the [`std::io::Read`]
//! and [`std::io::Write`] traits, since the assumptions
//! a user would have with those traits don't follow
//! for my current model.

use core::fmt;
use std::ffi::c_char;
use std::marker::PhantomData;
use std::net::SocketAddr;
use std::num::NonZeroU32;
use std::os::windows::io::{AsRawHandle, BorrowedHandle};

use bincode::de::read::Reader as _;
use bincode::{Decode, Encode};
use bitflags::bitflags;
use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_MORE_DATA, HANDLE, WIN32_ERROR};
use windows::Win32::Storage::FileSystem::{FILE_READ_DATA, FILE_WRITE_DATA};
use windows::Win32::System::Ioctl::{
    FILE_ANY_ACCESS, METHOD_BUFFERED, METHOD_IN_DIRECT, METHOD_NEITHER, METHOD_OUT_DIRECT,
};
use windows::Win32::System::IO::DeviceIoControl;

use crate::containers::iterators::BitShiftLeft;
use crate::containers::stacktools::StackStr;
use crate::util::EncodedSize;
use crate::{DeviceSpeed, BUS_ID_SIZE};

use super::consts::{NI_MAXHOST, NI_MAXSERV};

pub trait IoControl<T> {
    type Input: EncodedSize + bincode::Encode;
    // How to distiguish between containers and the actual type???
    type Output: EncodedSize + bincode::Decode;

    const FUNCTION: Function;

    fn ctrl_code() -> ControlCode {
        ControlCode(
            DeviceType::Unknown,
            RequiredAccess::READ_WRITE_DATA,
            <Self as IoControl<T>>::FUNCTION.as_u32(),
            TransferMethod::Buffered,
        )
    }

    fn send<E: bincode::enc::Encoder>(
        &self,
        encoder: &mut E,
    ) -> Result<(), bincode::error::EncodeError>;
    fn recv<D: bincode::de::Decoder>(decoder: &mut D) -> Result<T, bincode::error::DecodeError>;
}

#[derive(Debug, Clone, Copy)]
pub enum Function {
    PluginHardware = 0x800,
    PlugoutHardware,
    GetImportedDevices,
    SetPersistent,
    GetPersistent,
}

impl Function {
    pub const fn as_u32(&self) -> u32 {
        *self as u32
    }
}

pub struct DeviceLocation<'a> {
    host: SocketAddr,
    bus_id: &'a str,
}

unsafe impl EncodedSize for DeviceLocation<'_> {
    const ENCODED_SIZE_OF: usize = PortRecord::ENCODED_SIZE_OF;
}

impl bincode::Encode for DeviceLocation<'_> {
    fn encode<E: bincode::enc::Encoder>(
        &self,
        encoder: &mut E,
    ) -> Result<(), bincode::error::EncodeError> {
        PortRecord {
            port: 0,
            busid: StackStr::try_from(self.bus_id)
                .map_err(|err| bincode::error::EncodeError::UnexpectedEnd)?,
            service: StackStr::try_from(format_args!("{}", self.host.port()))
                .expect("converting a port number into a 32 byte stack string"),
            host: StackStr::try_from(format_args!("{}", self.host.ip()))
                .expect("converting ip address to 1025 byte stack str"),
        }
        .encode(encoder)
    }
}

impl<'a> DeviceLocation<'a> {
    pub const fn new(host: SocketAddr, bus_id: &'a str) -> Self {
        Self { host, bus_id }
    }
}

#[derive(bincode::Decode)]
pub struct Nothing;

unsafe impl EncodedSize for Nothing {
    const ENCODED_SIZE_OF: usize = 0;
}

pub struct Detach {
    port: Port
}

impl Detach {
    pub const fn new(port: u16) -> Self {
        Self {
            port: Port(port)
        }
    }
}

impl IoControl<()> for Detach {
    type Input = Port;
    type Output = Nothing;
    const FUNCTION: Function = Function::PlugoutHardware;
    
    fn send<E: bincode::enc::Encoder>(
        &self,
        encoder: &mut E,
    ) -> Result<(), bincode::error::EncodeError> {
        let size = Self::Input::ENCODED_SIZE_OF as u32;
        size.encode(encoder)?;
        self.port.encode(encoder)
    }
    
    fn recv<D: bincode::de::Decoder>(_: &mut D) -> Result<(), bincode::error::DecodeError> {
        Ok(())
    }
}

pub struct Attach<'a> {
    location: DeviceLocation<'a>,
}

impl<'a> Attach<'a> {
    pub const fn new(location: DeviceLocation<'a>) -> Self {
        Self { location }
    }
}

pub struct Port(u16);

impl bincode::Decode for Port {
    fn decode<D: bincode::de::Decoder>(
        decoder: &mut D,
    ) -> Result<Self, bincode::error::DecodeError> {
        let port = i32::decode(decoder)?;
        Ok(Port(port as u16))
    }
}

impl bincode::Encode for Port {
    fn encode<E: bincode::enc::Encoder>(
        &self,
        encoder: &mut E,
    ) -> Result<(), bincode::error::EncodeError> {
        let port = self.0 as i32;
        port.encode(encoder)
    }
}

unsafe impl EncodedSize for Port {
    const ENCODED_SIZE_OF: usize = core::mem::size_of::<i32>();
}

impl<'a> IoControl<u16> for Attach<'a> {
    type Input = DeviceLocation<'a>;
    type Output = Port;
    const FUNCTION: Function = Function::PluginHardware;

    fn send<E: bincode::enc::Encoder>(
        &self,
        encoder: &mut E,
    ) -> Result<(), bincode::error::EncodeError> {
        let size_of = (Self::Input::ENCODED_SIZE_OF + core::mem::size_of::<u32>()) as u32;
        size_of.encode(encoder)?;
        self.location.encode(encoder)?;
        Ok(())
    }

    fn recv<D: bincode::de::Decoder>(decoder: &mut D) -> Result<u16, bincode::error::DecodeError> {
        decoder.claim_bytes_read(core::mem::size_of::<u32>())?;
        decoder.reader().consume(core::mem::size_of::<u32>());
        let port = Port::decode(decoder)?;
        Ok(port.0)
    }
}

pub struct PortRecord {
    pub port: i32,
    pub busid: StackStr<BUS_ID_SIZE>,
    pub service: StackStr<NI_MAXSERV>,
    pub host: StackStr<NI_MAXHOST>,
}

unsafe impl EncodedSize for PortRecord {
    const ENCODED_SIZE_OF: usize = {
        #[repr(C)]
        struct EncodedPortRecord {
            port: i32,
            busid: [c_char; BUS_ID_SIZE],
            service: [c_char; NI_MAXSERV],
            host: [c_char; NI_MAXHOST],
        }

        core::mem::size_of::<EncodedPortRecord>()
    };
}

impl bincode::Decode for PortRecord {
    fn decode<D: bincode::de::Decoder>(
        decoder: &mut D,
    ) -> Result<Self, bincode::error::DecodeError> {
        use bincode::de::read::Reader as _;
        let port = i32::decode(decoder)?;
        let busid = StackStr::decode(decoder)?;
        let service = StackStr::decode(decoder)?;
        let host = StackStr::decode(decoder)?;
        // Account for padding from irregular struct size
        decoder.claim_bytes_read(3)?;
        decoder.reader().consume(3);

        Ok(Self {
            port,
            busid,
            service,
            host,
        })
    }
}

impl bincode::Encode for PortRecord {
    fn encode<E: bincode::enc::Encoder>(
        &self,
        encoder: &mut E,
    ) -> Result<(), bincode::error::EncodeError> {
        use bincode::enc::write::Writer;
        self.port.encode(encoder)?;
        self.busid.encode(encoder)?;
        self.service.encode(encoder)?;
        self.host.encode(encoder)?;
        encoder.writer().write(&[0, 0, 0])?;

        Ok(())
    }
}

pub struct GetImportedDevices;

impl IoControl<Vec<ImportedDevice>> for GetImportedDevices {
    type Input = SizeOf;
    type Output = ImportedDevice;
    const FUNCTION: Function = Function::GetImportedDevices;

    fn recv<D: bincode::de::Decoder>(
        decoder: &mut D,
    ) -> Result<Vec<Self::Output>, bincode::error::DecodeError> {
        let len = u32::decode(decoder)? as usize;
        let mut buf = Vec::with_capacity(len);

        decoder.claim_container_read::<[u8; Self::Output::ENCODED_SIZE_OF]>(len)?;

        for _ in 0..len {
            decoder.unclaim_bytes_read(Self::Output::ENCODED_SIZE_OF);

            let idev = Self::Output::decode(decoder)?;
            buf.push(idev);
        }

        Ok(buf)
    }

    fn send<E: bincode::enc::Encoder>(
        &self,
        encoder: &mut E,
    ) -> Result<(), bincode::error::EncodeError> {
        SizeOf(
            (Self::Output::ENCODED_SIZE_OF + core::mem::size_of::<u32>())
                .try_into()
                .unwrap(),
        )
        .encode(encoder)
    }
}

#[derive(bincode::Encode, bincode::Decode)]
pub struct SizeOf(u32);

unsafe impl EncodedSize for SizeOf {
    const ENCODED_SIZE_OF: usize = core::mem::size_of::<u32>();
}

pub struct ImportedDevice {
    pub record: PortRecord,
    pub devid: u32,
    pub speed: crate::DeviceSpeed,
    pub vendor: u16,
    pub product: u16,
}

unsafe impl EncodedSize for ImportedDevice {
    const ENCODED_SIZE_OF: usize = {
        #[repr(C)]
        struct EncodedImportedDevice {
            devid: u32,
            speed: crate::DeviceSpeed,
            vendor: u16,
            product: u16,
        }

        PortRecord::ENCODED_SIZE_OF + core::mem::size_of::<EncodedImportedDevice>()
    };
}

impl bincode::Encode for ImportedDevice {
    fn encode<E: bincode::enc::Encoder>(
        &self,
        encoder: &mut E,
    ) -> Result<(), bincode::error::EncodeError> {
        self.record.encode(encoder)?;
        self.devid.encode(encoder)?;
        self.speed.encode(encoder)?;
        self.vendor.encode(encoder)?;
        self.product.encode(encoder)?;

        Ok(())
    }
}

impl bincode::Decode for ImportedDevice {
    fn decode<D: bincode::de::Decoder>(
        decoder: &mut D,
    ) -> Result<Self, bincode::error::DecodeError> {
        let record = PortRecord::decode(decoder)?;
        let devid = u32::decode(decoder)?;
        let speed = DeviceSpeed::decode(decoder)?;
        let vendor = u16::decode(decoder)?;
        let product = u16::decode(decoder)?;

        Ok(Self {
            record,
            devid,
            speed,
            vendor,
            product,
        })
    }
}

#[derive(Debug)]
pub enum DoorError {
    Send(bincode::error::EncodeError),
    Recv(bincode::error::DecodeError),
    Io(std::io::Error),
}

impl fmt::Display for DoorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!()
    }
}

impl From<bincode::error::DecodeError> for DoorError {
    fn from(value: bincode::error::DecodeError) -> Self {
        DoorError::Recv(value)
    }
}

impl From<bincode::error::EncodeError> for DoorError {
    fn from(value: bincode::error::EncodeError) -> Self {
        DoorError::Send(value)
    }
}

impl From<std::io::Error> for DoorError {
    fn from(value: std::io::Error) -> Self {
        DoorError::Io(value)
    }
}

pub fn relay<T, I: IoControl<T>>(handle: BorrowedHandle, ioctl: I) -> Result<T, DoorError> {
    struct EncodeHelper<'a, T, I: IoControl<T>>(&'a I, PhantomData<&'a T>);
    impl<T, I: IoControl<T>> bincode::Encode for EncodeHelper<'_, T, I> {
        fn encode<E: bincode::enc::Encoder>(
            &self,
            encoder: &mut E,
        ) -> Result<(), bincode::error::EncodeError> {
            self.0.send(encoder)
        }
    }

    struct DecodeWrapper<T, I: IoControl<T>>(T, PhantomData<I>);
    impl<T, I: IoControl<T>> bincode::Decode for DecodeWrapper<T, I> {
        fn decode<D: bincode::de::Decoder>(
            decoder: &mut D,
        ) -> Result<Self, bincode::error::DecodeError> {
            let out = I::recv(decoder)?;
            Ok(Self(out, PhantomData))
        }
    }

    let code = I::ctrl_code().into_u32();
    let mut door = Door::new(handle, code);
    let input = bincode::encode_to_vec(
        EncodeHelper(&ioctl, PhantomData),
        crate::net::bincode_config().with_little_endian(),
    )?;
    let mut output = Vec::<u8>::new();
    let mut start = 0;

    for num_ioctl_devs in BitShiftLeft::new(NonZeroU32::new(1).unwrap(), 2) {
        output.resize(
            I::Output::ENCODED_SIZE_OF
                .checked_mul(num_ioctl_devs)
                .unwrap()
                .checked_add(core::mem::size_of::<u32>())
                .unwrap(),
            0,
        );

        match door.read_write(&input, &mut output[start..]) {
            Ok(0) => {
                output.resize(start, 0);
                break;
            }
            Ok(bytes_read) => {
                start += bytes_read;
            }
            Err(err) => {
                if err.kind() != std::io::ErrorKind::WriteZero {
                    return Err(err.into());
                }
            }
        }
    }

    let num_items =
        ((output.len() - core::mem::size_of::<u32>()) / I::Output::ENCODED_SIZE_OF) as u32;
    output[0..core::mem::size_of::<u32>()].copy_from_slice(&num_items.to_le_bytes());

    let output = bincode::decode_from_slice::<DecodeWrapper<T, I>, _>(
        &output,
        crate::net::bincode_config().with_little_endian(),
    )?
    .0;

    Ok(output.0)
}

/// Struct for keeping track of
/// [`IoControl`] operations. 
struct Door<'a> {
    end_of_req: bool,
    handle: BorrowedHandle<'a>,
    code: u32,
}

impl<'a> Door<'a> {
    const fn new(handle: BorrowedHandle<'a>, code: u32) -> Self {
        Self {
            end_of_req: false,
            handle,
            code,
        }
    }

    fn read_write(&mut self, input: &[u8], output: &mut [u8]) -> std::io::Result<usize> {
        if self.end_of_req {
            return Ok(0);
        }

        let code = self.code;
        let handle = HANDLE(self.handle.as_raw_handle() as isize);
        let mut bytes_returned: u32 = 0;

        // SAFETY: Both `input` and `output` are valid slices.
        let result = unsafe {
            DeviceIoControl(
                handle,
                code,
                Some(input.as_ptr().cast()),
                input.len() as u32,
                Some(output.as_mut_ptr().cast()),
                output.len() as u32,
                Some(core::ptr::addr_of_mut!(bytes_returned)),
                None,
            )
        };

        if let Err(err) = result {
            // We've hit a weird driver error. In the future we'll find a way to
            // tell what the actual error is
            if usize::try_from(bytes_returned).unwrap() < core::mem::size_of::<u32>() {
                return Err(std::io::Error::from(std::io::ErrorKind::InvalidData));
            }

            let win32_err =
                WIN32_ERROR::from_error(&err).expect("Converting error from DeviceIoControl");
            match win32_err {
                ERROR_MORE_DATA => Ok(bytes_returned.try_into().unwrap()),
                ERROR_INSUFFICIENT_BUFFER => {
                    Err(std::io::Error::from(std::io::ErrorKind::WriteZero))
                }
                _ => Err(std::io::Error::last_os_error()),
            }
        } else {
            self.end_of_req = true;
            Ok(bytes_returned.try_into().unwrap())
        }
    }
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

impl From<DeviceType> for u32 {
    fn from(val: DeviceType) -> Self {
        val.into_u32()
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

impl From<TransferMethod> for u32 {
    fn from(val: TransferMethod) -> Self {
        val.into_u32()
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

impl From<ControlCode> for u32 {
    fn from(val: ControlCode) -> Self {
        val.into_u32()
    }
}
