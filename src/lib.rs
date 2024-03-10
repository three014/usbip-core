use ffi::{SYSFS_BUS_ID_SIZE, SYSFS_PATH_MAX};
pub use libusbip_sys as ffi;
use serde::{Deserialize, Serialize};
use util::buffer::Buffer;

pub(crate) mod util {
    pub mod singleton {
        pub use error::Error;
        use std::sync::atomic::AtomicUsize;

        mod error {
            use std::fmt;

            /// The error type for the singleton module.
            /// Allows the `singleton::try_init` function
            /// to return an error of the user's choice
            /// should their initialization function fail.
            #[derive(Debug)]
            pub enum Error<E>
            where
                E: std::error::Error,
            {
                AlreadyInit,
                AlreadyFailed,
                UserSpecified(E),
            }

            impl<E> fmt::Display for Error<E>
            where
                E: std::error::Error,
            {
                fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    match self {
                        Error::AlreadyInit => write!(f, "singleton already initialized"),
                        Error::UserSpecified(user) => write!(f, "{user}"),
                        Error::AlreadyFailed => write!(f, "initialization had already failed"),
                    }
                }
            }

            impl<E> std::error::Error for Error<E> where E: std::error::Error {}
        }

        pub const UNINITIALIZED: usize = 0;
        pub const INITIALIZING: usize = 1;
        pub const INITIALIZED: usize = 2;
        pub const TERMINATING: usize = 3;
        pub const ERROR: usize = 4;

        /// Attempts to initialize the singleton using the
        /// provided `init` function, keeping synchronization
        /// with the `state` variable.
        ///
        /// On first init, `state` should be set to `singleton::UNINITIALIZED` or else
        /// `try_init` will never call `init` or worse, loop forever.
        pub fn try_init<F, T, E>(state: &AtomicUsize, init: F) -> Result<T, Error<E>>
        where
            F: FnOnce() -> Result<T, E>,
            E: std::error::Error,
        {
            use std::sync::atomic::Ordering;
            let old_state = match state.compare_exchange(
                UNINITIALIZED,
                INITIALIZING,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(s) | Err(s) => s,
            };

            match old_state {
                UNINITIALIZED => {
                    let value = init()
                        .inspect_err(|_| {
                            state.store(ERROR, Ordering::SeqCst);
                        })
                        .map_err(|err| Error::UserSpecified(err))?;
                    state.store(INITIALIZED, Ordering::SeqCst);
                    Ok(value)
                }
                INITIALIZING => {
                    while state.load(Ordering::SeqCst) == INITIALIZING {
                        std::hint::spin_loop();
                    }
                    Err(Error::AlreadyInit)
                }
                ERROR => Err(Error::AlreadyFailed),
                _ => Err(Error::AlreadyInit),
            }
        }
    }
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

pub mod vhci {
    use std::{io, path::Path, sync::atomic::AtomicUsize};

    pub use error::Error;
    use libusbip_sys::usbip_vhci_driver_open;

    use crate::util::singleton::{self, UNINITIALIZED};
    mod error {
        use std::fmt;

        #[derive(Debug, Clone, Copy)]
        pub enum Error {
            OpenFailed,
            NoFreePorts,
            ImportFailed,
            AlreadyOpen,
        }

        impl fmt::Display for Error {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self {
                    Error::OpenFailed => write!(f, "open vhci_driver failed (is vhci_hcd loaded?)"),
                    Error::NoFreePorts => write!(f, "no free ports"),
                    Error::ImportFailed => write!(f, "import device failed"),
                    Error::AlreadyOpen => write!(f, "already opened for this process"),
                }
            }
        }

        impl std::error::Error for Error {}
    }

    static STATE: AtomicUsize = AtomicUsize::new(UNINITIALIZED);
    pub struct VhciDriver;
    impl VhciDriver {
        pub fn try_open() -> Result<Self, Error> {
            let result = singleton::try_init(&STATE, || {
                let rc = unsafe { usbip_vhci_driver_open() };
                if rc < 0 {
                    Err(Error::OpenFailed)
                } else {
                    Ok(Self)
                }
            });

            result.map_err(|err| match err {
                singleton::Error::AlreadyInit => Error::AlreadyOpen,
                singleton::Error::UserSpecified(err) => err,
                singleton::Error::AlreadyFailed => Error::OpenFailed,
            })
        }

        pub fn open() -> Self {
            Self::try_open().unwrap()
        }
    }
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
