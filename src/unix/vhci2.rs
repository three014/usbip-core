#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddr};

    use super::*;

    #[test]
    fn driver_opens() {
        Driver::open().unwrap();
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
mod sysfs {
    use crate::{unix::sysfs::SysAttr, DeviceSpeed};

    use std::{
        io::Write,
        os::fd::{AsRawFd, BorrowedFd},
    };

    pub fn detach(udev: &udev::Device, port: u16) -> std::io::Result<()> {
        let mut sys = SysAttr::open(udev.syspath().to_str().unwrap(), "detach")?;
        write!(sys, "{port}")
    }

    pub fn attach(udev: &udev::Device, new_connection: NewConnection) -> std::io::Result<()> {
        let mut sys = SysAttr::open(udev.syspath().to_str().unwrap(), "attach")?;
        let NewConnection {
            port,
            fd,
            dev_id,
            speed,
        } = new_connection;

        write!(
            sys,
            "{} {} {} {}",
            port,
            fd.as_raw_fd(),
            dev_id,
            speed as u32
        )
    }

    pub struct NewConnection<'a> {
        pub port: u16,
        pub fd: BorrowedFd<'a>,
        pub dev_id: u32,
        pub speed: DeviceSpeed,
    }
}

use core::fmt::{self, Write};
use std::{
    fs,
    io::{self, Write as IoWrite},
    net::{AddrParseError, IpAddr, SocketAddr},
    num::{NonZeroUsize, ParseIntError},
    ops::Deref,
    os::fd::AsFd,
    path::{Path, PathBuf},
    str::FromStr,
};

use crate::{
    containers::{
        beef::Beef,
        stacktools::{self, StackStr},
    },
    net::{OpCommon, OpImportReply, OpImportRequest, Protocol, Recv, Send, Status},
    unix::{net::UsbipStream, vhci2::sysfs::NewConnection},
    util::{__private::Sealed, parse_token},
    vhci::{base, error2::Error, AttachArgs, HubSpeed},
    DeviceSpeed, DeviceStatus,
};

use super::udev_utils::UdevExt;

pub static STATE_PATH: &str = "/var/run/vhci_hcd";
static BUS_TYPE: &str = "platform";
static DEVICE_NAME: &str = "vhci_hcd.0";

/// Used to allow parsing an `Option<AvailableIdev>`
/// from a string slice, since it isn't an error
/// to not recieve a full [`AvailableIdev`] object.
struct MaybeAvailableIdev(Option<AvailableIdev>);

impl Deref for MaybeAvailableIdev {
    type Target = Option<AvailableIdev>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl FromStr for MaybeAvailableIdev {
    type Err = Box<dyn std::error::Error>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut tokens = s.split_whitespace();
        let hub = parse_token::<HubSpeed>(&mut tokens)?;
        let port = parse_token::<u16>(&mut tokens)?;
        let status = parse_token::<DeviceStatus>(&mut tokens)?;
        if status == DeviceStatus::PortAvailable {
            Ok(MaybeAvailableIdev(Some(AvailableIdev {
                port,
                hub_speed: hub,
                _status: status,
            })))
        } else {
            Ok(MaybeAvailableIdev(None))
        }
    }
}

struct MaybeUnixImportedDevice(Option<UnixImportedDevice>);

