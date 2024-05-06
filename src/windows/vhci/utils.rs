use windows::{
    core::{GUID, PCWSTR},
    Win32::{
        Devices::DeviceAndDriverInstallation::{
            CM_Get_Device_Interface_ListW, CM_Get_Device_Interface_List_SizeW,
            CM_GET_DEVICE_INTERFACE_LIST_FLAGS, CR_BUFFER_SMALL, CR_SUCCESS,
        },
        Foundation::{ERROR_INVALID_PARAMETER, ERROR_NOT_ENOUGH_MEMORY},
    },
};

use super::Win32Error;

pub mod ioctl;
pub mod consts {
    pub const NI_MAXSERV: usize = 32;
    pub const NI_MAXHOST: usize = 1025;
}

pub fn get_device_interface_list<P>(
    guid: GUID,
    pdeviceid: P,
    flags: CM_GET_DEVICE_INTERFACE_LIST_FLAGS,
) -> Result<Vec<u16>, Win32Error>
where
    P: ::windows::core::IntoParam<PCWSTR> + Copy,
{
    let mut v = Vec::<u16>::new();
    loop {
        let mut cch = 0;
        let ret = unsafe {
            CM_Get_Device_Interface_List_SizeW(
                std::ptr::addr_of_mut!(cch),
                std::ptr::addr_of!(guid),
                pdeviceid,
                flags,
            )
        };
        if ret != CR_SUCCESS {
            break Err(Win32Error::from_cmret(ret, ERROR_INVALID_PARAMETER));
        }

        v.resize(cch as usize, 0);

        let ret = unsafe {
            CM_Get_Device_Interface_ListW(std::ptr::addr_of!(guid), pdeviceid, &mut v, flags)
        };
        match ret {
            CR_BUFFER_SMALL => continue,
            CR_SUCCESS => break Ok(v),
            err => break Err(Win32Error::from_cmret(err, ERROR_NOT_ENOUGH_MEMORY)),
        }
    }
}
