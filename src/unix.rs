pub mod names;
mod udev_helpers;
pub mod vhci;
pub mod vhci2 {
    #[cfg(test)]
    mod tests {
        use crate::vhci::VhciDriver;

        use super::*;

        #[test]
        fn driver_opens() {
            let _driver = UnixDriver::open().unwrap();
        }
    }

    use std::{
        collections::HashMap,
        ffi::{c_char, OsStr},
        io::Write,
        net::TcpStream,
        num::NonZeroUsize,
        os::{fd::AsRawFd, unix::ffi::OsStrExt},
        str::FromStr,
    };

    use crate::{
        util::{beef::Beef, buffer::Buffer, parse_token},
        vhci::{HubSpeed, ImportedDeviceInner},
        DeviceSpeed, DeviceStatus,
    };

    use super::{
        udev_helpers::{TryFromDeviceError, UdevHelper},
        UdevError,
    };

    static BUS_TYPE: &str = "platform";
    static DEVICE_NAME: &str = "vhci_hcd.0";
    const SYSFS_PATH_MAX: usize = 255;

    enum MaybeIdev {
        InUse(UnixImportedDevice),
        Empty(IdevSkeleton),
    }

    impl FromStr for MaybeIdev {
        type Err = TryFromDeviceError;

        fn from_str(s: &str) -> Result<Self, Self::Err> {
            let mut tokens = s.split_whitespace();
            let hub: HubSpeed = parse_token(&mut tokens);
            let port: u16 = parse_token(&mut tokens);
            let status: DeviceStatus = parse_token(&mut tokens);
            let _speed: u32 = parse_token(&mut tokens);
            let devid: u32 = parse_token(&mut tokens);
            let _sockfd: u32 = parse_token(&mut tokens);
            let busid = tokens.next().unwrap().trim();
            match status {
                // This port is unused or not ready yet
                DeviceStatus::PortAvailable | DeviceStatus::PortInitializing => {
                    Ok(MaybeIdev::Empty(IdevSkeleton {
                        port,
                        hub_speed: hub,
                        status,
                    }))
                }
                _ => {
                    let sudev =
                        udev::Device::from_subsystem_sysname("usb".to_owned(), busid.to_owned())?;
                    let usb_dev = crate::UsbDevice::try_from(sudev)?;
                    let idev = UnixImportedDevice {
                        inner: ImportedDeviceInner {
                            hub,
                            port,
                            status,
                            vendor: usb_dev.id_vendor,
                            product: usb_dev.id_product,
                            devid,
                            udev: usb_dev,
                        },
                    };
                    debug_assert_eq!(idev.inner.usb_dev().busnum, idev.inner.dev_id() >> 16);
                    debug_assert_eq!(
                        idev.inner.usb_dev().devnum,
                        idev.inner.dev_id() & 0x0000ffff
                    );
                    Ok(MaybeIdev::InUse(idev))
                }
            }
        }
    }

    #[derive(Debug)]
    pub struct UnixImportedDevice {
        inner: ImportedDeviceInner,
    }

    #[derive(Debug, Clone, Copy)]
    struct IdevSkeleton {
        port: u16,
        hub_speed: HubSpeed,
        status: DeviceStatus,
    }

    #[derive(Debug)]
    enum ActivateError {
        PortNotAvailable,
    }

    #[derive(Debug)]
    pub(crate) struct ImportedDevices {
        active: HashMap<u16, UnixImportedDevice>,
        available: HashMap<u16, IdevSkeleton>,
    }

    impl ImportedDevices {
        fn new() -> Self {
            Self {
                active: HashMap::new(),
                available: HashMap::new(),
            }
        }

        fn next_available(&self, speed: DeviceSpeed) -> Option<u16> {
            speed.try_into().ok().and_then(|hub_speed| {
                self.available()
                    .values()
                    .find(|x| x.hub_speed == hub_speed && x.status == DeviceStatus::PortAvailable)
                    .map(|x| x.port)
            })
        }

        fn activate<F>(&mut self, port: u16, init: F) -> Result<(), ActivateError>
        where
            F: FnOnce(IdevSkeleton) -> UnixImportedDevice,
        {
            let idev = self
                .available_mut()
                .remove(&port)
                .ok_or(ActivateError::PortNotAvailable)
                .map(init)?;
            self.active_mut().insert(idev.inner.port(), idev);

            Ok(())
        }

        fn active_mut(&mut self) -> &mut HashMap<u16, UnixImportedDevice> {
            &mut self.active
        }

        fn available_mut(&mut self) -> &mut HashMap<u16, IdevSkeleton> {
            &mut self.available
        }

