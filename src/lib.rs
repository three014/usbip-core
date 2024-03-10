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

#[derive(Debug)]
pub struct Class {
    class: u8,
    subclass: u8,
    protocol: u8,
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
