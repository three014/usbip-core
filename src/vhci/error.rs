use core::fmt;
use std::{io, net::SocketAddr};

#[derive(Debug)]
pub enum Error {
    IO(io::Error),
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
        todo!()
    }
}

impl std::error::Error for Error {}

#[derive(Debug)]
pub struct AttachError {
    socket: SocketAddr,
    errno: io::Error
}

impl AttachError {
    pub fn into_parts(self) -> (SocketAddr, io::Error) {
        (self.socket, self.errno)
    }
}

impl fmt::Display for AttachError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (socket: {})", self.errno, self.socket)
    }
}

impl std::error::Error for AttachError {}

impl From<io::Error> for Error {
    fn from(value: io::Error) -> Self {
        Self::IO(value)
    }
}


