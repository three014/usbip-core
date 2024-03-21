use core::fmt;
use serde::{Serialize, Deserialize};
pub use util::buffer;
use util::buffer::Buffer;

#[cfg(target_family = "unix")]
pub mod unix;

pub mod names;

mod util {
    pub mod buffer;
    pub mod singleton;

    pub fn cast_u8_to_i8_slice(a: &[u8]) -> &[i8] {
        unsafe { std::slice::from_raw_parts(a.as_ptr().cast::<i8>(), a.len()) }
    }

    pub fn _cast_i8_to_u8_slice(a: &[i8]) -> &[u8] {
        unsafe { std::slice::from_raw_parts(a.as_ptr().cast::<u8>(), a.len()) }
    }
}

pub mod net {
    use core::fmt;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Copy, Serialize, Deserialize)]
    #[repr(u32)]
    pub enum Status {
        Success = 0x00,
        Failed = 0x01,
        DevBusy = 0x02,
        DevErr = 0x03,
        NoDev = 0x04,
        Unexpected = 0x05,
    }

    impl fmt::Display for Status {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Status::Success => write!(f, "Request succeeded"),
                Status::Failed => write!(f, "Request failed"),
                Status::DevBusy => write!(f, "Device busy (exported)"),
                Status::DevErr => write!(f, "Device in error state"),
                Status::NoDev => write!(f, "Device not found"),
                Status::Unexpected => write!(f, "Unexpected response"),
            }
        }
    }
}

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


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbDevice {

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

pub mod vhci {
    use std::net::SocketAddr;

    pub use error::Error;

    use crate::DeviceStatus;

    pub type Result<T> = std::result::Result<T, Error>;

    mod error {
        use std::io;

        #[derive(Debug)]
        pub enum Error {
            IO(io::Error),
        }
    }

    #[derive(Debug, Clone, Copy)]
    pub enum HubSpeed {
        High = 0,
        Super,
    }

    #[derive(Debug)]
    pub struct ImportedDevice {
        hub: HubSpeed,
        port: u16,
        status: DeviceStatus,
        vendor: u16,
        product: u16,
        dev_id: u32
    }

    impl ImportedDevice {
        pub const fn hub(&self) -> HubSpeed {
            self.hub
        }

        pub const fn port(&self) -> u16 {
            self.port
        }

        pub const fn status(&self) -> DeviceStatus {
            self.status
        }

        pub const fn vendor(&self) -> u16 {
            self.vendor
        }

        pub const fn dev_id(&self) -> u32 {
            self.dev_id
        }

        pub const fn product(&self) -> u16 {
            self.product
        }
    }

    pub trait VhciDriver: Sized {
        fn open() -> Result<Self>;
        fn attach(&self, socket: SocketAddr, bus_id: &str) -> Result<u16>;
        fn detach(&self, port: u16) -> Result<()>;
        fn imported_devices(&self) -> Result<&[ImportedDevice]>;
    }
}
