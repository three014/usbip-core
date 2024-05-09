pub mod __private {
    pub trait Sealed {}
}

use std::{ffi::c_char, str::FromStr};

pub unsafe trait EncodedSize {
    const ENCODED_SIZE_OF: usize;
}

#[allow(dead_code)]
pub fn parse_token<'a, 'b: 'a, T>(tokens: &'a mut impl Iterator<Item = &'b str>) -> T
where
    T: FromStr,
    T::Err: std::error::Error,
{
    tokens
        .next()
        .expect("There should be another item in the string stream")
        .trim()
        .parse()
        .expect("Token should be valid")
}

#[inline]
pub const fn cast_cchar_to_u8(a: &[c_char]) -> &[u8] {
    // SAFETY: The slice is of type c_char, which can
    //         only be u8 (in which this cast does nothing)
    //         or i8. UTF-8 allows individual character bytes
    //         to be either a u8 or i8.
    unsafe { std::slice::from_raw_parts(a.as_ptr().cast::<u8>(), a.len()) }
}

#[inline]
pub fn cast_cchar_to_u8_mut(a: &mut [c_char]) -> &mut [u8] {
    // SAFETY: The slice is of type c_char, which can
    //         only be u8 (in which this cast does nothing)
    //         or i8. UTF-8 allows individual character bytes
    //         to be either a u8 or i8.
    unsafe { std::slice::from_raw_parts_mut(a.as_mut_ptr().cast::<u8>(), a.len()) }
}
