use std::{borrow::Cow, net::SocketAddr, str::FromStr};

use bincode::{
    de::{read::Reader, Decoder},
    impl_borrow_decode, BorrowDecode, Encode,
};
use num_derive::FromPrimitive;
use num_traits::FromPrimitive;
use win_deviceioctl::{ControlCode, DeviceType, EncResult, RequiredAccess, TransferMethod};

use crate::{
    containers::stacktools::{StackStr, Str},
    util::EncodedSize,
    BusId, DeviceSpeed, BUS_ID_SIZE,
};

/// A non-exhaustive list of the error codes
/// that can be returned by the vhci driver.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, FromPrimitive)]
pub enum DriverError {
    InvalidAbi = 0xE1000008,
    IncompatibleProtocolVersion = 0xE1000005,
    //DevNotConnected = 0x8007048F,
    DevNotConnected = -2147023729,
    FileNotFound = -2147024894,
}

impl TryFrom<i32> for DriverError {
    type Error = ();
    fn try_from(value: i32) -> Result<Self, Self::Error> {
        Self::from_i32(value).ok_or(())
    }
}

impl std::fmt::Display for DriverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!()
    }
}

impl std::error::Error for DriverError {}

#[derive(Debug, Clone, Copy)]
pub enum Function {
    PluginHardware = 0x800,
    PlugoutHardware,
    GetImportedDevices,
    SetPersistent,
    GetPersistent,
}

impl Function {
    const fn make_ctrl_code(self) -> ControlCode {
        ControlCode(
            DeviceType::Unknown,
            RequiredAccess::READ_WRITE_DATA,
            self as u32,
            TransferMethod::Buffered,
        )
    }
}

pub struct DeviceLocation<'a> {
    pub host: SocketAddr,
    pub busid: BusId<'a>,
}

impl<'a> DeviceLocation<'a> {
    pub const fn new(host: SocketAddr, busid: &'a str) -> Option<Self> {
        match Str::new(busid) {
            Some(busid) => Some(Self {
                host,
                busid: BusId::new(Cow::Borrowed(busid)),
            }),
            None => None,
        }
    }
}

impl bincode::Encode for DeviceLocation<'_> {
    fn encode<E: bincode::enc::Encoder>(
        &self,
        encoder: &mut E,
    ) -> Result<(), bincode::error::EncodeError> {
        use bincode::enc::write::Writer;
        0i32.encode(encoder)?;
        self.busid.encode(encoder)?;
        StackStr::<32>::try_from(format_args!("{}", self.host.port()))
            .unwrap()
            .encode(encoder)?;
        StackStr::<1025>::try_from(format_args!("{}", self.host.ip()))
            .unwrap()
            .encode(encoder)?;
        encoder.writer().write(&[0, 0, 0])?;

        Ok(())
    }
}

impl FromStr for DeviceLocation<'static> {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use std::net::ToSocketAddrs;
        let mut split = s.split(',');
        let hostname = split.next().ok_or(())?;
        let service = split.next().ok_or(())?;
        let busid = split.next().ok_or(())?;

        let host = (hostname, service.parse().map_err(|_| ())?)
            .to_socket_addrs()
            .map_err(|_| ())?
            .next()
            .unwrap();

        Ok(Self {
            host,
            busid: BusId::new(Cow::Owned(StackStr::try_from(busid).unwrap())),
        })
    }
}

unsafe impl EncodedSize for DeviceLocation<'_> {
    const ENCODED_SIZE_OF: usize = 1096;
}

/// A helper struct for encoding/decoding
/// a port number.
struct Port(u16);

impl bincode::Decode for Port {
    fn decode<D: bincode::de::Decoder>(
        decoder: &mut D,
    ) -> Result<Self, bincode::error::DecodeError> {
        let port = i32::decode(decoder)?;
        Ok(Port(port as u16))
    }
}

impl bincode::Encode for Port {
    fn encode<E: bincode::enc::Encoder>(&self, encoder: &mut E) -> EncResult {
        let port = self.0 as i32;
        port.encode(encoder)
    }
}

impl_borrow_decode!(Port);

unsafe impl EncodedSize for Port {
    const ENCODED_SIZE_OF: usize = core::mem::size_of::<i32>();
}

pub struct Attach<'a> {
    location: DeviceLocation<'a>,
}

impl<'a> Attach<'a> {
    pub const fn new(location: DeviceLocation<'a>) -> Self {
        Self { location }
    }
}

impl win_deviceioctl::Send for Attach<'_> {
    fn send<E: bincode::enc::Encoder>(&self, encoder: &mut E) -> win_deviceioctl::EncResult {
        let size_of = (DeviceLocation::ENCODED_SIZE_OF + core::mem::size_of::<u32>()) as u32;
        size_of.encode(encoder)?;
        self.location.encode(encoder)
    }
}

impl win_deviceioctl::Recv for Attach<'_> {
    type Output = u16;

    fn buf_starting_capacity(&self) -> Option<usize> {
        Some(Port::ENCODED_SIZE_OF + core::mem::size_of::<u32>())
    }

    fn recv(bytes: &[u8]) -> win_deviceioctl::DecResult<Self::Output> {
        if bytes.len() < core::mem::size_of::<u32>() {
            return Err(bincode::error::DecodeError::UnexpectedEnd {
                additional: Port::ENCODED_SIZE_OF,
            });
        }
        let port = bincode::decode_from_slice::<Port, _>(
            &bytes[core::mem::size_of::<u32>()..],
            win_deviceioctl::bincode_config(),
        )?;
        Ok(port.0 .0)
    }
}

