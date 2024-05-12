use core::fmt;
use std::{io, net::TcpStream};

#[cfg(unix)]
use std::net::SocketAddr;

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    AttachFailed(AttachError),
    #[cfg(windows)]
    Windows(::windows::core::Error),
    #[cfg(windows)]
    MultipleDevInterfaces(usize),
    //#[cfg(windows)]
    #[cfg(unix)]
    Udev(crate::unix::UdevError),
    #[cfg(unix)]
    NoFreeControllers,
    #[cfg(unix)]
    NoFreePorts,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(i) => write!(f, "VHCI I/O (is driver loaded?): {i}"),
            Error::AttachFailed(a) => write!(f, "VHCI Attach Failed: {a}"),
            #[cfg(windows)]
            Error::Windows(_) => todo!(),
            #[cfg(windows)]
            Error::MultipleDevInterfaces(num) => write!(f, "Multiple instances of VHCI device interface found ({num})"),
            #[cfg(unix)]
            Error::Udev(u) => write!(f, "VHCI Udev (is driver loaded?): {u}"),
            #[cfg(unix)]
            Error::NoFreeControllers => todo!(),
            #[cfg(unix)]
            Error::NoFreePorts => todo!(),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Debug)]
pub enum AttachErrorKind {
    OutOfPorts,
    #[cfg(windows)]
    Door(crate::windows::vhci::DoorError),
    #[cfg(unix)]
    SysFs(io::Error),
}

impl fmt::Display for AttachErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AttachErrorKind::OutOfPorts => write!(f, "Out of ports"),
            #[cfg(windows)]
            AttachErrorKind::Door(d) => write!(f, "Driver error: {}", d),
            #[cfg(unix)]
            AttachErrorKind::SysFs(i) => todo!()
        }
    }
}

#[derive(Debug)]
pub struct AttachError {
    pub(crate) socket: TcpStream,
    pub(crate) kind: AttachErrorKind,
}

impl AttachError {
    pub fn into_parts(self) -> (TcpStream, AttachErrorKind) {
        (self.socket, self.kind)
    }
}

impl fmt::Display for AttachError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} (socket: {})",
            self.kind,
            self.socket.peer_addr().unwrap()
        )
    }
}

impl std::error::Error for AttachError {}

impl From<io::Error> for Error {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

#[cfg(windows)]
impl From<::windows::core::Error> for Error {
    fn from(value: ::windows::core::Error) -> Self {
        Self::Windows(value)
    }
}
