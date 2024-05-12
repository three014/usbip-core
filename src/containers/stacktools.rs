use core::fmt::{self, Write};
use std::{
    ffi::{c_char, OsStr},
    fmt::Arguments,
    ops::Deref,
    path::Path,
    str::Utf8Error,
};


/// A UTF-8 encoded string, but stored entirely on the stack.
/// 
/// # Examples
/// 
/// You can create a [`StackStr`] with [`StackStr::try_from`]:
/// 
/// [`StackStr::try_from`]: TryFrom::try_from
/// 
/// ```
/// use usbip_core::containers::stacktools::StackStr;
/// 
/// // We know the string is less than 32 bytes, so we'll use `unwrap()`.
/// let hello = StackStr::<32>::try_from("Hello, world!").unwrap();
/// ```
/// 
/// [`StackStr`] implements the [`Write`] trait, so you
/// can use it as a cool stack-allocated buffer.
///
/// ```
/// use usbip_core::containers::stacktools::StackStr;
/// use core::fmt::Write;
/// 
/// let mut hello = StackStr::<256>::new();
/// 
/// write!(&mut hello, "Hello, world!").unwrap();
/// ```
/// 
/// # Deref
/// 
/// `StackStr` implements <code>[Deref]<Target = [str]></code>, and
/// so inherits all of [`str`]'s methods. In addition,
/// this means you can pass a `StackStr` to a function
/// which takes a [`&str`] by using an ampersand (`&`):
/// 
/// ```
/// use usbip_core::containers::stacktools::StackStr;
/// 
/// fn takes_str(s: &str) { }
/// 
/// let s = StackStr::<32>::try_from("Hello").unwrap();
/// 
/// takes_str(&s);
/// 
/// ```
#[derive(Debug, PartialEq, Eq)]
pub struct StackStr<const N: usize> {
    len: usize,
    buf: [c_char; N],
}

impl<const N: usize> StackStr<N> {
    /// Creates a new string slice on
    /// the stack with a zeroed buffer of size `N`.
    #[inline(always)]
    pub const fn new() -> Self {
        Self {
            len: 0,
            buf: [0; N],
        }
    }

    /// Converts a [`StackStr`] to a [`Path`].
    pub fn as_path(&self) -> &Path {
        Path::new(self.deref())
    }

    /// Converts a [`StackStr`] into an [`OsStr`].
    #[inline]
    pub fn as_os_str(&self) -> &OsStr {
        OsStr::new(self.deref())
    }

    /// Sets the length of `self` to `0` without
    /// modifying the internal buffer.
    pub fn clear(&mut self) {
        self.fill(0);
        self.len = 0;
    }

    /// Fills `self` with elements by copying `value`.
    /// Only fills the bytes from `0..self.len`.
    pub fn fill(&mut self, value: c_char) {
        let len = self.len;
        self.buf[0..len].fill(value);
    }

    /// Form a [`StackStr`] from an array and a length.
    /// 
    /// The `len` argument is the number of bytes.
    /// 
    /// # SAFETY
    /// 
    /// `buf` MUST be a valid UTF-8 slice.
    #[inline(always)]
    pub const unsafe fn from_raw_parts(buf: [c_char; N], len: usize) -> Self {
        Self { buf, len }
    }
}

impl<const N: usize> Deref for StackStr<N> {
    type Target = str;

    #[inline]
    fn deref(&self) -> &Self::Target {
        let slice = &self.buf[..self.len];
        let slice = crate::util::cast_cchar_to_u8(slice);
        // SAFETY: `StackStr` is always instantiated
        //         with valid UTF-8, and cannot be constructed
        //         any other way.
        unsafe { std::str::from_utf8_unchecked(slice) }
    }
}

impl<const N: usize> TryFrom<Arguments<'_>> for StackStr<N> {
    type Error = TryFromStrErr;

    fn try_from(value: Arguments<'_>) -> Result<Self, Self::Error> {
        let mut stack_s = StackStr::new();
        stack_s
            .write_fmt(value)
            .map_err(|_| TryFromStrErr::Length {
                max: N,
                actual: usize::MAX,
            })?;
        Ok(stack_s)
    }
}

