use std::borrow::Cow;

#[derive(Debug)]
pub enum Error {
    NoAttribute(Cow<'static, str>),
    NoParent,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NoAttribute(s) => write!(f, "missing attribute \"{s}\" for udev"),
            Error::NoParent => write!(f, "udev has no parent device"),
        }
    }
}

impl std::error::Error for Error {}

impl From<Error> for crate::vhci::Error {
    fn from(value: Error) -> Self {
        crate::vhci::Error::Udev(value)
    }
}

pub fn get_sysattr<'a>(
    dev: &'a udev::Device,
    attribute: Cow<'static, str>,
) -> Result<&'a str, Error> {
    Ok(dev
        .attribute_value(attribute.as_ref())
        .ok_or_else(|| Error::NoAttribute(attribute))?
        .to_str()
        .unwrap())
}

pub fn get_sysattr_clone_err<'a, 'b>(
    dev: &'a udev::Device,
    attribute: &'b str,
) -> Result<&'a str, Error> {
    Ok(dev
        .attribute_value(attribute)
        .ok_or_else(|| Error::NoAttribute(Cow::Owned(attribute.to_owned())))?
        .to_str()
        .unwrap())
}
