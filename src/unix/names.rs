use std::{ffi::CString, io, os::unix::ffi::OsStrExt, path::Path, sync::atomic::AtomicUsize};

use crate::unix::ffi::{
    usbip_names_free, usbip_names_get_class, usbip_names_get_product, usbip_names_init,
};

use crate::{
    util::singleton::{self, UNINITIALIZED},
    unix::{Class, ID},
};

pub struct Names;
static STATE: AtomicUsize = AtomicUsize::new(UNINITIALIZED);

impl Names {
    pub fn try_init<P>(path: P) -> singleton::Result<Self, io::Error>
    where
        P: AsRef<Path>,
    {
        singleton::try_init(&STATE, || {
            let f = CString::new(path.as_ref().as_os_str().as_bytes())
                .unwrap_or_default()
                .into_boxed_c_str();
            // SAFETY: `usbip_names_init does not modify the
            // string, and the boxed string is dropped at the
            // end of the function`
            unsafe {
                let rc = usbip_names_init(f.as_ptr().cast_mut());
                if rc != 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(Self)
                }
            }
        })
    }

    pub fn read_class<B>(&self, mut buf: B, class: Class)
    where
        B: AsMut<[i8]>,
    {
        unsafe {
            usbip_names_get_class(
                buf.as_mut().as_mut_ptr(),
                buf.as_mut().len(),
                class.class(),
                class.subclass(),
                class.protocol(),
            );
        }
    }

    pub fn read_product<B>(&self, mut buf: B, id: ID)
    where
        B: AsMut<[i8]>,
    {
        unsafe {
            usbip_names_get_product(
                buf.as_mut().as_mut_ptr(),
                buf.as_mut().len(),
                id.vendor(),
                id.product(),
            );
        }
    }
}

impl Drop for Names {
    fn drop(&mut self) {
        singleton::terminate(&STATE, || unsafe { usbip_names_free() });
    }
}