        fn available(&self) -> &HashMap<u16, IdevSkeleton> {
            &self.available
        }
    }

    impl FromIterator<MaybeIdev> for ImportedDevices {
        fn from_iter<T: IntoIterator<Item = MaybeIdev>>(iter: T) -> Self {
            let mut imported_devices = ImportedDevices::new();
            for item in iter {
                match item {
                    MaybeIdev::InUse(idev) => {
                        imported_devices
                            .active_mut()
                            .insert(idev.inner.port(), idev);
                    }
                    MaybeIdev::Empty(skel) => {
                        imported_devices.available_mut().insert(skel.port, skel);
                    }
                }
            }
            imported_devices
        }
    }

    struct InitData<'a> {
        hc_device: &'a udev::Device,
        num_controllers: NonZeroUsize,
        num_ports: NonZeroUsize,
    }

    impl TryFrom<InitData<'_>> for ImportedDevices {
        type Error = UdevError;

        fn try_from(init: InitData<'_>) -> Result<Self, Self::Error> {
            let mut attr_buf = Buffer::<20, c_char>::new();
            let mut idevs = Vec::with_capacity(init.num_ports.get());

            write!(attr_buf.as_mut_u8_bytes(), "status").unwrap();

            for i in 0..init.num_controllers.get() {
                if i > 0 {
                    attr_buf.as_mut_u8_bytes().fill(0);
                    write!(attr_buf.as_mut_u8_bytes(), "status.{i}").unwrap();
                }

                let status = init.hc_device.sysattr(Beef::Borrowed(
                    attr_buf.to_str().unwrap().trim().trim_matches(char::from(0)),
                ))?;
                let mut lines = status.lines();
                lines.next();
                for line in lines {
                    idevs.push(line.parse().map_err(Into::<UdevError>::into)?);
                }
            }
            Ok(idevs.into_iter().collect())
        }
    }

    #[derive(Debug, Clone)]
    pub struct UsbId<'a> {
        inner: crate::vhci::UsbIdInner<'a>,
        dev_id: u32,
        speed: crate::DeviceSpeed,
    }

    impl crate::UsbDevice {
        pub fn usb_id(&self) -> UsbId<'_> {
            UsbId {
                inner: crate::vhci::UsbIdInner {
                    bus_id: self.bus_id(),
                },
                dev_id: self.dev_id(),
                speed: self.speed(),
            }
        }
    }

    pub struct UnixDriver {
        inner: DriverInner,
    }

    struct DriverInner {
        hc_device: udev::Device,
        imported_devices: ImportedDevices,
        num_controllers: NonZeroUsize,
        num_ports: NonZeroUsize,
    }

    impl DriverInner {
        fn try_open() -> crate::vhci::Result<Self> {
            let hc_device =
                udev::Device::from_subsystem_sysname(BUS_TYPE.into(), DEVICE_NAME.into())?;
            let num_ports: NonZeroUsize = hc_device.parse_sysattr(Beef::Static("nports"))?;
            let num_controllers = num_controllers(&hc_device)?;
            let imported_devices = InitData {
                hc_device: &hc_device,
                num_controllers,
                num_ports,
            }
            .try_into()?;

            Ok(Self {
                hc_device,
                imported_devices,
                num_controllers,
                num_ports,
            })
        }

        const fn udev(&self) -> &udev::Device {
            &self.hc_device
        }

        fn imported_devices(&self) -> impl ExactSizeIterator<Item = &'_ UnixImportedDevice> + '_ {
            self.imported_devices.active.values()
        }

        const fn num_controllers(&self) -> NonZeroUsize {
            self.num_controllers
        }

        const fn num_ports(&self) -> NonZeroUsize {
            self.num_ports
        }

        fn refresh_imported_devices(&mut self) -> Result<(), crate::vhci::Error> {
            self.imported_devices = InitData {
                hc_device: self.udev(),
                num_controllers: self.num_controllers(),
                num_ports: self.num_ports(),
            }
            .try_into()?;
            Ok(())
        }

        fn attach(&mut self, socket: TcpStream, usb_id: UsbId) -> crate::vhci::Result<u16> {
            use crate::vhci::error::*;
            let port = match self.imported_devices.next_available(usb_id.speed) {
                Some(port) => port,
                None => {
                    return Err(Error::AttachFailed(AttachError {
                        socket,
                        kind: AttachErrorKind::OutOfPorts,
                    }))
                }
            };
            let mut path_buf = Buffer::<SYSFS_PATH_MAX, c_char>::new();
            let path = self.hc_device.syspath();
            write!(path_buf.as_mut_u8_bytes(), "{:?}/attach", path).unwrap();
            let mut file = match std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(OsStr::from_bytes(path_buf.as_u8_bytes()))
            {
                Ok(file) => file,
                Err(e) => {
                    return Err(Error::AttachFailed(AttachError {
                        socket,
                        kind: AttachErrorKind::SysFs(e),
                    }))
                }
            };
            let mut buf = Buffer::<200, c_char>::new();

            write!(
                buf.as_mut_u8_bytes(),
                "{} {} {} {}",
                port,
                socket.as_raw_fd(),
                usb_id.dev_id,
                usb_id.speed as u32
            )
            .unwrap();
            if let Err(e) = file.write_all(buf.as_u8_bytes()) {
                return Err(Error::AttachFailed(AttachError {
                    socket,
                    kind: AttachErrorKind::SysFs(e),
                }));
            }

            self.imported_devices
                .activate(port, |_| {
                    let udev = udev::Device::from_subsystem_sysname(
                        "usb".to_owned(),
                        usb_id.inner.bus_id().to_owned(),
                    )
                    .expect("Creating repr of device already attached");
                    let usb_dev: crate::UsbDevice = udev.try_into().unwrap();
                    UnixImportedDevice {
                        inner: ImportedDeviceInner {
                            hub: usb_id.speed.try_into().unwrap(),
                            port,
                            status: DeviceStatus::PortInUse,
                            vendor: usb_dev.id_vendor,
                            product: usb_dev.id_product,
                            devid: usb_id.dev_id,
                            udev: usb_dev,
                        },
                    }
                })
                .expect("Port should've been open");
            Ok(port)
        }
    }

    impl crate::vhci::VhciDriver for UnixDriver {
        fn open() -> crate::vhci::Result<Self> {
            Ok(Self {
                inner: DriverInner::try_open()?,
            })
        }

        fn detach(&mut self, port: u16) -> crate::vhci::Result<()> {
            todo!()
        }

        fn imported_devices(&self) -> impl ExactSizeIterator<Item = &'_ UnixImportedDevice> + '_ {
            self.inner.imported_devices()
        }

        fn attach(&mut self, socket: TcpStream, usb_id: UsbId) -> crate::vhci::Result<u16> {
            self.inner.attach(socket, usb_id)
        }
    }

    fn num_controllers(hc_device: &udev::Device) -> crate::vhci::Result<NonZeroUsize> {
        use crate::vhci::Error;
        let platform = hc_device.parent().ok_or(Error::Udev(UdevError::NoParent))?;
        let count: NonZeroUsize = platform
            .syspath()
            .read_dir()?
            .filter(|e| {
                e.as_ref().is_ok_and(|entry| {
                    entry
                        .file_name()
                        .as_os_str()
                        .to_str()
                        .is_some_and(|name| name.starts_with("vhci_hcd."))
                })
            })
            .count()
            .try_into()
            .map_err(|_| Error::NoFreeControllers)?;
        Ok(count)
    }

    impl crate::util::__private::Sealed for UnixDriver {}
}