impl Deref for MaybeUnixImportedDevice {
    type Target = Option<UnixImportedDevice>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl FromStr for MaybeUnixImportedDevice {
    type Err = Box<dyn std::error::Error>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut tokens = s.split_whitespace();
        let hub = parse_token::<HubSpeed>(&mut tokens)?;
        let port = parse_token::<u16>(&mut tokens)?;
        let status = parse_token::<DeviceStatus>(&mut tokens)?;
        if status == DeviceStatus::PortAvailable {
            return Ok(MaybeUnixImportedDevice(None));
        }

        let _speed = parse_token::<u32>(&mut tokens)?;
        let devid = parse_token::<u32>(&mut tokens)?;
        let _sockfd = parse_token::<u32>(&mut tokens)?;
        let busid = tokens.next().unwrap().trim();
        let sudev = udev::Device::from_subsystem_sysname("usb".to_owned(), busid.to_owned())?;
        let usb_dev = crate::UsbDevice::try_from(sudev).map_err(|err| err.into_custom_err())?;
        let idev = UnixImportedDevice {
            base: base::ImportedDevice {
                vendor: usb_dev.id_vendor,
                product: usb_dev.id_product,
                devid,
            },
            port,
            hub,
            usb_dev,
            status,
        };

        Ok(MaybeUnixImportedDevice(Some(idev)))
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
            PortRecordError::Buffer(b) => write!(f, "{b}"),
            PortRecordError::Io(i) => write!(f, "{i}"),
            PortRecordError::Addr(a) => write!(f, "{a}"),
            PortRecordError::Int(i) => write!(f, "{i}"),
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
pub struct UnixImportedDevices(Box<[UnixImportedDevice]>);

#[derive(Debug)]
pub struct UnixImportedDevice {
    base: base::ImportedDevice,
    port: u16,
    hub: HubSpeed,
    status: crate::DeviceStatus,
    usb_dev: crate::UsbDevice,
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

    pub const fn status(&self) -> DeviceStatus {
        self.status
    }

    pub const fn port(&self) -> u16 {
        self.port
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
        let idev = self.idev;
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

        let product = self
            .names
            .product_display(idev.base.vendor(), idev.base.product());
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
struct AvailableIdev {
    port: u16,
    hub_speed: HubSpeed,
    _status: DeviceStatus,
}

#[derive(Debug)]
struct OpenPorts(Vec<AvailableIdev>);

impl OpenPorts {
    fn get(&self) -> &[AvailableIdev] {
        &*self.0
    }

    fn get_mut(&mut self) -> &mut Vec<AvailableIdev> {
        &mut self.0
    }

    fn push(&mut self, port: AvailableIdev) {
        self.get_mut().push(port)
    }

    fn get_next(&mut self, speed: DeviceSpeed) -> Option<AvailableIdev> {
        self.get()
            .iter()
            .position(|port| speed == port.hub_speed.into())
            .map(|pos| self.get_mut().swap_remove(pos))
    }
}

struct InitData<'a> {
    hc_device: &'a udev::Device,
    num_controllers: NonZeroUsize,
    num_ports: NonZeroUsize,
}

impl From<InitData<'_>> for OpenPorts {
    fn from(init: InitData<'_>) -> Self {
        let mut attr = StackStr::<20>::try_from(format_args!("status")).unwrap();
        let mut open_ports = Vec::<AvailableIdev>::with_capacity(init.num_ports.get());

        for i in 0..init.num_controllers.get() {
            if i > 0 {
                attr.clear();
                write!(attr, "status.{i}").unwrap();
            }

            let status = init
                .hc_device
                .sysattr_str(&*attr)
                .expect("vhci udev should have this controller");
            for line in status.lines().skip(1) {
                let open_port = if let MaybeAvailableIdev(Some(open_port)) = line.parse().unwrap() {
                    open_port
                } else {
                    continue;
                };
                open_ports.push(open_port);
            }
        }

        OpenPorts(open_ports)
    }
}

impl From<InitData<'_>> for UnixImportedDevices {
    fn from(init: InitData) -> Self {
        let mut attr = StackStr::<20>::new();
        let mut idevs = Vec::new();

        write!(attr, "status").unwrap();

        for i in 0..init.num_controllers.get() {
            if i > 0 {
                attr.clear();
                write!(attr, "status.{i}").unwrap();
            }

            let status = init.hc_device.sysattr_str(&*attr).unwrap();
            for line in status.lines().skip(1) {
                let idev = if let MaybeUnixImportedDevice(Some(idev)) = line
                    .parse()
                    .expect("data came from udev and should have been valid")
                {
                    idev
                } else {
                    continue;
                };
                idevs.push(idev);
            }
        }
        UnixImportedDevices(idevs.into_boxed_slice())
    }
}

pub struct Driver {
    hc_device: udev::Device,
    open_ports: OpenPorts,
    num_controllers: NonZeroUsize,
    num_ports: NonZeroUsize,
}

impl Driver {
    pub fn open() -> crate::vhci::Result<Self> {
        let hc_device = udev::Device::from_subsystem_sysname(BUS_TYPE.into(), DEVICE_NAME.into())
            .map_err(|_| Error::DriverNotFound)?;
        let num_ports: NonZeroUsize = hc_device
            .sysattr("nports")
            .expect("udev should have this attribute");
        let num_controllers = num_controllers(&hc_device)?;
        let open_ports = InitData {
            hc_device: &hc_device,
            num_controllers,
            num_ports,
        }.into();

        Ok(Self {
            hc_device,
            open_ports,
            num_controllers,
            num_ports,
        })
    }

    #[inline(always)]
    const fn udev(&self) -> &udev::Device {
        &self.hc_device
    }

    #[inline(always)]
    const fn num_controllers(&self) -> NonZeroUsize {
        self.num_controllers
    }

    #[inline(always)]
    const fn num_ports(&self) -> NonZeroUsize {
        self.num_ports
    }

    #[inline(always)]
    fn open_ports_mut(&mut self) -> &mut OpenPorts {
        &mut self.open_ports
    }

