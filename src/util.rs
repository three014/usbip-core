pub mod __private {
    pub trait Sealed {}
}

use std::{ffi::c_char, str::FromStr};

/// Describes the encoded size of the object
/// when written to/read from a [`bincode`] buffer.
///
/// # Safety
///
/// Consumers of this trait must correctly report
/// the size of the object when encoded into/decoded
/// from [`bincode`]. Furthermore, the object's
/// encoded size must be known at compile time.
pub unsafe trait EncodedSize {
    const ENCODED_SIZE_OF: usize;
    const IS_ZERO_SIZED: bool = <Self as EncodedSize>::ENCODED_SIZE_OF == 0;
}


pub fn decode_zero_byte<D: bincode::de::Decoder>(decoder: &mut D) -> Result<(), bincode::error::DecodeError> {
    // Gotta make sure it's a null byte!
    use bincode::Decode;
    if u8::decode(decoder)? != 0u8 {
        Err(bincode::error::DecodeError::Other(
            "Nonzero value in the null-byte position",
        ))
    } else {
        Ok(())
    }
}

#[allow(dead_code)]
pub fn parse_token<'a, 'b: 'a, T>(
    tokens: &'a mut impl Iterator<Item = &'b str>,
) -> Result<T, T::Err>
where
    T: FromStr,
    T::Err: std::error::Error,
{
    tokens
        .next()
        .expect("There should be another item in the string stream")
        .trim()
        .parse()
}

pub fn into_dyn_err<T: std::error::Error + 'static>(err: T) -> Box<dyn std::error::Error> {
    Box::from(err)
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