use crate::{
    net::Status,
    unix::udev_helpers::UdevHelper,
    util::{
        beef::Beef,
        buffer::{Buffer, FormatError},
    },
    DeviceStatus,
};
pub use ffi::{SYSFS_BUS_ID_SIZE, SYSFS_PATH_MAX};
pub use libusbip_sys::unix as ffi;
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, ffi::OsStr, io, os::unix::ffi::OsStrExt, path::Path, str::FromStr};
pub use udev;
pub use udev_helpers::Error as UdevError;

use self::udev_helpers::ParseAttributeError;

impl<const N: usize> TryFrom<&OsStr> for Buffer<N, i8> {
    type Error = FormatError;

    fn try_from(value: &OsStr) -> Result<Self, Self::Error> {
        value.as_bytes().try_into()
    }
}

impl<const N: usize> TryFrom<&Path> for Buffer<N, i8> {
    type Error = FormatError;

    fn try_from(value: &Path) -> Result<Self, Self::Error> {
        value.as_os_str().try_into()
    }
}

impl From<ffi::usbip_device_status> for DeviceStatus {
    fn from(value: ffi::usbip_device_status) -> Self {
        match value {
            ffi::usbip_device_status::SDEV_ST_AVAILABLE => Self::DevAvailable,
            ffi::usbip_device_status::SDEV_ST_USED => Self::DevInUse,
            ffi::usbip_device_status::SDEV_ST_ERROR => Self::DevError,
            ffi::usbip_device_status::VDEV_ST_NULL => Self::PortAvailable,
            ffi::usbip_device_status::VDEV_ST_NOTASSIGNED => Self::PortInitializing,
            ffi::usbip_device_status::VDEV_ST_USED => Self::PortInUse,
            ffi::usbip_device_status::VDEV_ST_ERROR => Self::PortError,
        }
    }
}

