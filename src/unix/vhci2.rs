#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddr};

    use crate::vhci::VhciDriver;

    use super::*;

    #[test]
    fn driver_opens() {
        UnixDriver::open().unwrap();
    }

    #[test]
    fn parse_record() {
        let record = str::parse::<PortRecord>("127.0.0.1 3240 1-1").unwrap();
        assert_eq!(
            record.host(),
            &SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 3240)
        );
        assert_eq!(record.bus_id(), "1-1");
    }
}

use core::fmt::{self, Write};
use std::{
    collections::HashMap,
    fs,
    io::{self, Write as IoWrite},
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
        stacktools::{self, StackStr},
    },
    util::parse_token,
    vhci::{base, HubSpeed},
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
                    base: base::ImportedDevice {
                        port,
                        status,
                        vendor: usb_dev.id_vendor,
                        product: usb_dev.id_product,
                        devid,
                    },
                    hub,
                    usb_dev,
                };

                Ok(MaybeIdev::InUse(idev))
            }
        }
    }
}

#[derive(Debug)]
pub enum PortRecordError {
    Buffer(stacktools::TryFromStrErr),
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

impl From<stacktools::TryFromStrErr> for PortRecordError {
    fn from(value: stacktools::TryFromStrErr) -> Self {
        Self::Buffer(value)
    }
}

pub struct PortRecord {
    base: base::PortRecord,
}

impl PortRecord {
    fn read(port: u16) -> Result<Self, PortRecordError> {
        let path = PathBuf::from(format!("{}/port{}", STATE_PATH, port));
        let s = fs::read_to_string(path)?;
        s.parse()
    }
}

impl Deref for PortRecord {
    type Target = base::PortRecord;

    fn deref(&self) -> &Self::Target {
        &self.base
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
            base: base::PortRecord {
                host: SocketAddr::new(host, srv_port),
                busid: busid.try_into()?,
            },
        })
    }
}

#[derive(Debug)]
pub struct UnixImportedDevice {
    base: base::ImportedDevice,
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
    type Target = base::ImportedDevice;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

struct UnixIdevDisplay<'a, 'b> {
    idev: &'a UnixImportedDevice,
    names: &'b crate::names::Names,
}

impl fmt::Display for UnixIdevDisplay<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let idev = &self.idev.base;
        let usb_dev = &self.idev.usb_dev;
        if idev.status() == DeviceStatus::PortInitializing
            || idev.status() == DeviceStatus::PortAvailable
        {
            return write!(f, "");
        }

        let record = PortRecord::read(idev.port()).inspect_err(|err| {
            writeln!(f, "Error when reading port record: {err}").unwrap();
        });

        writeln!(
            f,
            "Port {:02}: <{}> at {}",
            idev.port(),
            idev.status(),
            usb_dev.speed()
        )?;

        let product = self.names.product_display(idev.vendor(), idev.product());
        writeln!(f, "       {product}")?;

        match record {
            Ok(record) => {
                writeln!(
                    f,
                    "{:>10} -> usbip://{}/{}",
                    usb_dev.bus_id(),
                    record.host(),
                    record.bus_id()
                )?;
            }
            Err(_) => {
                writeln!(
                    f,
                    "{:>10} -> unknown host, remote port and remote busid",
                    usb_dev.bus_id()
                )?;
            }
        }

