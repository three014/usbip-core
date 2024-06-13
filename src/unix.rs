mod udev_utils {
    use std::str::FromStr;

    use crate::util::__private::Sealed;

    pub trait UdevExt: Sealed {
        fn sysattr<T>(&self, attr: &str) -> Result<T, Error<T::Err>>
        where
            T: FromStr;
        fn sysattr_str(&self, attr: &str) -> Result<&str, Error<()>>;
    }

    impl Sealed for udev::Device {}
    impl UdevExt for udev::Device {
        fn sysattr<T>(&self, attr: &str) -> Result<T, Error<T::Err>>
        where
            T: FromStr,
        {
            self.attribute_value(attr)
                .ok_or(Error::AttributeNotFound)?
                .to_str()
                .ok_or(Error::NotUtf8)?
                .parse()
                .map_err(Error::CustomErr)
        }

        fn sysattr_str(&self, attr: &str) -> Result<&str, Error<()>> {
            self.attribute_value(attr)
                .ok_or(Error::AttributeNotFound)?
                .to_str()
                .ok_or(Error::NotUtf8)
        }
    }

    #[derive(Debug)]
    pub enum Error<T> {
        AttributeNotFound,
        NotUtf8,
        CustomErr(T),
    }

    impl<T> Error<T> {
        /// Consumes `self` and returns the inner
        /// error if it was the custom error value.
        ///
        /// # Panic
        /// This function panics if `self` was
        /// not the `Error::CustomErr` variant.
        pub fn into_custom_err(self) -> T {
            match self {
                Error::AttributeNotFound => panic!("udev attribute not found"),
                Error::NotUtf8 => panic!("udev attribute value not in utf8"),
                Error::CustomErr(err) => err,
            }
        }
    }

    impl<T: std::error::Error + 'static> Error<T> {
        pub fn into_dyn(self) -> Error<Box<dyn std::error::Error>> {
            match self {
                Error::AttributeNotFound => Error::AttributeNotFound,
                Error::NotUtf8 => Error::NotUtf8,
                Error::CustomErr(err) => Error::CustomErr(crate::util::into_dyn_err(err)),
            }
        }
    }
}
mod sysfs {
    use std::path::Path;

    use crate::containers::stacktools::StackStr;

    pub const PATH_MAX: usize = 255;

    pub fn open<P: AsRef<Path>>(path: P) -> std::io::Result<std::fs::File> {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
    }

    pub struct SysAttr {
        attr: std::fs::File,
    }

    impl SysAttr {
        pub fn open(path: &str, attr: &str) -> std::io::Result<Self> {
            let syspath = StackStr::<PATH_MAX>::try_from(format_args!("{path}/{attr}")).unwrap();
            let file = open(&*syspath)?;
            Ok(Self { attr: file })
        }
    }

    impl std::io::Write for SysAttr {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.attr.write(buf)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.attr.flush()
        }
    }
}
pub mod vhci2;
pub mod host {
    use std::path::PathBuf;

    use crate::unix::udev_utils::UdevExt;

    mod sysfs {
        use crate::{
            containers::stacktools::StackStr,
            unix::sysfs::{SysAttr, PATH_MAX},
        };

        use super::SYS_PATH;

        use std::io::Write;

        pub fn match_busid_add(bus_id: &str) -> std::io::Result<()> {
            let mut sys = SysAttr::open(SYS_PATH, "match_busid")?;
            write!(sys, "add {bus_id}")
        }

        pub fn match_busid_del(bus_id: &str) -> std::io::Result<()> {
            let mut sys = SysAttr::open(SYS_PATH, "match_busid")?;
            write!(sys, "del {bus_id}")
        }

        pub fn bind(bus_id: &str) -> std::io::Result<()> {
            let mut sys = SysAttr::open(SYS_PATH, "bind")?;
            write!(sys, "{bus_id}")
        }

        pub fn rebind(bus_id: &str) -> std::io::Result<()> {
            let mut sys = SysAttr::open(SYS_PATH, "rebind")?;
            write!(sys, "{bus_id}")
        }

        pub fn unbind_other(udev: &udev::Device, bus_id: &str) -> std::io::Result<()> {
            if let Some(driver) = udev.driver() {
                let driver = driver.to_str().expect("turning udev driver name into str");
                let syspath =
                    StackStr::<PATH_MAX>::try_from(format_args!("/sys/bus/usb/drivers/{driver}"))
                        .unwrap();
                let mut sys = SysAttr::open(&*syspath, "unbind")?;
                write!(sys, "{bus_id}")
            } else {
                Ok(())
            }
        }

        pub fn unbind(bus_id: &str) -> std::io::Result<()> {
            let mut sys = SysAttr::open(SYS_PATH, "unbind")?;
            write!(sys, "{bus_id}")
        }
    }