    #[inline(always)]
    fn open_ports(&self) -> &OpenPorts {
        &self.open_ports
    }

    pub fn imported_devices(&self) -> crate::vhci::Result<UnixImportedDevices> {
        Ok(UnixImportedDevices::try_from(InitData {
            hc_device: self.udev(),
            num_controllers: self.num_controllers(),
            num_ports: self.num_ports(),
        })
        .expect(
            "if vhci driver is open, then driver is loaded, and should have all the information",
        ))
    }

    pub fn attach(&mut self, args: AttachArgs) -> crate::vhci::Result<u16> {
        let AttachArgs { host, bus_id } = args;

        let mut socket = UsbipStream::connect(&host)?;

        // Query host for USB info
        let req = OpCommon::request(Protocol::OP_REQ_IMPORT);
        socket.send(&req)?;

        let req = OpImportRequest::new(bus_id);
        socket.send(&req)?;

        let rep: OpCommon = socket.recv()?;
        assert_ne!(rep.validate(Protocol::OP_REP_IMPORT)?, Status::Unexpected);

        let rep: OpImportReply = socket.recv()?;
        let usb_dev = rep.into_inner();

        if usb_dev.bus_id() != bus_id {
            return Err(
                crate::net::Error::BusIdMismatch(Beef::Borrowed(usb_dev.bus_id()).into()).into(),
            );
        }

        // Find open port for attaching USB device
        let speed = usb_dev.speed();
        let dev_id = usb_dev.dev_id();

        let port = self
            .open_ports_mut()
            .get_next(speed)
            .ok_or(Error::NoFreePorts)?;

        sysfs::attach(
            self.udev(),
            NewConnection {
                port: port.port,
                fd: socket.as_fd(),
                dev_id,
                speed,
            },
        )
        .inspect_err(|_| self.open_ports_mut().push(port))?;

        // Record connection
        if let Err(err) = self.record_connection(port.port, socket.peer_addr()?, bus_id) {
            eprintln!("Failed to record new connection: {err}");
        }

        Ok(port.port)
    }

    fn record_connection(&self, port: u16, host: SocketAddr, bus_id: &str) -> std::io::Result<()> {
        create_state_path()?;

        let path = StackStr::<256>::try_from(format_args!("{}/port{}", STATE_PATH, port)).unwrap();
        let mut file = file_open(&*path)?;
        writeln!(file, "{} {}", host, bus_id)?;

        Ok(())
    }

    pub fn detach(&mut self, port: u16) -> crate::vhci::Result<()> {
        if self
            .open_ports()
            .get()
            .iter()
            .find(|open| open.port == port)
            .is_some()
        {
            return Ok(());
        }

        self.remove_connection(port);
        sysfs::detach(self.udev(), port)?;

        // TODO: Add some sort of way to add back the port

        Ok(())
    }

    fn remove_connection(&self, port: u16) {
        let path = StackStr::<200>::try_from(format_args!("{}/port{}", STATE_PATH, port)).unwrap();
        let _ = std::fs::remove_file(&*path);
    }
}

/// Creates the VHCI state path for persisting connection info,
/// returning if the directory already exists.
///
/// # Error
///
/// This function will return an error if the path already
/// exists and is NOT a directory, or if a general I/O error
/// has occurred.
fn create_state_path() -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    if let Err(err) = std::fs::DirBuilder::new().mode(0o700).create(STATE_PATH) {
        match err.kind() {
            std::io::ErrorKind::AlreadyExists
                if std::fs::metadata(STATE_PATH).is_ok_and(|s| s.is_dir()) => {}
            _ => Err(err)?,
        }
    }

    Ok(())
}

/// Opens a file with the preferred settings for VHCI state files.
fn file_open<P: AsRef<Path>>(path: P) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;

    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .truncate(true)
        .create(true)
        .mode(0o700)
        .open(path)
}

fn num_controllers(hc_device: &udev::Device) -> crate::vhci::Result<NonZeroUsize> {
    let platform = hc_device.parent().ok_or(Error::DriverNotFound)?;
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
        .map_err(|_| Error::NoFreePorts)?;
    Ok(count)
}

pub trait UnixVhciExt: Sealed {
    fn refresh_open_ports(&mut self);
}

impl Sealed for Driver {}
impl UnixVhciExt for Driver {
    fn refresh_open_ports(&mut self) {
        let open_ports = InitData {
            hc_device: self.udev(),
            num_controllers: self.num_controllers(),
            num_ports: self.num_ports(),
        }
        .try_into()
        .expect("parsing open port data from open udev context");

        self.open_ports = open_ports;
    }
}
