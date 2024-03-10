use core::fmt;

use ffi::{SYSFS_BUS_ID_SIZE, SYSFS_PATH_MAX};
pub use libusbip_sys as ffi;
use serde::{Deserialize, Serialize};
use util::buffer::Buffer;

pub mod vhci;

pub(crate) mod util {
    pub mod singleton;

    pub mod buffer {
        use serde::{de::DeserializeOwned, Deserialize, Serialize};

        pub mod serde_helpers;

        #[repr(transparent)]
        #[derive(Serialize, Deserialize, Debug)]
        pub struct Buffer<const N: usize, T>(#[serde(with = "serde_helpers")] [T; N])
        where
            T: DeserializeOwned + Serialize;

        impl<const N: usize, T> From<[T; N]> for Buffer<N, T>
        where
            T: DeserializeOwned + Serialize,
        {
            fn from(value: [T; N]) -> Self {
                Self(value)
            }
        }
    }
}

pub mod names;

#[derive(Debug, Clone, Copy)]
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
            DeviceStatus::DevAvailable => write!(f, "device is available"),
            DeviceStatus::DevInUse => write!(f, "device is in use"),
            DeviceStatus::DevError => write!(f, "device is unusable because of a fatal error"),
            DeviceStatus::PortAvailable => write!(f, "port is available"),
            DeviceStatus::PortInitializing => write!(f, "port is initializing"),
            DeviceStatus::PortInUse => write!(f, "port is in use"),
            DeviceStatus::PortError => write!(f, "port error"),
        }
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

#[derive(Debug)]
pub struct Info {
    devnum: u32,
    busnum: u32,
    speed: u32,
}
impl Info {
    fn speed(&self) -> u32 {
        self.speed
    }

    fn devid(&self) -> u32 {
        (self.busnum << 16) | self.devnum
    }
}

#[derive(Debug)]
pub struct ID {
    vendor: u16,
    product: u16,
}

impl ID {
    fn vendor(&self) -> u16 {
        self.vendor
    }

    fn product(&self) -> u16 {
        self.product
    }
}

#[derive(Debug)]
pub struct Class {
    class: u8,
    subclass: u8,
    protocol: u8,
}

impl Class {
    fn class(&self) -> u8 {
        self.class
    }

    fn subclass(&self) -> u8 {
        self.subclass
    }

    fn protocol(&self) -> u8 {
        self.protocol
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbInterface {
    b_interface_class: u8,
    b_interface_subclass: u8,
    b_interface_protocol: u8,
    padding: __padding::Padding,
}

mod __padding {
    use serde::{de::Visitor, ser::SerializeTuple, Deserialize, Serialize};

    #[derive(Debug)]
    pub struct Padding;
    const PADDING_SIZE: usize = std::mem::size_of::<u8>();

    impl Serialize for Padding {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            let mut tup = serializer.serialize_tuple(PADDING_SIZE)?;
            for _ in 0..PADDING_SIZE {
                tup.serialize_element(&0x41_u8)?;
            }
            tup.end()
        }
    }

    impl<'de> Deserialize<'de> for Padding {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            struct PaddingVisitor;
            impl<'de> Visitor<'de> for PaddingVisitor {
                type Value = Padding;

                fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                    write!(formatter, "{} byte(s)", PADDING_SIZE)
                }

                fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
                where
                    A: serde::de::SeqAccess<'de>,
                {
                    for _ in 0..PADDING_SIZE {
                        seq.next_element::<u8>()?;
                    }
                    Ok(Padding)
                }
            }

            deserializer.deserialize_tuple(PADDING_SIZE, PaddingVisitor)
        }
    }
}
