use std::{
    borrow::{Borrow, Cow},
    num::{NonZeroUsize, ParseIntError}, str::FromStr,
};

use crate::util::{beef::Beef, buffer};

#[derive(Debug)]
pub enum Error {
    NoParent,
    Parse(ParseAttributeError),
    TryFromDev(TryFromDeviceError),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NoParent => write!(f, "udev has no parent device"),
            Error::Parse(_) => todo!(),
            Error::TryFromDev(_) => todo!(),
        }
    }
}

impl std::error::Error for Error {}

impl From<Error> for crate::vhci::Error {
    fn from(value: Error) -> Self {
        crate::vhci::Error::Udev(value)
    }
}

#[derive(Debug)]
pub enum TryFromDeviceError {
    IO(std::io::Error),
    Parse(ParseAttributeError),
}

impl std::fmt::Display for TryFromDeviceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!()
    }
}

impl std::error::Error for TryFromDeviceError {}

impl From<std::io::Error> for TryFromDeviceError {
    fn from(value: std::io::Error) -> Self {
        Self::IO(value)
    }
}

impl From<ParseAttributeError> for TryFromDeviceError {
    fn from(value: ParseAttributeError) -> Self {
        Self::Parse(value)
    }
}

impl From<TryFromDeviceError> for Error {
    fn from(value: TryFromDeviceError) -> Self {
        Self::TryFromDev(value)
    }
}

pub trait UdevHelper: crate::util::__private::Sealed + Borrow<udev::Device> {
    fn parse_sysattr<'a, 'b, T>(&'a self, attr: Beef<'b, str>) -> Result<T, ParseAttributeError>
    where
        T: FromStr,
        <T as FromStr>::Err: Into<ParseAttributeError>,
    {
        let udev: &udev::Device = self.borrow();
        let data = if let Some(value) = udev.attribute_value(&*attr) {
            value
        } else {
            return Err(ParseAttributeError::NoAttribute(Cow::from(attr)));
        };
        let data = data.to_str().ok_or_else(|| ParseAttributeError::NotUtf8)?;
        data.parse().map_err(|e: T::Err| e.into())
    }

    fn sysattr<'a, 'b>(&'a self, attr: Beef<'b, str>) -> Result<&'a str, ParseAttributeError> {
        let udev: &udev::Device = self.borrow();
        udev.attribute_value(&*attr)
            .ok_or_else(|| ParseAttributeError::NoAttribute(Cow::from(attr)))?
            .to_str()
            .ok_or_else(|| ParseAttributeError::NotUtf8)
    }
}

impl crate::util::__private::Sealed for udev::Device {}
impl UdevHelper for udev::Device {}

#[derive(Debug)]
pub enum ParseAttributeError {
    NoAttribute(Cow<'static, str>),
    Int(ParseIntError),
    Dyn(Box<dyn std::error::Error>),
    NotUtf8,
    Buffer(buffer::FormatError),
}

impl std::fmt::Display for ParseAttributeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!()
    }
}

impl std::error::Error for ParseAttributeError {}

impl From<ParseAttributeError> for crate::vhci::Error {
    fn from(value: ParseAttributeError) -> Self {
        Self::Udev(value.into())
    }
}

impl From<ParseAttributeError> for Error {
    fn from(value: ParseAttributeError) -> Self {
        Self::Parse(value)
    }
}

impl From<Box<dyn std::error::Error>> for ParseAttributeError {
    fn from(value: Box<dyn std::error::Error>) -> Self {
        ParseAttributeError::Dyn(value)
    }
}

impl From<ParseIntError> for ParseAttributeError {
    fn from(value: ParseIntError) -> Self {
        Self::Int(value)
    }
}

impl From<buffer::FormatError> for ParseAttributeError {
    fn from(value: buffer::FormatError) -> Self {
        Self::Buffer(value)
    }
}