impl From<ffi::op_code_status> for Status {
    fn from(value: ffi::op_code_status) -> Self {
        match value {
            ffi::op_code_status::ST_OK => Self::Success,
            ffi::op_code_status::ST_NA => Self::Failed,
            ffi::op_code_status::ST_DEV_BUSY => Self::DevBusy,
            ffi::op_code_status::ST_DEV_ERR => Self::DevErr,
            ffi::op_code_status::ST_NODEV => Self::NoDev,
            ffi::op_code_status::ST_ERROR => Self::Unexpected,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbDevice {
    path: Buffer<SYSFS_PATH_MAX, i8>,
    busid: Buffer<SYSFS_BUS_ID_SIZE, i8>,
    busnum: u32,
    devnum: u32,
    speed: u32,
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
    pub fn id(&self) -> ID {
        ID {
            vendor: self.id_vendor,
            product: self.id_product,
        }
    }

    pub fn class(&self) -> Class {
        Class {
            class: self.b_device_class,
            subclass: self.b_device_subclass,
            protocol: self.b_device_protocol,
        }
    }

    pub fn info(&self) -> Info {
        Info {
            devnum: self.devnum,
            busnum: self.busnum,
            speed: self.speed,
        }
    }
}

impl From<ffi::usbip_usb_device> for UsbDevice {
    fn from(value: ffi::usbip_usb_device) -> Self {
        Self {
            path: value.path.into(),
            busid: value.busid.into(),
            busnum: value.busnum,
            devnum: value.devnum,
            speed: value.speed,
            id_vendor: value.idVendor,
            id_product: value.idProduct,
            bcd_device: value.bcdDevice,
            b_device_class: value.bDeviceClass,
            b_device_subclass: value.bDeviceSubClass,
            b_device_protocol: value.bDeviceProtocol,
            b_configuration_value: value.bConfigurationValue,
            b_num_configurations: value.bNumConfigurations,
            b_num_interfaces: value.bNumInterfaces,
        }
    }
}

impl TryFrom<udev::Device> for crate::UsbDevice {
    type Error = ParseAttributeError;

    fn try_from(udev: udev::Device) -> Result<Self, Self::Error> {
        let path: Buffer<{ crate::DEV_PATH_MAX }, i8> = udev.syspath().try_into()?;
        let busid: Buffer<{ crate::BUS_ID_SIZE }, i8> = udev.sysname().try_into()?;
        let id_vendor: u16 = udev.parse_sysattr(Beef::Static("idVendor"))?;
        let id_product: u16 = udev.parse_sysattr(Beef::Static("idProduct"))?;
        let busnum: u32 = udev.parse_sysattr(Beef::Static("busnum"))?;
        let devnum = u32::try_from(
            udev.devnum()
                .ok_or(ParseAttributeError::NoAttribute(Cow::Borrowed("devnum")))?,
        )
        .unwrap();
        let speed = udev
            .sysattr(Beef::Static("speed"))?
            .parse::<u32>()?
            .try_into()
            .map_err(|e| -> Box<dyn std::error::Error> { Box::new(e) })?;
        let bcd_device: u16 = udev.parse_sysattr(Beef::Static("bcdDevice"))?;
        let b_device_class: u8 = udev.parse_sysattr(Beef::Static("bDeviceClass"))?;
        let b_device_subclass: u8 = udev.parse_sysattr(Beef::Static("bDeviceSubClass"))?;
        let b_device_protocol: u8 = udev.parse_sysattr(Beef::Static("bDeviceProtocol"))?;
        let b_configuration_value: u8 = udev.parse_sysattr(Beef::Static("bConfigurationValue"))?;
        let b_num_configurations: u8 = udev.parse_sysattr(Beef::Static("bNumConfigurations"))?;
        let b_num_interfaces: u8 = udev.parse_sysattr(Beef::Static("bNumInterfaces"))?;

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

impl TryFrom<udev::Device> for UsbDevice {
    type Error = Box<str>;

    fn try_from(udev: udev::Device) -> Result<Self, Self::Error> {
        fn get_attribute<T, V>(udev: &udev::Device, value: V) -> Result<T, Box<str>>
        where
            T: FromStr,
            <T as FromStr>::Err: ToString,
            V: AsRef<OsStr>,
        {
            fn inner<'a>(udev: &'a udev::Device, value: &OsStr) -> Result<&'a str, Box<str>> {
                udev.attribute_value(value)
                    .ok_or_else(|| {
                        format!(
                            "Problem getting device attributes: {}",
                            io::Error::last_os_error()
                        )
                    })?
                    .to_str()
                    .ok_or_else(|| "notutf8_err".into())
            }

            inner(udev, value.as_ref())?
                .parse::<T>()
                .map_err(|e| e.to_string().into_boxed_str())
        }

        let buf_err = |err: FormatError| err.to_string();

        let path: Buffer<SYSFS_PATH_MAX, i8> = udev.syspath().try_into().map_err(buf_err)?;
        let busid: Buffer<SYSFS_BUS_ID_SIZE, i8> = udev.sysname().try_into().map_err(buf_err)?;
        let id = ID {
            vendor: get_attribute(&udev, "idVendor")?,
            product: get_attribute(&udev, "idProduct")?,
        };
        let info = Info {
            busnum: get_attribute(&udev, "busnum")?,
            devnum: u32::try_from(udev.devnum().ok_or("devnum not found")?).unwrap(),
            speed: get_attribute(&udev, "speed")?,
        };
        let bcd_device = get_attribute(&udev, "bcdDevice")?;
        let b_device_class = get_attribute(&udev, "bDeviceClass")?;
        let b_device_subclass = get_attribute(&udev, "bDeviceSubClass")?;
        let b_device_protocol = get_attribute(&udev, "bDeviceProtocol")?;
        let b_configuration_value = get_attribute(&udev, "bConfigurationValue")?;
        let b_num_configurations = get_attribute(&udev, "bNumConfigurations")?;
        let b_num_interfaces = get_attribute(&udev, "bNumInterfaces")?;

        Ok(Self {
            path,
            busid,
            id_vendor: id.vendor,
            id_product: id.product,
            busnum: info.busnum,
            devnum: info.devnum,
            speed: info.speed,
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

#[derive(Debug, Clone, Copy)]
pub struct Info {
    devnum: u32,
    busnum: u32,
    speed: u32,
}
impl Info {
    pub fn dev_num(&self) -> u32 {
        self.devnum
    }

    pub fn bus_num(&self) -> u32 {
        self.busnum
    }

    pub fn speed(&self) -> u32 {
        self.speed
    }

    pub fn dev_id(&self) -> u32 {
        (self.busnum << 16) | self.devnum
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ID {
    vendor: u16,
    product: u16,
}

impl ID {
    pub fn vendor(&self) -> u16 {
        self.vendor
    }

    pub fn product(&self) -> u16 {
        self.product
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Class {
    class: u8,
    subclass: u8,
    protocol: u8,
}

impl Class {
    pub fn class(&self) -> u8 {
        self.class
    }

    pub fn subclass(&self) -> u8 {
        self.subclass
    }

    pub fn protocol(&self) -> u8 {
        self.protocol
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbInterface {
    b_interface_class: u8,
    b_interface_subclass: u8,
    b_interface_protocol: u8,
    padding: __padding::Padding<u8>,
}

mod __padding {
    use std::marker::PhantomData;

    use serde::{de::Visitor, ser::SerializeTuple, Deserialize, Serialize};

    #[derive(Debug)]
    pub struct Padding<T>(PhantomData<T>);
    impl<T> Padding<T> {
        const SIZE: usize = std::mem::size_of::<T>();
    }

    impl<T> Serialize for Padding<T> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            let mut tup = serializer.serialize_tuple(Padding::<T>::SIZE)?;
            for _ in 0..Padding::<T>::SIZE {
                tup.serialize_element(&0x00_u8)?;
            }
            tup.end()
        }
    }

    impl<'de, T> Deserialize<'de> for Padding<T> {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            struct PaddingVisitor<T>(PhantomData<T>);
            impl<'de, T> Visitor<'de> for PaddingVisitor<T> {
                type Value = Padding<T>;

                fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                    write!(formatter, "{} byte(s)", Padding::<T>::SIZE)
                }

                fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
                where
                    A: serde::de::SeqAccess<'de>,
                {
                    for _ in 0..Padding::<T>::SIZE {
                        seq.next_element::<u8>()?;
                    }
                    Ok(Padding(PhantomData))
                }
            }

            deserializer.deserialize_tuple(Padding::<T>::SIZE, PaddingVisitor(PhantomData))
        }
    }
}
