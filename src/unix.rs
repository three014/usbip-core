mod udev_helpers;
pub mod vhci2;
mod net {
    use std::{
        ffi::c_int,
        net::{SocketAddr, TcpStream},
        os::fd::{AsRawFd, AsFd},
    };

    use libc::{c_void, socklen_t};

    use crate::{
        net::{bincode_config, Error, Recv},
        util::__private::Sealed,
    };

    pub struct UsbipStream(TcpStream);

    impl UsbipStream {
        #[inline(always)]
        const fn new(inner: TcpStream) -> Self {
            Self(inner)
        }

        #[inline(always)]
        const fn get(&self) -> &TcpStream {
            &self.0
        }

        #[inline(always)]
        fn get_mut(&mut self) -> &mut TcpStream {
            &mut self.0
        }

        pub fn connect(host: &SocketAddr) -> std::io::Result<Self> {
            let socket = TcpStream::connect(host)?;
            socket.set_nodelay(true)?;
            socket.set_keepalive(true)?;
            Ok(Self::new(socket))
        }

        pub fn peer_addr(&self) -> std::io::Result<SocketAddr> {
            self.get().peer_addr()
        }
    }

    impl std::io::Read for UsbipStream {
        #[inline(always)]
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.get_mut().read(buf)
        }
    }

    impl std::io::Write for UsbipStream {
        #[inline(always)]
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.get_mut().write(buf)
        }

        #[inline(always)]
        fn flush(&mut self) -> std::io::Result<()> {
            self.get_mut().flush()
        }
    }

    impl crate::net::Send for UsbipStream {
        fn send<T: bincode::Encode>(&mut self, data: &T) -> Result<usize, Error> {
            bincode::encode_into_std_write(data, self, bincode_config()).map_err(Error::Enc)
        }
    }

    impl Recv for UsbipStream {
        fn recv<T: bincode::Decode>(&mut self) -> Result<T, Error> {
            bincode::decode_from_std_read(self, bincode_config()).map_err(Error::De)
        }
    }

    pub trait TcpStreamExt: Sealed {
        fn set_keepalive(&self, keepalive: bool) -> std::io::Result<()>;
    }

    impl Sealed for TcpStream {}
    impl Sealed for UsbipStream {}

    impl TcpStreamExt for TcpStream {
        fn set_keepalive(&self, keepalive: bool) -> std::io::Result<()> {
            let val = c_int::from(keepalive);
            let rc = unsafe {
                libc::setsockopt(
                    self.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_KEEPALIVE,
                    core::ptr::addr_of!(val).cast::<c_void>(),
                    socklen_t::try_from(core::mem::size_of::<c_int>()).unwrap(),
                )
            };
            if rc < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        }
    }

    impl AsFd for UsbipStream {
        fn as_fd(&self) -> std::os::unix::prelude::BorrowedFd<'_> {
            self.get().as_fd()
        }
    }
}

use crate::{
    containers::{
        beef::Beef,
        stacktools::{self, StackStr},
    },
    unix::udev_helpers::UdevHelper,
    BUS_ID_SIZE, DEV_PATH_MAX,
};
use std::{borrow::Cow, ffi::OsStr, os::unix::ffi::OsStrExt, path::Path};
use udev;
pub use udev_helpers::Error as UdevError;

use udev_helpers::ParseAttributeError;

pub static USB_IDS: &str = "/usr/share/hwdata/usb.ids";

impl<const N: usize> TryFrom<&OsStr> for StackStr<N> {
    type Error = stacktools::TryFromStrErr;

    fn try_from(value: &OsStr) -> Result<Self, Self::Error> {
        std::str::from_utf8(value.as_bytes())
            .map_err(|err| stacktools::TryFromStrErr::NotUtf8(err))?
            .try_into()
    }
}

impl<const N: usize> TryFrom<&Path> for StackStr<N> {
    type Error = stacktools::TryFromStrErr;

    fn try_from(value: &Path) -> Result<Self, Self::Error> {
        value.as_os_str().try_into()
    }
}

impl TryFrom<udev::Device> for crate::UsbDevice {
    type Error = ParseAttributeError;

    fn try_from(udev: udev::Device) -> Result<Self, Self::Error> {
        let path: StackStr<DEV_PATH_MAX> = udev.syspath().try_into()?;
        let busid: StackStr<BUS_ID_SIZE> = udev.sysname().try_into()?;
        let id_vendor: u16 = udev.parse_sysattr(Beef::Static("idVendor"))?;
        let id_product: u16 = udev.parse_sysattr(Beef::Static("idProduct"))?;
        let busnum: u32 = udev.parse_sysattr(Beef::Static("busnum"))?;
        let devnum = u32::try_from(
            udev.devnum()
                .ok_or(ParseAttributeError::NoAttribute(Cow::Borrowed("devnum")))?,
        )
        .unwrap();
        let speed = udev.parse_sysattr(Beef::Static("speed"))?;
        let bcd_device: u16 = udev.parse_sysattr(Beef::Static("bcdDevice"))?;
        let b_device_class: u8 = udev.parse_sysattr(Beef::Static("bDeviceClass"))?;
        let b_device_subclass: u8 = udev.parse_sysattr(Beef::Static("bDeviceSubClass"))?;
        let b_device_protocol: u8 = udev.parse_sysattr(Beef::Static("bDeviceProtocol"))?;
        let b_configuration_value: u8 = udev.parse_sysattr(Beef::Static("bConfigurationValue"))?;
        let b_num_configurations: u8 = udev
            .parse_sysattr(Beef::Static("bNumConfigurations"))
            .ok()
            .unwrap_or_default();
        let b_num_interfaces: u8 = udev
            .parse_sysattr(Beef::Static("bNumInterfaces"))
            .ok()
            .unwrap_or_default();

        Ok(Self {
            path,
            busid,
            id_vendor,
            id_product,
            busnum,
            devnum,
            speed,
            bcd_device,
            b_device_class,
            b_device_subclass,
            b_device_protocol,
            b_configuration_value,
            b_num_configurations,
            b_num_interfaces,
        })
    }
}
