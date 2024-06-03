//! # Goals for this module
//!
//! I want to define some sort of interface that specifies
//! an IoCtl function, control code, and a data layout suitable
//! for both the input/output.
//!
//! This data layout does not need to be in its final form,
//! but should avoid taking advantage of that fact through
//! "hacky" formats and tricks.
//!
//! # Motivation
//!
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
//!
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
use std::net::SocketAddr;
use std::num::NonZeroU32;
use std::os::windows::io::{AsRawHandle, BorrowedHandle};
use std::str::FromStr;

use bincode::de::read::{BorrowReader, Reader};
use bincode::de::{BorrowDecoder, Decoder};
use bincode::{impl_borrow_decode, Decode, Encode};
use bitflags::bitflags;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};
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

use crate::windows::util::consts::{NI_MAXHOST, NI_MAXSERV};

type BincodeConfig = bincode::config::Configuration<
    bincode::config::LittleEndian,
    bincode::config::Fixint,
    bincode::config::NoLimit,
>;

// New idea: Create a writer that rips off the bincode writers, then use that as the concrete type.
type IoctlEncoder = bincode::enc::EncoderImpl<ConcreteWriter, BincodeConfig>;
type IoctlDecoder<'a> = bincode::de::DecoderImpl<SliceReader<'a>, BincodeConfig>;

type EncResult = Result<(), bincode::error::EncodeError>;
type DecResult<T> = Result<T, bincode::error::DecodeError>;

#[derive(Default)]
pub struct VecWriter {
    inner: Vec<u8>,
}

impl VecWriter {
    /// Create a new vec writer with the given capacity
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            inner: Vec::with_capacity(cap),
        }
    }
}

impl bincode::enc::write::Writer for VecWriter {
    #[inline(always)]
    fn write(&mut self, bytes: &[u8]) -> EncResult {
        self.inner.extend_from_slice(bytes);
        Ok(())
    }
}

pub struct ConcreteWriter {
    inner: AlmostGenericWriter,
}

impl ConcreteWriter {
    const fn new(writer: AlmostGenericWriter) -> Self {
        Self { inner: writer }
    }
}

enum AlmostGenericWriter {
    Size(bincode::enc::write::SizeWriter),
    Vec(VecWriter),
}

impl ConcreteWriter {
    fn bytes_written(&self) -> usize {
        match &self.inner {
            AlmostGenericWriter::Size(w) => w.bytes_written,
            AlmostGenericWriter::Vec(w) => w.inner.len(),
        }
    }

    fn into_vec(self) -> Option<Vec<u8>> {
        match self.inner {
            AlmostGenericWriter::Vec(w) => Some(w.inner),
            _ => panic!("not a VecWriter!"),
        }
    }
}

impl bincode::enc::write::Writer for ConcreteWriter {
    fn write(&mut self, bytes: &[u8]) -> EncResult {
        match &mut self.inner {
            AlmostGenericWriter::Size(w) => w.write(bytes),
            AlmostGenericWriter::Vec(w) => w.write(bytes),
        }
    }
}

pub struct SliceReader<'a> {
    reader: bincode::de::read::SliceReader<'a>,
    len: usize,
}

impl<'a> SliceReader<'a> {
    fn new(slice: &'a [u8]) -> Self {
        let len = slice.len();
        Self {
            reader: bincode::de::read::SliceReader::new(slice),
            len,
        }
    }

    const fn len(&self) -> usize {
        self.len
    }
}

impl<'a> bincode::de::read::Reader for SliceReader<'a> {
    fn read(&mut self, bytes: &mut [u8]) -> DecResult<()> {
        self.reader.read(bytes)
    }

    fn peek_read(&mut self, n: usize) -> Option<&[u8]> {
        self.reader.peek_read(n)
    }

    fn consume(&mut self, n: usize) {
        self.reader.consume(n)
    }
}

impl<'a> bincode::de::read::BorrowReader<'a> for SliceReader<'a> {
    fn take_bytes(&mut self, length: usize) -> DecResult<&'a [u8]> {
        self.reader.take_bytes(length)
    }
}

const fn bincode_config() -> BincodeConfig {
    bincode::config::standard()
        .with_little_endian()
        .with_fixed_int_encoding()
        .with_no_limit()
}

/// A non-exhaustive list of the error codes
/// that can be returned by the vhci driver.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, FromPrimitive)]
enum DriverError {
    InvalidAbi = 0xE1000008,
    IncompatibleProtocolVersion = 0xE1000005,
    DevNotConnected = 0x8007048F,
}