impl<const N: usize> TryFrom<&str> for StackStr<N> {
    type Error = TryFromStrErr;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let mut stack_s = StackStr::<N>::new();
        write!(stack_s, "{value}").map_err(|_| TryFromStrErr::Length {
            max: N,
            actual: value.len(),
        })?;
        Ok(stack_s)
    }
}

impl<const N: usize> fmt::Display for StackStr<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.deref().fmt(f)
    }
}

impl<const N: usize> Write for StackStr<N> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if s.len() > N - self.len() {
            Err(fmt::Error::default())
        } else {
            let len = self.len();
            let u8_buf = crate::util::cast_cchar_to_u8_mut(&mut self.buf);
            u8_buf[len..len + s.len()].copy_from_slice(s.as_bytes());
            self.len += s.len();
            Ok(())
        }
    }
}

impl<const N: usize> bincode::Decode for StackStr<N> {
    fn decode<D: bincode::de::Decoder>(
        decoder: &mut D,
    ) -> Result<Self, bincode::error::DecodeError> {
        let (buf, len) = decode_and_validate(decoder)?;

        // SAFETY: The entire array was checked to be a valid UTF-8 string,
        //         and the length was correctly calculated.
        Ok(unsafe { Self::from_raw_parts(buf, len) })
    }
}

impl<'de, const N: usize> bincode::BorrowDecode<'de> for StackStr<N> {
    fn borrow_decode<D: bincode::de::BorrowDecoder<'de>>(
        decoder: &mut D,
    ) -> Result<Self, bincode::error::DecodeError> {
        let (buf, len) = decode_and_validate(decoder)?;

        // SAFETY: The entire array was checked to be a valid UTF-8 string,
        //         and the length was correctly calculated.
        Ok(unsafe { Self::from_raw_parts(buf, len) })
    }
}

#[inline(always)]
fn decode_and_validate<D: bincode::de::Decoder, const N: usize>(
    decoder: &mut D,
) -> Result<([c_char; N], usize), bincode::error::DecodeError> {
    let buf: [c_char; N] = bincode::Decode::decode(decoder)?;

    let u8_buf = crate::util::cast_cchar_to_u8(&buf[0..N]);
    let len = std::str::from_utf8(u8_buf)
        .map_err(|err| bincode::error::DecodeError::Utf8 { inner: err })?
        // What happens if the start of the string has a buncha null bytes?
        //.trim_start_matches(char::from(0u8))
        .trim_end_matches(char::from(0u8))
        .len();

    Ok((buf, len))
}

impl<const N: usize> bincode::Encode for StackStr<N> {
    fn encode<E: bincode::enc::Encoder>(
        &self,
        encoder: &mut E,
    ) -> Result<(), bincode::error::EncodeError> {
        bincode::Encode::encode(&self.buf, encoder)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum TryFromStrErr {
    Length { max: usize, actual: usize },
    NotUtf8(Utf8Error),
}

impl fmt::Display for TryFromStrErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TryFromStrErr::Length { max, actual } => write!(
                f,
                "invalid length of str (max: {}, actual: {})",
                max, actual
            ),
            TryFromStrErr::NotUtf8(err) => write!(f, "{err}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn valid_len_try_from_str_works() {
        let str = "Hello!";
        let result = StackStr::<{ "Hello!".as_bytes().len() }>::try_from(str);
        let stack_s = result.unwrap();
        println!("{stack_s}");
    }

    #[test]
    fn invalid_len_fails() {
        let s = "I am a big string!";
        let result = StackStr::<4>::try_from(s);
        assert_eq!(
            result,
            Err(TryFromStrErr::Length {
                max: 4,
                actual: s.len()
            })
        );
    }

    #[test]
    fn valid_conversion_has_correct_len() {
        let s = "Super cool string!";
        let stack_s = StackStr::<56>::try_from(s).unwrap();
        assert_eq!(stack_s.len(), s.len());
    }

    #[test]
    fn convert_from_format_args() {
        let mexico = "Mexico";
        let s = StackStr::<256>::try_from(format_args!("Hello from {}!", mexico));
        assert_eq!(
            s,
            Ok(StackStr::<256>::try_from("Hello from Mexico!").unwrap())
        )
    }
}
