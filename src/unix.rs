pub mod names;
mod udev_helpers;
pub mod vhci;
pub mod vhci2 {
    use std::{borrow::Cow, fmt::Write, num::NonZeroUsize, str::FromStr};

    static BUS_TYPE: &str = "platform";
    static DEVICE_NAME: &str = "vhci_hcd.0";

    impl FromStr for crate::vhci::ImportedDevice {
        type Err = crate::vhci::Error;

        fn from_str(s: &str) -> Result<Self, Self::Err> {
            let mut tokens = s.splitn(7, ' ');
            let hub: crate::vhci::HubSpeed = crate::util::get_token(&mut tokens);
            let port: u16 = crate::util::get_token(&mut tokens);
            let status: crate::DeviceStatus = crate::util::get_token(&mut tokens);
            let _speed: u32 = crate::util::get_token(&mut tokens);
            let dev_id: u32 = crate::util::get_token(&mut tokens);
            let _sockfd: u32 = crate::util::get_token(&mut tokens);
            let bus_id = tokens.next().unwrap().trim();

            // TODO: We need the udev context to get usbdevice details

            todo!()
        }
    }

    #[derive(Debug)]
    pub struct ImportedDevices(Box<[crate::vhci::ImportedDevice]>);

    pub struct Driver {
        inner: DriverInner,
    }

    struct DriverInner {
        hc_device: udev::Device,
        imported_devices: ImportedDevices,
    }

    impl DriverInner {
        fn try_open() -> crate::vhci::Result<Self> {
            use crate::unix::udev_helpers::{get_sysattr, get_sysattr_clone_err};

            let context = udev::Udev::new()?;
            let hc_device = udev::Device::from_subsystem_sysname_with_context(
                context.clone(),
                BUS_TYPE.into(),
                DEVICE_NAME.into(),
            )?;
            let num_ports: NonZeroUsize = get_sysattr(&hc_device, Cow::Borrowed("nports"))?
                .parse()
                .expect("Parsing num_ports from str");
            let num_controllers = num_controllers(&hc_device)?;

            let mut attr_buf = String::new();
            let mut idevs = Vec::with_capacity(num_ports.get());

            write!(&mut attr_buf, "status").unwrap();

            for i in 0..num_controllers.get() {
                if i > 0 {
                    attr_buf.clear();
                    write!(&mut attr_buf, "status.{i}").unwrap();
                }
                let attr = get_sysattr_clone_err(&hc_device, &attr_buf)?;

                let mut lines = attr.lines();
                lines.next();
                for line in lines {
                    let idev = line.parse()?;
                    idevs.push(idev);
                }
            }

            Ok(Self {
                hc_device,
                imported_devices: ImportedDevices(idevs.into_boxed_slice()),
            })
        }
    }

    impl crate::vhci::VhciDriver for Driver {
        fn open() -> crate::vhci::Result<Self> {
            Ok(Self {
                inner: DriverInner::try_open()?,
            })
        }

        fn detach(&self, port: u16) -> crate::vhci::Result<()> {
            todo!()
        }

        fn imported_devices(&self) -> crate::vhci::Result<&[crate::vhci::ImportedDevice]> {
            todo!()
        }

        fn attach(&self, socket: std::net::SocketAddr, bus_id: &str) -> crate::vhci::Result<u16> {
            todo!()
        }
    }

    fn num_controllers(hc_device: &udev::Device) -> crate::vhci::Result<NonZeroUsize> {
        use super::UdevError;
        use crate::vhci::Error;

        let platform = hc_device
            .parent()
            .ok_or_else(|| Error::Udev(UdevError::NoParent))?;

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

    impl crate::util::__private::Sealed for Driver {}
}

use crate::{
    net::Status,
    util::buffer::{Buffer, FormatError},
    DeviceStatus,
};
pub use ffi::{SYSFS_BUS_ID_SIZE, SYSFS_PATH_MAX};
pub use libusbip_sys::unix as ffi;
use serde::{Deserialize, Serialize};
use std::{ffi::OsStr, io, os::unix::ffi::OsStrExt, path::Path, str::FromStr};
pub use udev;
pub use udev_helpers::Error as UdevError;

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
