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

use crate::windows::Win32Error;

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

/// Modified slightly from the `bytemuck` crate.
#[inline]
pub fn cast_u8_to_u16_slice(a: &[u8]) -> &[u16] {
    use core::mem::{align_of, size_of};
    // Note(Lokathor): everything with `align_of` and `size_of` will optimize away
    // after monomorphization.
    if align_of::<u16>() > align_of::<u8>()
        && !is_aligned_to(a.as_ptr() as *const (), align_of::<u16>())
    {
        panic!("Target alignment greater and input not aligned")
    } else if core::mem::size_of_val(a) % size_of::<u16>() == 0 {
        let new_len = core::mem::size_of_val(a) / size_of::<u16>();
        unsafe { core::slice::from_raw_parts(a.as_ptr() as *const u16, new_len) }
    } else {
        panic!("Output slice would have slop")
    }
}

/// Checks if `ptr` is aligned to an `align` memory boundary.
/// 
/// From the `bytemuck` crate.
///
/// ## Panics
/// * If `align` is not a power of two. This includes when `align` is zero.
#[inline]
fn is_aligned_to(ptr: *const (), align: usize) -> bool {
    // This is in a way better than `ptr as usize % align == 0`,
    // because casting a pointer to an integer has the side effect that it
    // exposes the pointer's provenance, which may theoretically inhibit
    // some compiler optimizations.
    ptr.align_offset(align) == 0
}