impl win_deviceioctl::CtrlCode for Attach<'_> {
    const CODE: win_deviceioctl::ControlCode = Function::PluginHardware.make_ctrl_code();
}

pub struct Detach {
    port: Port,
}

impl Detach {
    pub const fn new(port: u16) -> Self {
        Self { port: Port(port) }
    }
}

impl win_deviceioctl::Send for Detach {
    fn send<E: bincode::enc::Encoder>(&self, encoder: &mut E) -> EncResult {
        let size = (Port::ENCODED_SIZE_OF + core::mem::size_of::<u32>()) as u32;
        size.encode(encoder)?;
        self.port.encode(encoder)
    }
}

impl win_deviceioctl::CtrlCode for Detach {
    const CODE: ControlCode = Function::PlugoutHardware.make_ctrl_code();
}

pub struct PortRecord<'a> {
    pub port: i32,
    pub busid: &'a Str<BUS_ID_SIZE>,
    pub service: &'a Str<32>,
    pub host: &'a Str<1025>,
}

impl<'de> bincode::BorrowDecode<'de> for PortRecord<'de> {
    fn borrow_decode<D: bincode::de::BorrowDecoder<'de>>(
        decoder: &mut D,
    ) -> Result<Self, bincode::error::DecodeError> {
        let port = i32::borrow_decode(decoder)?;
        let busid: &Str<BUS_ID_SIZE> = bincode::BorrowDecode::borrow_decode(decoder)?;
        let service: &Str<32> = bincode::BorrowDecode::borrow_decode(decoder)?;
        let host: &Str<1025> = bincode::BorrowDecode::borrow_decode(decoder)?;
        // Account for padding from irregular array size
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

pub struct ImportedDevice<'a> {
    pub record: PortRecord<'a>,
    pub devid: u32,
    pub speed: DeviceSpeed,
    pub vendor: u16,
    pub product: u16,
}

impl<'de> bincode::BorrowDecode<'de> for ImportedDevice<'de> {
    fn borrow_decode<D: bincode::de::BorrowDecoder<'de>>(
        decoder: &mut D,
    ) -> Result<Self, bincode::error::DecodeError> {
        let record = PortRecord::borrow_decode(decoder)?;
        let devid = u32::borrow_decode(decoder)?;
        let speed = DeviceSpeed::borrow_decode(decoder)?;
        let vendor = u16::borrow_decode(decoder)?;
        let product = u16::borrow_decode(decoder)?;

        Ok(Self {
            record,
            devid,
            speed,
            vendor,
            product,
        })
    }
}

unsafe impl EncodedSize for ImportedDevice<'_> {
    const ENCODED_SIZE_OF: usize = 1108;
}

pub struct GetImportedDevices;

impl win_deviceioctl::Send for GetImportedDevices {
    fn send<E: bincode::enc::Encoder>(&self, encoder: &mut E) -> EncResult {
        let size_of = (ImportedDevice::ENCODED_SIZE_OF + core::mem::size_of::<u32>()) as u32;
        size_of.encode(encoder)
    }
}

impl win_deviceioctl::Recv for GetImportedDevices {
    type Output = Vec<super::WindowsImportedDevice>;

    fn buf_starting_capacity(&self) -> Option<usize> {
        Some(ImportedDevice::ENCODED_SIZE_OF + core::mem::size_of::<u32>())
    }

    fn recv(bytes: &[u8]) -> win_deviceioctl::DecResult<Self::Output> {
        let buf_len = bytes.len();
        let num_items = (buf_len - core::mem::size_of::<u32>()) / ImportedDevice::ENCODED_SIZE_OF;
        let mut buf = Vec::with_capacity(num_items);

        let reader = bincode::de::read::SliceReader::new(&bytes[core::mem::size_of::<u32>()..]);
        let mut decoder = bincode::de::DecoderImpl::new(reader, win_deviceioctl::bincode_config());

        decoder.claim_container_read::<[u8; ImportedDevice::ENCODED_SIZE_OF]>(num_items)?;

        for _ in 0..num_items {
            decoder.unclaim_bytes_read(ImportedDevice::ENCODED_SIZE_OF);

            let idev = ImportedDevice::borrow_decode(&mut decoder)?;
            buf.push(idev);
        }

        Ok(buf.into_iter().map(|idev| idev.into()).collect::<Vec<_>>())
    }
}

impl win_deviceioctl::CtrlCode for GetImportedDevices {
    const CODE: ControlCode = Function::GetImportedDevices.make_ctrl_code();
}

pub struct GetPersistentDevices;

impl win_deviceioctl::Recv for GetPersistentDevices {
    type Output = Vec<DeviceLocation<'static>>;

    fn buf_starting_capacity(&self) -> Option<usize> {
        None
    }

    fn recv(bytes: &[u8]) -> win_deviceioctl::DecResult<Self::Output> {
        // Now we're going to be silly.
        // This will panic if not properly aligned, which will
        // definitely mean that I did something wrong.
        let phat_buf = crate::windows::util::cast_u8_to_u16_slice(bytes);

        // If this fails this might also be my fault
        let entries = String::from_utf16(phat_buf).map_err(|_| {
            bincode::error::DecodeError::Other("Failed to decode UTF-16 slice as a String")
        })?;

        Ok(entries
            .split_terminator('\0')
            .filter_map(|s| s.parse().ok())
            .collect())
    }
}

impl win_deviceioctl::CtrlCode for GetPersistentDevices {
    const CODE: ControlCode = Function::GetPersistent.make_ctrl_code();
}
