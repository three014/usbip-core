use windows::Win32::{
    Devices::DeviceAndDriverInstallation::{CM_MapCrToWin32Err, CONFIGRET},
    Foundation::WIN32_ERROR,
};

pub mod vhci {
    use std::{
        ffi::OsString, fs::File, io::Read, net::{SocketAddr, ToSocketAddrs}, ops::Deref, os::windows::{
            ffi::OsStringExt,
            fs::OpenOptionsExt,
            io::{AsHandle, BorrowedHandle},
        }, path::PathBuf
    };

    use bincode::error::DecodeError;
    use windows::{
        core::{GUID, PCWSTR},
        Win32::{
            Devices::DeviceAndDriverInstallation::CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
            Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE},
        },
    };

    use crate::{
        vhci::base,
        windows::vhci::utils::ioctl,
    };

    use super::Win32Error;

    mod utils;

    pub static STATE_PATH: &str = "";
    const GUID_DEVINTERFACE_USB_HOST_CONTROLLER: GUID = GUID::from_values(
        0xB4030C06,
        0xDC5F,
        0x4FCC,
        [0x87, 0xEB, 0xE5, 0x51, 0x5A, 0x09, 0x35, 0xC0],
    );

    #[derive(Debug)]
    pub struct AttachArgs<'a> {
        pub host: SocketAddr,
        pub bus_id: &'a str,
    }

    #[derive(Debug)]
    pub struct PortRecord {
        base: base::PortRecord,
    }

    #[derive(Debug)]
    pub struct WindowsImportedDevice {
        base: base::ImportedDevice,
        record: PortRecord,
        speed: crate::DeviceSpeed,
    }

    impl From<ioctl::ImportedDevice> for WindowsImportedDevice {
        fn from(value: ioctl::ImportedDevice) -> Self {
            let host = (value.host.deref(), value.service.parse().unwrap());
            Self {
                base: base::ImportedDevice {
                    port: value.port as u16,
                    vendor: value.vendor,
                    product: value.product,
                    devid: value.devid,
                },
                record: PortRecord {
                    base: base::PortRecord {
                        busid: value.busid,
                        host: host.to_socket_addrs().unwrap().next().unwrap(),
                    },
                },
                speed: value.speed,
            }
        }
    }

    #[derive(Debug)]
    pub struct WindowsImportedDevices(Box<[WindowsImportedDevice]>);

    impl WindowsImportedDevices {
        pub fn iter(&self) -> core::slice::Iter<'_, WindowsImportedDevice> {
            self.get().iter()
        }

        pub fn get(&self) -> &[WindowsImportedDevice] {
            &self.0
        }
    }

    impl bincode::Decode for WindowsImportedDevices {
        fn decode<D: bincode::de::Decoder>(decoder: &mut D) -> Result<Self, DecodeError> {
            let len = u32::decode(decoder)? as usize;

            // We use a byte array that's the same size as the
            // encoded struct to not leak the struct definition
            decoder.claim_container_read::<[u8; ioctl::ImportedDevice::encoded_size_of()]>(len)?;

            let mut vec = Vec::with_capacity(len);
            for _ in 0..len {
                decoder.unclaim_bytes_read(ioctl::ImportedDevice::encoded_size_of());

                let ioctldev = ioctl::ImportedDevice::decode(decoder)?;
                let imported_device = WindowsImportedDevice::from(ioctldev);
                vec.push(imported_device);
            }

            Ok(WindowsImportedDevices(vec.into_boxed_slice()))
        }
    }

    struct InnerDriver {
        handle: File,
    }

    impl InnerDriver {
        fn as_handle(&self) -> BorrowedHandle {
            self.handle.as_handle()
        }

        fn try_open() -> crate::vhci::Result<Self> {
            let file = File::options()
                .create(true)
                .read(true)
                .write(true)
                .attributes((FILE_SHARE_READ | FILE_SHARE_WRITE).0)
                .open(Self::path().inspect(|path| println!("Driver path: {}", path.display()))?)?;

            Ok(Self { handle: file })
        }

        fn imported_devices(&self) -> Result<WindowsImportedDevices, bincode::error::DecodeError> {
            let mut reader = ioctl::Reader::<{ ioctl::ImportedDevice::encoded_size_of() }>::new(
                self.as_handle(),
                ioctl::DeviceType::Unknown,
                ioctl::Function::GetImportedDevices.as_u32(),
            );

            let mut buf = Vec::new();
            reader
                .read_to_end(&mut buf)
                .map_err(|err| bincode::error::DecodeError::Io {
                    inner: err,
                    additional: 0,
                })?;

            let config = crate::net::bincode_config().with_little_endian();
            let idevs: WindowsImportedDevices = bincode::decode_from_slice(&buf, config)?.0;
            Ok(idevs)
        }

        fn path() -> crate::vhci::Result<PathBuf> {
            let v = utils::get_device_interface_list(
                GUID_DEVINTERFACE_USB_HOST_CONTROLLER,
                PCWSTR::null(),
                CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
            )?;
            let mut p = v.split(|&elm| elm == 0).filter(|slice| !slice.is_empty());
            if let Some(path) = p.next() {
                if p.next().is_some() {
                    // We add 2 because of the first slice and
                    // this second slice we just found.
                    Err(crate::vhci::Error::MultipleDevInterfaces(2 + p.count()))
                } else {
                    Ok(PathBuf::from(OsString::from_wide(path)))
                }
            } else {
                Err(std::io::Error::from(std::io::ErrorKind::NotFound).into())
            }
        }
    }

    pub struct WindowsVhciDriver {
        inner: InnerDriver,
    }

    impl WindowsVhciDriver {
        #[inline(always)]
        pub fn open() -> crate::vhci::Result<Self> {
            Ok(Self {
                inner: InnerDriver::try_open()?,
            })
        }

        pub fn attach(&mut self, args: AttachArgs) -> Result<u16, crate::vhci::error::AttachError> {
            todo!()
        }

        pub fn detach(&mut self, port: u16) -> crate::vhci::Result<()> {
            todo!()
        }

        pub fn imported_devices(&self) -> crate::vhci::Result<WindowsImportedDevices> {
            Ok(self.inner.imported_devices().map_err(|err| match err {
                DecodeError::Io { inner, .. } => inner,
                _ => std::io::Error::other(err),
            })?)
        }
    }

    impl From<Win32Error> for crate::vhci::Error {
        fn from(value: Win32Error) -> Self {
            Self::Windows(value.into())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn driver_can_open() {
            WindowsVhciDriver::open().unwrap();
        }

        #[test]
        fn imported_devices_doesnt_die() {
            let driver = WindowsVhciDriver::open().unwrap();

            driver.imported_devices().unwrap();
        }
    }
}

pub static USB_IDS: &str = "";

struct Win32Error(WIN32_ERROR);

impl Win32Error {
    pub fn get(self) -> WIN32_ERROR {
        self.0
    }

    pub fn from_cmret(cm_ret: CONFIGRET, default_err: WIN32_ERROR) -> Self {
        let code = unsafe { CM_MapCrToWin32Err(cm_ret, default_err.0) };
        Self(WIN32_ERROR(code))
    }
}

impl From<Win32Error> for ::windows::core::Error {
    fn from(value: Win32Error) -> Self {
        ::windows::core::Error::from(value.get())
    }
}