pub enum OutputFn<T, U> {
    Recv {
        recv: fn(&mut IoctlDecoder) -> DecResult<T>,
        regrow_strategy: fn() -> U,
    },
    Create(fn() -> T),
}

/// The main trait for defining an ioctl function
/// for the vhci driver.
///
/// Consumers of this trait will define:
/// - the function code
/// - whether [`relay`] will send data to [`DeviceIoControl`]
/// - whether [`relay`] will receive data from [`DeviceIoControl`],
///   and if so,
///   - how to regrow the buffer to receive the data (using [`IoControl::RegrowIter`])
///   - if not receiving data, then the consumer must specify
///     how to produce [`IoControl::Output`]
///
/// # Why aren't [`IoControl::SEND`] and [`IoControl::RECV`] just normal trait functions?
pub trait IoControl
where
    Self::RegrowIter: Iterator<Item = usize>,
{
    type RegrowIter;
    type Output;
    const FUNCTION: Function;
    const SEND: Option<fn(&Self, &mut IoctlEncoder) -> EncResult>;
    const RECV: OutputFn<Self::Output, Self::RegrowIter>;

    #[inline(always)]
    fn ctrl_code() -> ControlCode {
        ControlCode(
            DeviceType::Unknown,
            RequiredAccess::READ_WRITE_DATA,
            <Self as IoControl>::FUNCTION.to_u32().unwrap(),
            TransferMethod::Buffered,
        )
    }
}

#[derive(Debug, Clone, Copy, ToPrimitive)]
pub enum Function {
    PluginHardware = 0x800,
    PlugoutHardware,
    GetImportedDevices,
    SetPersistent,
    GetPersistent,
}

pub struct OnceSize {
    byte_size: usize,
    called: u32,
}

impl Iterator for OnceSize {
    type Item = usize;
    fn next(&mut self) -> Option<Self::Item> {
        if self.called == 2 {
            panic!("function called more than twice!")
        }
        self.called += 1;
        Some(self.byte_size)
    }
}

pub struct NoIter;
impl Iterator for NoIter {
    type Item = usize;
    fn next(&mut self) -> Option<Self::Item> {
        None
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
    ) -> EncResult {
        PortRecord {
            port: 0,
            busid: StackStr::try_from(self.bus_id)
                .map_err(|_| bincode::error::EncodeError::UnexpectedEnd)?,
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

/// Used to request for the vhci driver
/// to detach a device.
pub struct Detach {
    port: Port,
}

impl Detach {
    pub const fn new(port: u16) -> Self {
        Self { port: Port(port) }
    }
}

impl IoControl for Detach {
    type Output = ();
    type RegrowIter = std::ops::Range<usize>;
    const FUNCTION: Function = Function::PlugoutHardware;
    const SEND: Option<fn(&Self, &mut IoctlEncoder) -> EncResult> =
        Some(|ioctl, encoder| {
            let size = (Port::ENCODED_SIZE_OF + core::mem::size_of::<u32>()) as u32;
            size.encode(encoder)?;
            ioctl.port.encode(encoder)
        });
    const RECV: OutputFn<Self::Output, Self::RegrowIter> = OutputFn::Create(Default::default);
}

/// Used to request for the vhci driver
/// to connect to a host and attach one
/// of its usb devices.
pub struct Attach<'a> {
    location: DeviceLocation<'a>,
}

impl<'a> Attach<'a> {
    pub const fn new(location: DeviceLocation<'a>) -> Self {
        Self { location }
    }
}

impl IoControl for Attach<'_> {
    type Output = u16;
    type RegrowIter = OnceSize;
    const FUNCTION: Function = Function::PluginHardware;
    const SEND: Option<fn(&Self, &mut IoctlEncoder) -> EncResult> =
        Some(|ioctl: &Self, encoder| {
            let size_of = (DeviceLocation::ENCODED_SIZE_OF + core::mem::size_of::<u32>()) as u32;
            size_of.encode(encoder)?;
            ioctl.location.encode(encoder)?;
            Ok(())
        });
    const RECV: OutputFn<Self::Output, Self::RegrowIter> = OutputFn::Recv {
        recv: |decoder| {
            decoder.claim_bytes_read(core::mem::size_of::<u32>())?;
            decoder.reader().consume(core::mem::size_of::<u32>());
            let port = Port::decode(decoder)?;
            Ok(port.get())
        },
        regrow_strategy: || OnceSize {
            byte_size: core::mem::size_of::<u32>() + core::mem::size_of::<i32>(),
            called: 0,
        },
    };
}

/// A helper struct for encoding/decoding
/// a port number.
pub struct Port(u16);

impl Port {
    const fn get(&self) -> u16 {
        self.0
    }
}

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
    ) -> EncResult {
        let port = self.0 as i32;
        port.encode(encoder)
    }
}

