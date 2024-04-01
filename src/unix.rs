mod udev_helpers;
pub mod vhci2 {
    #[cfg(test)]
    mod tests {
        use crate::{unix::USB_IDS, vhci::VhciDriver};

        use super::*;

        #[test]
        fn driver_opens() {
            match UnixDriver::open() {
                Ok(d) => match crate::names::parse(USB_IDS) {
                    Ok(names) => {
                        d.imported_devices()
                            .for_each(|idev| println!("{}", idev.display(&names)));
                    }
                    Err(e) => eprintln!("Couldn't open names: {e}"),
                },
                Err(e) => println!("{e}"),
            }
        }

        #[test]
        fn parse_record() {
            let record = str::parse::<PortRecord>("127.0.0.1 3240 1-1").unwrap();
            println!(
                "Host: {}, busid: {}",
                record.inner.host,
                record.inner.busid.to_str().unwrap()
            );
        }
    }

    use core::fmt;
    use std::{
        collections::HashMap,
        ffi::c_char,
        fs,
        io::{self, Write},
        net::{AddrParseError, IpAddr, SocketAddr, TcpStream},
        num::{NonZeroUsize, ParseIntError},
        ops::Deref,
        os::fd::AsRawFd,
        path::PathBuf,
        str::FromStr,
    };

    use crate::{
        containers::{
            beef::Beef,
            buffer::{self, Buffer},
        },
        util::parse_token,
        vhci::{inner, HubSpeed},
        DeviceSpeed, DeviceStatus,
    };

    use super::{
        udev_helpers::{TryFromDeviceError, UdevHelper},
        UdevError,
    };

