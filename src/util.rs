pub mod __private {
    pub trait Sealed {}
}

use std::{ffi::c_char, str::FromStr};

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

pub const fn cast_cchar_to_u8(a: &[c_char]) -> &[u8] {
    // SAFETY: The slice is of type c_char, which can
    //         only be u8 (in which this cast does nothing)
    //         or i8. UTF-8 allows individual character bytes
    //         to be either a u8 or i8.
    unsafe { std::slice::from_raw_parts(a.as_ptr().cast::<u8>(), a.len()) }
}