    static DRIVER_NAME: &str = "usbip-host";
    static SYS_PATH: &str = "/sys/bus/usb/drivers/usbip-host";

    pub enum Error {
        BusIdNotFound,
        BindLoop(PathBuf),
        AlreadyBound,
        UnbindFailed(Option<std::io::Error>),
        BindFailed(std::io::Error),
    }

    pub type Result<T> = std::result::Result<T, Error>;

    pub struct Driver {
        context: udev::Udev,
    }

    impl Driver {
        #[inline]
        pub fn new() -> std::io::Result<Self> {
            Ok(Self {
                context: udev::Udev::new()?,
            })
        }

        pub fn bind(&self, bus_id: &str) -> Result<()> {
            // Do verification first
            let dev = udev::Device::from_subsystem_sysname_with_context(
                self.context.clone(),
                "usb".to_owned(),
                bus_id.to_owned(),
            )
            .map_err(|_| Error::BusIdNotFound)?;

            if dev.devpath().to_str().unwrap().contains(DRIVER_NAME) {
                return Err(Error::BindLoop(PathBuf::from(dev.devpath())));
            }

            self.unbind_other(bus_id)?;

            // Bind away!
            sysfs::match_busid_add(bus_id).map_err(Error::BindFailed)?;
            sysfs::bind(bus_id).map_err(Error::BindFailed)?;

            todo!()
        }

        fn unbind_other(&self, bus_id: &str) -> Result<()> {
            let dev = udev::Device::from_subsystem_sysname_with_context(
                self.context.clone(),
                "usb".to_owned(),
                bus_id.to_owned(),
            )
            .map_err(|_| Error::BusIdNotFound)?;

            let b_dev_class: u32 = dev.sysattr("bDeviceClass").unwrap();

            if b_dev_class == 9 {
                return Err(Error::UnbindFailed(None));
            }

            if let Some(driver) = dev.driver() {
                if driver.to_str().unwrap() == DRIVER_NAME {
                    return Err(Error::AlreadyBound);
                }
            }

            sysfs::unbind_other(&dev, &bus_id).map_err(|err| Error::UnbindFailed(Some(err)))
        }
    }
}
mod net {
    use std::{
        ffi::c_int,
        net::{SocketAddr, TcpStream},
        os::fd::{AsFd, AsRawFd},
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
    containers::stacktools::{self, StackStr},
    unix::udev_utils::UdevExt,
    DeviceSpeed, BUS_ID_SIZE, DEV_PATH_MAX, SysPath, BusId,
};
use std::{ffi::OsStr, os::unix::ffi::OsStrExt, path::Path, borrow::Cow};

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
    type Error = udev_utils::Error<Box<dyn std::error::Error>>;

    fn try_from(udev: udev::Device) -> Result<Self, Self::Error> {
        let path: StackStr<{ DEV_PATH_MAX - 1 }> = udev
            .syspath()
            .try_into()
            .map_err(|err| udev_utils::Error::CustomErr(err).into_dyn())?;
        let busid: StackStr<{ BUS_ID_SIZE - 1 }> = udev
            .sysname()
            .try_into()
            .map_err(|err| udev_utils::Error::CustomErr(err).into_dyn())?;
        let id_vendor: u16 = udev.sysattr("idVendor").map_err(|err| err.into_dyn())?;
        let id_product: u16 = udev.sysattr("idProduct").map_err(|err| err.into_dyn())?;
        let busnum: u32 = udev.sysattr("busnum").map_err(|err| err.into_dyn())?;
        let devnum: u32 = udev.devnum().ok_or(udev_utils::Error::AttributeNotFound)? as _;
        let speed: DeviceSpeed = udev.sysattr("speed").map_err(|err| err.into_dyn())?;
        let bcd_device: u16 = udev.sysattr("bcdDevice").map_err(|err| err.into_dyn())?;
        let b_device_class: u8 = udev.sysattr("bDeviceClass").map_err(|err| err.into_dyn())?;
        let b_device_subclass: u8 = udev
            .sysattr("bDeviceSubClass")
            .map_err(|err| err.into_dyn())?;
        let b_device_protocol: u8 = udev
            .sysattr("bDeviceProtocol")
            .map_err(|err| err.into_dyn())?;
        let b_configuration_value: u8 = udev
            .sysattr("bConfigurationValue")
            .map_err(|err| err.into_dyn())?;
        let b_num_configurations: u8 = udev.sysattr("bNumConfigurations").ok().unwrap_or_default();
        let b_num_interfaces: u8 = udev.sysattr("bNumInterfaces").ok().unwrap_or_default();

        Ok(Self {
            path: SysPath::new(Cow::Owned(path)),
            busid: BusId::new(Cow::Owned(busid)),
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