    pub static STATE_PATH: &str = "/var/run/vhci_hcd";
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
                        inner: inner::ImportedDevice {
                            port,
                            status,
                            vendor: usb_dev.id_vendor,
                            product: usb_dev.id_product,
                            devid,
                        },
                        hub,
                        usb_dev,
                    };

                    //debug_assert_eq!( dbg!(idev.inner.usb_dev().devnum), dbg!(idev.inner.dev_id() & 0x0000ffff) );
                    //debug_assert_eq!( dbg!(idev.inner.usb_dev().busnum), dbg!(idev.inner.dev_id() >> 16) );

                    Ok(MaybeIdev::InUse(idev))
                }
            }
        }
    }

    #[derive(Debug)]
    pub enum PortRecordError {
        Buffer(buffer::FormatError),
        Io(io::Error),
        Addr(AddrParseError),
        Int(ParseIntError),
        Invalid,
    }

    impl fmt::Display for PortRecordError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                PortRecordError::Buffer(b) => write!(f, "Buffer Format: {b}"),
                PortRecordError::Io(i) => write!(f, "I/O: {i}"),
                PortRecordError::Addr(a) => write!(f, "Address Parsing: {a}"),
                PortRecordError::Int(i) => write!(f, "Int Parsing: {i}"),
                PortRecordError::Invalid => write!(f, "Invalid port record"),
            }
        }
    }

    impl From<io::Error> for PortRecordError {
        fn from(value: io::Error) -> Self {
            Self::Io(value)
        }
    }

    impl From<AddrParseError> for PortRecordError {
        fn from(value: AddrParseError) -> Self {
            Self::Addr(value)
        }
    }

    impl From<ParseIntError> for PortRecordError {
        fn from(value: ParseIntError) -> Self {
            Self::Int(value)
        }
    }

    impl From<buffer::FormatError> for PortRecordError {
        fn from(value: buffer::FormatError) -> Self {
            Self::Buffer(value)
        }
    }

    pub struct PortRecord {
        inner: inner::PortRecord,
    }

    impl PortRecord {
        fn read(port: u16) -> Result<Self, PortRecordError> {
            let path = PathBuf::from(format!("{}/port{}", STATE_PATH, port));
            let s = fs::read_to_string(path)?;
            s.parse()
        }
    }

    impl Deref for PortRecord {
        type Target = inner::PortRecord;

        fn deref(&self) -> &Self::Target {
            &self.inner
        }
    }

    impl FromStr for PortRecord {
        type Err = PortRecordError;

        fn from_str(s: &str) -> Result<Self, Self::Err> {
            let mut split = s.split_whitespace();
            let host = split
                .next()
                .ok_or(PortRecordError::Invalid)?
                .parse::<IpAddr>()?;
            let srv_port = split
                .next()
                .ok_or(PortRecordError::Invalid)?
                .parse::<u16>()?;
            let busid = split.next().ok_or(PortRecordError::Invalid)?.trim();
            Ok(Self {
                inner: inner::PortRecord {
                    host: SocketAddr::new(host, srv_port),
                    busid: busid.try_into()?,
                },
            })
        }
    }

    #[derive(Debug)]
    pub struct UnixImportedDevice {
        inner: inner::ImportedDevice,
        usb_dev: crate::UsbDevice,
        hub: HubSpeed,
    }

    impl UnixImportedDevice {
        pub const fn display<'a: 'c, 'b: 'c, 'c>(
            &'a self,
            names: &'b crate::names::Names,
        ) -> impl fmt::Display + 'c {
            UnixIdevDisplay { idev: self, names }
        }

        pub const fn hub(&self) -> HubSpeed {
            self.hub
        }
    }

    impl Deref for UnixImportedDevice {
        type Target = inner::ImportedDevice;

        fn deref(&self) -> &Self::Target {
            &self.inner
        }
    }

    struct UnixIdevDisplay<'a, 'b> {
        idev: &'a UnixImportedDevice,
        names: &'b crate::names::Names,
    }

    impl fmt::Display for UnixIdevDisplay<'_, '_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            let idev = &self.idev.inner;
            let usb_dev = &self.idev.usb_dev;
            if idev.status() == DeviceStatus::PortInitializing
                || idev.status() == DeviceStatus::PortAvailable
            {
                return write!(f, "");
            }

            writeln!(
                f,
                "Port {:02}: <{}> at {}",
                idev.port(),
                idev.status(),
                usb_dev.speed()
            )?;

            let product = self
                .names
                .product(idev.vendor(), idev.product())
                .unwrap_or("unknown product");
            writeln!(f, "       {}", product)?;

            match PortRecord::read(idev.port()) {
                Ok(record) => {
                    writeln!(
                        f,
                        "       {:>10} -> usbip://{}/{}",
                        usb_dev.bus_id(),
                        record.host(),
                        record.bus_id()
                    )?;
                }
                Err(err) => {
                    writeln!(f, "Error parsing record: {err}")?;
                    writeln!(
                        f,
                        "       {:>10} -> unknown host, remote port and remote busid",
                        usb_dev.bus_id()
                    )?;
                }
            }

            writeln!(
                f,
                "           -> remote bus/dev {:03}/{:03}",
                usb_dev.dev_num(),
                usb_dev.bus_num()
            )
        }
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
    pub(crate) struct IdevRecords {
        active: HashMap<u16, UnixImportedDevice>,
        available: HashMap<u16, IdevSkeleton>,
    }

    impl IdevRecords {
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

    impl FromIterator<MaybeIdev> for IdevRecords {
        fn from_iter<T: IntoIterator<Item = MaybeIdev>>(iter: T) -> Self {
            let mut imported_devices = IdevRecords::new();
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

    impl TryFrom<InitData<'_>> for IdevRecords {
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

                let status = init
                    .hc_device
                    .sysattr(Beef::Borrowed(attr_buf.to_str().unwrap()))?;
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
        inner: inner::UsbId<'a>,
        dev_id: u32,
        speed: crate::DeviceSpeed,
    }

    impl UsbId<'_> {
        pub const fn dev_id(&self) -> u32 {
            self.dev_id
        }

        pub const fn speed(&self) -> crate::DeviceSpeed {
            self.speed
        }
    }

    impl<'a> Deref for UsbId<'a> {
        type Target = inner::UsbId<'a>;

        fn deref(&self) -> &Self::Target {
            &self.inner
        }
    }

    impl crate::UsbDevice {
        pub fn usb_id(&self) -> UsbId<'_> {
            UsbId {
                inner: inner::UsbId {
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

    impl UnixDriver {
        pub fn refresh_imported_devices(&mut self) -> crate::vhci::Result<()> {
            self.inner.refresh_imported_devices()
        }

        pub fn imported_devices(
            &self,
        ) -> impl ExactSizeIterator<Item = &'_ UnixImportedDevice> + '_ {
            self.inner.imported_devices()
        }
    }

    struct DriverInner {
        hc_device: udev::Device,
        imported_devices: IdevRecords,
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
            let path = self.udev().syspath();
            write!(path_buf.as_mut_u8_bytes(), "{:?}/attach", path).unwrap();
            let mut file = match std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path_buf.to_str().unwrap())
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
            if let Err(e) = file.write_all(
                buf.as_u8_bytes()
                    .strip_suffix(&[0u8])
                    .unwrap_or(buf.as_u8_bytes()),
            ) {
                return Err(Error::AttachFailed(AttachError {
                    socket,
                    kind: AttachErrorKind::SysFs(e),
                }));
            }

            self.imported_devices
                .activate(port, |_| {
                    let udev = udev::Device::from_subsystem_sysname(
                        "usb".to_owned(),
                        usb_id.bus_id().to_owned(),
                    )
                    .expect("Creating repr of device already attached");
                    let usb_dev: crate::UsbDevice = udev.try_into().unwrap();
                    UnixImportedDevice {
                        inner: inner::ImportedDevice {
                            port,
                            status: DeviceStatus::PortInUse,
                            vendor: usb_dev.id_vendor,
                            product: usb_dev.id_product,
                            devid: usb_id.dev_id(),
                        },
                        usb_dev,
                        hub: usb_id.speed().try_into().unwrap(),
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
            .filter(|result| {
                result.as_ref().is_ok_and(|entry| {
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
    containers::{
        beef::Beef,
        buffer::{Buffer, FormatError},
    },
    unix::udev_helpers::UdevHelper,
    BUS_ID_SIZE, DEV_PATH_MAX,
};
use std::{
    borrow::Cow,
    ffi::{c_char, OsStr},
    os::unix::ffi::OsStrExt,
    path::Path,
};
use udev;
pub use udev_helpers::Error as UdevError;

use udev_helpers::ParseAttributeError;

pub static USB_IDS: &str = "/usr/share/hwdata/usb.ids";

impl<const N: usize> TryFrom<&OsStr> for Buffer<N, c_char> {
    type Error = FormatError;

    fn try_from(value: &OsStr) -> Result<Self, Self::Error> {
        value.as_bytes().try_into()
    }
}

impl<const N: usize> TryFrom<&Path> for Buffer<N, c_char> {
    type Error = FormatError;

    fn try_from(value: &Path) -> Result<Self, Self::Error> {
        value.as_os_str().try_into()
    }
}

impl TryFrom<udev::Device> for crate::UsbDevice {
    type Error = ParseAttributeError;

    fn try_from(udev: udev::Device) -> Result<Self, Self::Error> {
        let path: Buffer<DEV_PATH_MAX, c_char> = udev.syspath().try_into()?;
        let busid: Buffer<BUS_ID_SIZE, c_char> = udev.sysname().try_into()?;
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