impl_borrow_decode!(Port);

unsafe impl EncodedSize for Port {
    const ENCODED_SIZE_OF: usize = core::mem::size_of::<i32>();
}

pub struct PortRecord {
    pub port: i32,
    pub busid: StackStr<BUS_ID_SIZE>,
    pub service: StackStr<32>,
    pub host: StackStr<1025>,
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
    ) -> EncResult {
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

impl IoControl for GetImportedDevices {
    type Output = Vec<ImportedDevice>;
    type RegrowIter =
        std::iter::Map<crate::containers::iterators::BitShiftLeft, fn(usize) -> usize>;
    const FUNCTION: Function = Function::GetImportedDevices;
    const SEND: Option<fn(&Self, &mut IoctlEncoder) -> EncResult> =
        Some(|_, encoder| {
            SizeOf((ImportedDevice::ENCODED_SIZE_OF + core::mem::size_of::<u32>()) as u32)
                .encode(encoder)
        });
    const RECV: OutputFn<Self::Output, Self::RegrowIter> = OutputFn::Recv {
        recv: |decoder| {
            decoder.claim_bytes_read(core::mem::size_of::<u32>())?;
            decoder.reader().consume(core::mem::size_of::<u32>());

            let buf_len = decoder.borrow_reader().len();
            let len = (buf_len - core::mem::size_of::<u32>()) / ImportedDevice::ENCODED_SIZE_OF;
            let mut buf = Vec::with_capacity(len);

            decoder.claim_container_read::<[u8; ImportedDevice::ENCODED_SIZE_OF]>(len)?;

            for _ in 0..len {
                decoder.unclaim_bytes_read(ImportedDevice::ENCODED_SIZE_OF);

                let idev = ImportedDevice::decode(decoder)?;
                buf.push(idev);
            }

            Ok(buf)
        },
        regrow_strategy: || {
            BitShiftLeft::new(NonZeroU32::new(1).unwrap(), ImportedDevice::ENCODED_SIZE_OF)
                .map(|x| x + core::mem::size_of::<u32>())
        },
    };
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
    ) -> EncResult {
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

impl_borrow_decode!(ImportedDevice);

pub struct OwnedDeviceLocation {
    pub host: SocketAddr,
    pub bus_id: StackStr<BUS_ID_SIZE>,
}

impl FromStr for OwnedDeviceLocation {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        todo!()
    }
}

#[derive(bincode::Decode, bincode::Encode)]
struct WideChar(u16);

unsafe impl EncodedSize for WideChar {
    const ENCODED_SIZE_OF: usize = core::mem::size_of::<u16>();
}

pub struct GetPersistentDevices;

impl IoControl for GetPersistentDevices {
    type Output = Vec<OwnedDeviceLocation>;
    type RegrowIter = BitShiftLeft;
    const FUNCTION: Function = Function::GetPersistent;
    const SEND: Option<fn(&Self, &mut IoctlEncoder) -> EncResult> =
        None;
    const RECV: OutputFn<Self::Output, Self::RegrowIter> = OutputFn::Recv {
        recv: |decoder| {
            let len = decoder.borrow_reader().len();
            let buf = decoder.borrow_reader().take_bytes(len)?;

            // Now we're going to be silly.
            // This will panic if not properly aligned, which will
            // definitely mean that I did something wrong.
            let phat_buf = crate::windows::util::cast_u8_to_u16_slice(buf);

            // If this fails this might also be my fault, not sure
            Ok(String::from_utf16(phat_buf)
                .map_err(|_| {
                    bincode::error::DecodeError::Other("Failed to decode UTF-16 slice as String")
                })?
                .split_terminator('\0')
                .filter_map(|s| s.parse::<OwnedDeviceLocation>().ok())
                .collect::<Self::Output>())
        },
        regrow_strategy: || {
            crate::containers::iterators::BitShiftLeft::new(NonZeroU32::new(1).unwrap(), 32)
        },
    };
}

#[derive(Debug)]
pub enum DoorError {
    Send(bincode::error::EncodeError),
    Recv(bincode::error::DecodeError),
    Io(std::io::Error),
}

impl fmt::Display for DoorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DoorError::Send(s) => s.fmt(f),
            DoorError::Recv(r) => r.fmt(f),
            DoorError::Io(i) => i.fmt(f),
        }
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

