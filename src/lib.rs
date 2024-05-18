//! # usbip-core
//!
//! A userspace library for interacting with the vhci kernel drivers.
//!
//! This is a rust port of two major libraries, [usbip-win2][https://github.com/vadimgrn/usbip-win2]
//! (Windows) and [usbip-utils][https://github.com/torvalds/linux/tree/master/tools/usb/usbip] (Linux).
//! 
//! The goal of this library is to provide a platform-independent interface for sharing USB devices across
//! the local internet. Currently only client-mode is supported, but future work will focus on supporting
//! server-mode for at least Linux.

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;
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
    mod singleton;
    pub mod stacktools;
    pub mod iterators {
        use std::num::NonZeroU32;

        pub struct BitShiftLeft {
            mask: NonZeroU32,
            num: usize,
        }

        impl BitShiftLeft {
            pub const fn new(mask: NonZeroU32, num: usize) -> Self {
                Self { mask, num }
            }
        }

        impl Iterator for BitShiftLeft {
            type Item = usize;

            fn next(&mut self) -> Option<Self::Item> {
                let next = self.num;
                self.num = self.num.checked_shl(self.mask.get())?;
                Some(next)
            }
        }
    }
}
mod util;
pub mod net {
    //! Contains the implementation of the USB/IP [protocol]
    //! as defined by the linux kernel.
    //!
    //! [protocol]: https://www.kernel.org/doc/html/latest/usb/usbip_protocol.html
    use core::fmt;
    use std::borrow::Cow;

    use bincode::{
        config::{BigEndian, Configuration, Fixint},
        error::AllowedEnumVariants,
        impl_borrow_decode,
    };

    use crate::{
        containers::stacktools::StackStr,
        util::__private::Sealed,
        UsbDevice, BUS_ID_SIZE, USBIP_VERSION,
    };

    use bitflags::bitflags;

