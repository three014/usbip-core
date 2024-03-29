use core::fmt;
use std::{io, net::{SocketAddr, TcpStream}};

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    AttachFailed(AttachError),
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
            Error::Io(i) => write!(f, "VHCI I/O: {i}"),
            Error::AttachFailed(a) => write!(f, "VHCI Attach Failed: {a}"),
            Error::Udev(u) => write!(f, "VHCI Udev: {u}"),
            Error::NoFreeControllers => todo!(),
            Error::NoFreePorts => todo!(),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Debug)]
pub enum AttachErrorKind {
    OutOfPorts,
    #[cfg(unix)]
    SysFs(io::Error)
}

impl fmt::Display for AttachErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!()
    }
}

#[derive(Debug)]
pub struct AttachError {
    pub(crate) socket: TcpStream,
    pub(crate) kind: AttachErrorKind
}

impl AttachError {
    pub fn into_parts(self) -> (TcpStream, AttachErrorKind) {
        (self.socket, self.kind)
    }
}

impl fmt::Display for AttachError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (socket: {})", self.kind, self.socket.peer_addr().unwrap())
    }
}

impl std::error::Error for AttachError {}

impl From<io::Error> for Error {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}