fn encode_to_vec<I: IoControl>(
    ioctl: &I,
    config: BincodeConfig,
) -> Result<Option<Vec<u8>>, bincode::error::EncodeError> {
    I::SEND
        .map(|send| {
            let size = {
                let writer = ConcreteWriter::new(AlmostGenericWriter::Size(bincode::enc::write::SizeWriter::default()));
                let mut size_writer = bincode::enc::EncoderImpl::<_, _>::new(writer, config);
                send(ioctl, &mut size_writer)?;
                size_writer.into_writer().bytes_written()
            };
            let writer = ConcreteWriter::new(AlmostGenericWriter::Vec(VecWriter::with_capacity(size)));
            let mut encoder = bincode::enc::EncoderImpl::<_, _>::new(writer, config);
            send(ioctl, &mut encoder)?;
            Ok(encoder.into_writer().into_vec().unwrap())
        })
        .transpose()
}

pub fn relay<I: IoControl>(handle: BorrowedHandle, ioctl: I) -> Result<I::Output, DoorError> {
    let config = bincode_config();
    let code = I::ctrl_code().into_u32();
    let mut door = Door::new(handle, code);

    let input = encode_to_vec(&ioctl, config)?;
    let input_ref = input.as_ref().map(|buf| buf.as_slice());

    match I::RECV {
        OutputFn::Recv {
            recv,
            regrow_strategy,
        } => {
            let mut output = Vec::<u8>::new();
            let mut start = 0;
            for size in regrow_strategy() {
                output.resize(size, 0);

                match door.read_write(input_ref, Some(&mut output[start..])) {
                    Ok(0) => {
                        // Door's read_write implementation requires that we
                        // call until we get Ok(0), which is at least two
                        // times due to Door setting it's completion flag after
                        // a call to DeviceIoControl.
                        //
                        // Before we leave this loop, we have to first make
                        // a trip to Ok(bytes_read) and correct the value of
                        // start no matter what. Therefore, this operation
                        // here will give us the correct length.
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
            let reader = SliceReader::new(&output);
            let mut decoder = bincode::de::DecoderImpl::new(reader, config);
            Ok(recv(&mut decoder)?)
        }
        OutputFn::Create(create) => {
            door.read_write(input_ref, None)?;
            Ok(create())
        }
    }
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

    /// Performs a call to [`DeviceIoControl`], reading from `input` and writing
    /// to `output` and using the stored handle and control code as the request.
    ///
    /// Returns the number of bytes written to `output`. If `Ok(0)` is returned,
    /// then the function is done writing data for the specific request.
    /// Users are expected to perform repeated calls to [`Door::read_write`]
    /// until receiving 0 bytes, using the same buffer for input. The output
    /// buffer should start right after where this function stopped writing to.
    fn read_write(
        &mut self,
        input: Option<&[u8]>,
        output: Option<&mut [u8]>,
    ) -> std::io::Result<usize> {
        if self.end_of_req {
            return Ok(0);
        }

        let code = self.code;
        let handle = HANDLE(self.handle.as_raw_handle() as isize);
        let input_len = input
            .as_ref()
            .map(|buf| buf.len() as u32)
            .unwrap_or_default();
        let output_len = output
            .as_ref()
            .map(|buf| buf.len() as u32)
            .unwrap_or_default();
        let mut bytes_returned: u32 = 0;

        // SAFETY: Both `input` and `output` are valid slices.
        let result = unsafe {
            DeviceIoControl(
                handle,
                code,
                input.map(|buf| buf.as_ptr().cast()),
                input_len,
                output.map(|buf| buf.as_mut_ptr().cast()),
                output_len,
                Some(core::ptr::addr_of_mut!(bytes_returned)),
                None,
            )
        };

        if let Err(err) = result {
            if usize::try_from(bytes_returned).unwrap() < core::mem::size_of::<u32>() {
                let driver_err = match DriverError::from_u32(err.code().0 as u32) {
                    Some(DriverError::InvalidAbi) => std::io::ErrorKind::InvalidData.into(),
                    Some(DriverError::IncompatibleProtocolVersion) => {
                        std::io::ErrorKind::InvalidData.into()
                    }
                    Some(DriverError::DevNotConnected) => std::io::ErrorKind::NotConnected.into(),
                    None => std::io::Error::other(err.message()),
                };
                return Err(driver_err);
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
