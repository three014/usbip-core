use core::fmt::{self, Write};
use std::{
    ffi::{c_char, OsStr},
    fmt::Arguments,
    ops::Deref,
    path::Path,
    str::Utf8Error,
};

use serde::{de::Error, Deserialize, Deserializer, Serialize};

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct StackStr<const N: usize> {
    #[serde(skip_serializing)]
    len: usize,
    #[serde(with = "crate::util::serde_helpers")]
    buf: [c_char; N],
}

impl<const N: usize> StackStr<N> {
    pub const fn new() -> Self {
        Self {
            len: 0,
            buf: [0; N],
        }
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub fn as_path(&self) -> &Path {
        Path::new(self.deref())
    }

    pub fn as_os_str(&self) -> &OsStr {
        OsStr::new(self.deref())
    }

    /// Sets all bytes in the array to 0,
    /// and sets `len` to 0 as well.
    pub fn clear(&mut self) {
        self.buf.fill(0);
        self.len = 0;
    }

    pub const unsafe fn from_raw_parts(buf: [c_char; N], len: usize) -> Self {
        Self { buf, len }
    }
}

impl<const N: usize> Deref for StackStr<N> {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        let slice = &self.buf[..self.len()];
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
            let len = self.len;
            for (idx, &byte) in s.as_bytes().iter().enumerate() {
                self.buf[idx + len] = byte as c_char;
            }
            self.len = s.len() + len;
            Ok(())
        }
    }
}

impl<'de, const N: usize> Deserialize<'de> for StackStr<N> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let buf: [c_char; N] = crate::util::serde_helpers::deserialize(deserializer)?;
        let u8_buf = crate::util::cast_cchar_to_u8(&buf);
        std::str::from_utf8(u8_buf).map_err(D::Error::custom)?;
        let len = buf
            .as_slice()
            .strip_suffix(&[0 as c_char])
            .unwrap_or(buf.as_slice())
            .len();

        // SAFETY: The entire array was checked to be a valid UTF-8 string,
        //         and the length was correctly calculated.
        Ok(unsafe { Self::from_raw_parts(buf, len) })
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