    bitflags! {
        /// The USB/IP protocol.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct Protocol: u16 {
            // Common header for all the kinds of PDUs.
            const OP_REQUEST = 0x80 << 8;
            const OP_REPLY = 0x00 << 8;

            // Import a remote USB device.
            const OP_IMPORT = 0x03;
            const OP_REQ_IMPORT = Self::OP_REQUEST.bits() | Self::OP_IMPORT.bits();
            const OP_REP_IMPORT = Self::OP_REPLY.bits() | Self::OP_IMPORT.bits();

            // Dummy code
            const OP_UNSPEC = 0x00;
            const _OP_REQ_UNSPEC = Self::OP_UNSPEC.bits();
            const _OP_REP_UNSPEC = Self::OP_UNSPEC.bits();

            // Retrieve the list of exported USB devices
            const OP_DEVLIST = 0x05;
            const OP_REQ_DEVLIST = Self::OP_REQUEST.bits() | Self::OP_DEVLIST.bits();
            const OP_REP_DEVLIST = Self::OP_REPLY.bits() | Self::OP_DEVLIST.bits();

            // Export a USB device to a remote host
            const OP_EXPORT = 0x06;
            const OP_REQ_EXPORT = Self::OP_REQUEST.bits() | Self::OP_EXPORT.bits();
            const OP_REP_EXPORT = Self::OP_REPLY.bits() | Self::OP_EXPORT.bits();
        }
    }

    impl bincode::Encode for Protocol {
        fn encode<E: bincode::enc::Encoder>(
            &self,
            encoder: &mut E,
        ) -> Result<(), bincode::error::EncodeError> {
            self.bits().encode(encoder)
        }
    }

    impl bincode::Decode for Protocol {
        fn decode<D: bincode::de::Decoder>(
            decoder: &mut D,
        ) -> Result<Self, bincode::error::DecodeError> {
            static PROTO_SIMPLE_FLAGS: &'static [u32] = &[
                Protocol::OP_REQUEST.bits() as u32,
                Protocol::OP_REPLY.bits() as u32,
                Protocol::OP_IMPORT.bits() as u32,
                Protocol::OP_REQ_IMPORT.bits() as u32,
                Protocol::OP_REP_IMPORT.bits() as u32,
                Protocol::OP_UNSPEC.bits() as u32,
                Protocol::_OP_REQ_UNSPEC.bits() as u32,
                Protocol::_OP_REP_UNSPEC.bits() as u32,
                Protocol::OP_DEVLIST.bits() as u32,
                Protocol::OP_REQ_DEVLIST.bits() as u32,
                Protocol::OP_REP_DEVLIST.bits() as u32,
                Protocol::OP_EXPORT.bits() as u32,
                Protocol::OP_REQ_EXPORT.bits() as u32,
                Protocol::OP_REP_EXPORT.bits() as u32,
            ];

            static BINCODE_PROTO_ALLOWED_FLAGS: AllowedEnumVariants =
                AllowedEnumVariants::Allowed(PROTO_SIMPLE_FLAGS);

            let code = u16::decode(decoder)?;

            Self::from_bits(code).ok_or(bincode::error::DecodeError::UnexpectedVariant {
                type_name: "Protocol",
                allowed: &BINCODE_PROTO_ALLOWED_FLAGS,
                found: code as u32,
            })
        }
    }

    impl_borrow_decode!(Protocol);

    /// The result of a USB/IP network request.
    /// Will encode/decode as a 4 byte value.
    #[derive(Debug, Clone, Copy, bincode::Encode, bincode::Decode, PartialEq, Eq)]
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

    /// Returns the [`bincode::Configuration`] used
    /// for network communication.
    ///
    /// The current config is no limit on transfers, big endian, and fixed int encoding.
    ///
    /// [`bincode::Configuration`]: bincode::config::Configuration
    pub const fn bincode_config() -> Configuration<BigEndian, Fixint> {
        bincode::config::standard()
            .with_no_limit()
            .with_big_endian()
            .with_fixed_int_encoding()
    }

    /// Convenience trait for encoding and 
    /// writing the encoded data into a buffer
    /// that implements the [`std::io::Write`]
    /// trait.
    pub trait Send: std::io::Write + Sealed {
        fn send<T: bincode::Encode>(&mut self, data: &T) -> Result<usize, Error>;
    }

    /// Convenience trait for reading data from
    /// a buffer that implements [`std::io::Read`]
    /// and decoding it into the type `T`.
    pub trait Recv: std::io::Read + Sealed {
        fn recv<T: bincode::Decode>(&mut self) -> Result<T, Error>;
    }

    impl From<bincode::error::DecodeError> for Error {
        fn from(value: bincode::error::DecodeError) -> Self {
            Self::De(value)
        }
    }

    impl From<bincode::error::EncodeError> for Error {
        fn from(value: bincode::error::EncodeError) -> Self {
            Self::Enc(value)
        }
    }

    /// Represents all the possible userspace errors
    /// that could occur with communicating between
    /// a host and client.
    #[derive(Debug)]
    pub enum Error {
        VersionMismatch(u16),
        BusIdMismatch(Cow<'static, str>),
        Enc(bincode::error::EncodeError),
        De(bincode::error::DecodeError),
    }

    impl core::fmt::Display for Error {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Error::VersionMismatch(bad_version) => write!(
                    f,
                    "Version mismatch! Them: {}, Us: {}",
                    bad_version, USBIP_VERSION
                ),
                Error::BusIdMismatch(bus_id) => write!(f, "Received different busid \"{bus_id}\""),
                Error::Enc(enc) => write!(f, "Encode error! {enc}"),
                Error::De(de) => write!(f, "Decode error! {de}"),
            }
        }
    }

    impl std::error::Error for Error {}

    impl From<Error> for crate::vhci::error2::Error {
        fn from(value: Error) -> Self {
            Self::Net(value)
        }
    }

    #[derive(Debug, Clone, Copy, bincode::Encode, bincode::Decode)]
    pub struct OpCommon {
        version: u16,
        code: Protocol,
        status: Status,
    }

    impl OpCommon {
        /// Creates an [`OpCommon`] with
        /// `code` as the request.
        /// 
        /// Depending on the [`Protocol`] used,
        /// the remote device will expect
        /// more data to be sent.
        #[inline]
        pub const fn request(code: Protocol) -> Self {
            Self {
                version: super::USBIP_VERSION as u16,
                code,
                status: Status::Success,
            }
        }

        /// Consumes an [`OpCommon`] and returns another
        /// one with the `status` of whatever request
        /// the remote device made.
        #[inline]
        pub const fn reply(self, status: Status) -> Self {
            Self {
                status,
                ..self
            }
        }

        /// Performs basic validation on the [`OpCommon`] object.
        ///
        /// On success, returns the Status code of the [`OpCommon`].
        ///
        /// # Error
        ///
        /// This function will return an error if:
        /// - the version number differs from the version number
        ///   used in this userspace library
        /// - the code inside the [`OpCommon`] object does not match
        ///   `expected`
        pub fn validate(&self, expected: Protocol) -> Result<Status, Error> {
            if self.version as usize != USBIP_VERSION {
                Err(Error::VersionMismatch(self.version))
            } else if expected != Protocol::OP_UNSPEC && expected != self.code {
                Ok(Status::Unexpected)
            } else {
                Ok(self.status)
            }
        }
    }

    #[derive(Debug)]
    pub struct OpImportRequest<'a> {
        bus_id: &'a str,
    }

    impl bincode::Encode for OpImportRequest<'_> {
        fn encode<E: bincode::enc::Encoder>(
            &self,
            encoder: &mut E,
        ) -> Result<(), bincode::error::EncodeError> {
            let s = StackStr::<BUS_ID_SIZE>::try_from(self.bus_id)
                .map_err(|_| bincode::error::EncodeError::UnexpectedEnd)?;

            s.encode(encoder)
        }
    }

    impl<'a> OpImportRequest<'a> {
        /// Constructs a new [`OpImportRequest`]
        /// out of a [`str`] slice.
        #[inline(always)]
        pub const fn new(bus_id: &'a str) -> Self {
            Self { bus_id }
        }
    }

    /// The owned version of a [`OpImportRequest`].
    /// Used for decoding from a buffer, since we
    /// can't guarantee that the data in this struct
    /// will last long enough for usage.
    #[derive(Debug, bincode::Decode)]
    pub struct OwnedOpImportRequest {
        bus_id: StackStr<BUS_ID_SIZE>,
    }

    impl OwnedOpImportRequest {
        #[inline(always)]
        pub const fn into_inner(self) -> StackStr<BUS_ID_SIZE> {
            self.bus_id
        }
    }

    #[derive(Debug, bincode::Encode, bincode::Decode)]
    pub struct OpImportReply {
        usb_dev: UsbDevice,
    }

    impl OpImportReply {
        #[inline(always)]
        pub const fn new(usb_dev: UsbDevice) -> Self {
            Self { usb_dev }
        }

        #[inline(always)]
        pub const fn into_inner(self) -> UsbDevice {
            self.usb_dev
        }
    }

    #[derive(Debug, bincode::Encode, bincode::Decode)]
    pub struct OpDevlistReply {
        num_devices: u32,
    }

    impl OpDevlistReply {
        #[inline(always)]
        pub const fn new(num_devices: u32) -> Self {
            Self { num_devices }
        }

        #[inline(always)]
        pub const fn num_devices(&self) -> u32 {
            self.num_devices
        }
    }
}