        writeln!(
            f,
            "{:10} -> remote bus/dev {:03}/{:03}",
            " ",
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

    fn activate(
        &mut self,
        port: u16,
        dev_id: u32,
        hub_speed: HubSpeed,
        bus_id: &str,
    ) -> Result<(), ActivateError> {
        let _ = self
            .available_mut()
            .remove(&port)
            .ok_or(ActivateError::PortNotAvailable)?;
        let udev = udev::Device::from_subsystem_sysname("usb".to_owned(), bus_id.to_owned())
            .expect("Creating udev of an already attached device");
        let usb_dev: crate::UsbDevice = udev
            .try_into()
            .expect("Converting valid udev into crate repr");
        self.active_mut().insert(
            port,
            UnixImportedDevice {
                base: base::ImportedDevice {
                    port,
                    status: DeviceStatus::PortInUse,
                    vendor: usb_dev.id_vendor,
                    product: usb_dev.id_product,
                    devid: dev_id,
                },
                usb_dev,
                hub: hub_speed,
            },
        );

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
                        .insert(idev.base.port(), idev);
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

    fn try_from(init: InitData) -> Result<Self, Self::Error> {
        let mut attr = StackStr::<20>::new();
        let mut idevs = Vec::with_capacity(init.num_ports.get());

        write!(attr, "status").unwrap();

        for i in 0..init.num_controllers.get() {
            if i > 0 {
                attr.clear();
                write!(attr, "status.{i}").unwrap();
            }

            let status = init.hc_device.sysattr(Beef::Borrowed(&attr))?;
            let mut lines = status.lines();
            lines.next();
            for line in lines {
                idevs.push(line.parse().map_err(Into::<UdevError>::into)?);
            }
        }
        Ok(idevs.into_iter().collect())
    }
}

impl crate::UsbDevice {
    pub fn attach_args(&self, socket: TcpStream) -> AttachArgs {
        AttachArgs {
            bus_id: self.bus_id(),
            socket,
            dev_id: self.dev_id(),
            device_speed: self.speed(),
        }
    }
}

pub struct UnixDriver {
    inner: InnerDriver,
}

impl UnixDriver {
    pub fn refresh_imported_devices(&mut self) -> crate::vhci::Result<()> {
        self.inner.refresh_imported_devices()
    }
}

struct InnerDriver {
    hc_device: udev::Device,
    imported_devices: IdevRecords,
    num_controllers: NonZeroUsize,
    num_ports: NonZeroUsize,
}

impl InnerDriver {
    fn try_open() -> crate::vhci::Result<Self> {
        let hc_device = udev::Device::from_subsystem_sysname(BUS_TYPE.into(), DEVICE_NAME.into())?;
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

    fn attach(&mut self, args: AttachArgs) -> Result<u16, crate::vhci::error::AttachError> {
        use crate::vhci::error::*;
        let AttachArgs {
            bus_id,
            socket,
            dev_id,
            device_speed,
        } = args;
        let port = match self.imported_devices.next_available(device_speed) {
            Some(port) => port,
            None => {
                return Err(AttachError {
                    socket,
                    kind: AttachErrorKind::OutOfPorts,
                })
            }
        };
        let path = self.udev().syspath();
        let path = StackStr::<SYSFS_PATH_MAX>::try_from(format_args!("{:?}/attach", path)).unwrap();
        let mut file = match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path.as_path())
        {
            Ok(file) => file,
            Err(e) => {
                return Err(AttachError {
                    socket,
                    kind: AttachErrorKind::SysFs(e),
                })
            }
        };

        let buf = StackStr::<200>::try_from(format_args!(
            "{} {} {} {}",
            port,
            socket.as_raw_fd(),
            dev_id,
            device_speed as u32
        ))
        .unwrap();

        if let Err(e) = file.write_all(buf.as_bytes()) {
            return Err(AttachError {
                socket,
                kind: AttachErrorKind::SysFs(e),
            });
        }

        self.imported_devices
            .activate(port, dev_id, device_speed.try_into().unwrap(), bus_id)
            .expect("Port should've been open");
        Ok(port)
    }
}

pub struct AttachArgs<'a> {
    pub socket: TcpStream,
    pub bus_id: &'a str,
    pub dev_id: u32,
    pub device_speed: DeviceSpeed,
}

impl crate::vhci::VhciDriver for UnixDriver {
    fn open() -> crate::vhci::Result<Self> {
        Ok(Self {
            inner: InnerDriver::try_open()?,
        })
    }

    fn detach(&mut self, port: u16) -> crate::vhci::Result<()> {
        todo!()
    }

    fn attach(&mut self, args: AttachArgs) -> Result<u16, crate::vhci::error::AttachError> {
        self.inner.attach(args)
    }

    fn imported_devices(&self) -> impl ExactSizeIterator<Item = &'_ UnixImportedDevice> + '_ {
        self.inner.imported_devices()
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
