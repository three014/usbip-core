mod udev_helpers;
pub mod vhci2;

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