use core::fmt;
use std::{num::ParseIntError, path::Path, str::FromStr};

use bincode::de::read::Reader;
use containers::stacktools::StackStr;

pub use platform::USB_IDS;

pub const USBIP_VERSION: usize = 0x111;
pub const DEV_PATH_MAX: usize = 256;
pub const BUS_ID_SIZE: usize = 32;

#[derive(Debug, bincode::Encode, bincode::Decode)]
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
        &*self.busid
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

/// The state of a [`vhci`] device port.
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
        let status = match s.parse::<u8>().map_err(Self::Err::Parse)? {
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

#[derive(Debug, Clone, Copy)]
pub struct UsbInterface {
    b_interface_class: u8,
    b_interface_subclass: u8,
    b_interface_protocol: u8,
}

impl bincode::Encode for UsbInterface {
    fn encode<E: bincode::enc::Encoder>(
        &self,
        encoder: &mut E,
    ) -> Result<(), bincode::error::EncodeError> {
        bincode::Encode::encode(&self.b_interface_class, encoder)?;
        bincode::Encode::encode(&self.b_interface_subclass, encoder)?;
        bincode::Encode::encode(&self.b_interface_protocol, encoder)?;
        bincode::Encode::encode(&0u8, encoder)?;
        Ok(())
    }
}

impl bincode::Decode for UsbInterface {
    fn decode<D: bincode::de::Decoder>(
        decoder: &mut D,
    ) -> Result<Self, bincode::error::DecodeError> {
        let b_interface_class = u8::decode(decoder)?;
        let b_interface_subclass = u8::decode(decoder)?;
        let b_interface_protocol = u8::decode(decoder)?;
        decoder.claim_bytes_read(core::mem::size_of::<u8>())?;
        decoder.reader().consume(core::mem::size_of::<u8>());

        Ok(UsbInterface {
            b_interface_class,
            b_interface_subclass,
            b_interface_protocol,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, bincode::Decode, bincode::Encode)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_speed_sizeof_i32() {
        assert_eq!(
            std::mem::size_of::<DeviceSpeed>(),
            std::mem::size_of::<i32>()
        );
    }
}
