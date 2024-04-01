use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{ffi::c_char, fmt, str::Utf8Error, string::FromUtf8Error};

pub mod serde_helpers;

fn cast_cchar_to_u8<T>(a: &[T]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(a.as_ptr().cast::<u8>(), a.len()) }
}

fn cast_cchar_to_u8_mut<T>(a: &mut [T]) -> &mut [u8] {
    unsafe { std::slice::from_raw_parts_mut(a.as_mut_ptr().cast::<u8>(), a.len()) }
}

fn cast_u8_to_cchar(a: &[u8]) -> &[c_char] {
    unsafe { std::slice::from_raw_parts(a.as_ptr().cast::<c_char>(), a.len()) }
}

#[repr(transparent)]
#[derive(Serialize, Deserialize, Debug)]
pub struct Buffer<const N: usize, T>(#[serde(with = "serde_helpers")] [T; N])
where
    T: DeserializeOwned + Serialize;

impl<const N: usize> Buffer<N, c_char> {
    pub fn new() -> Buffer<N, c_char> {
        Buffer([0 as c_char; N])
    }

    pub fn as_mut_slice(&mut self) -> &mut [c_char] {
        &mut self.0
    }

    pub fn as_slice(&self) -> &[c_char] {
        &self.0
    }

    pub fn to_str(&self) -> Result<&str, Utf8Error> {
        let slice = self.as_u8_bytes();
        std::str::from_utf8(slice).map(|s| s.trim().trim_matches(char::from(0)))
    }

    pub fn to_mut_str(&mut self) -> Result<&mut str, Utf8Error> {
        let mut_slice = self.as_mut_u8_bytes();
        std::str::from_utf8_mut(mut_slice)
    }

    pub fn as_u8_bytes(&self) -> &[u8] {
        cast_cchar_to_u8(self.as_slice())
    }

    pub fn as_mut_u8_bytes(&mut self) -> &mut [u8] {
        cast_cchar_to_u8_mut(self.as_mut_slice())
    }
}

impl<const N: usize, T> From<[T; N]> for Buffer<N, T>
where
    T: DeserializeOwned + Serialize,
{
    fn from(value: [T; N]) -> Self {
        Self(value)
    }
}

impl<const N: usize> TryFrom<Buffer<N, c_char>> for String {
    type Error = FromUtf8Error;

    fn try_from(value: Buffer<N, c_char>) -> Result<Self, Self::Error> {
        let slice = value.as_u8_bytes();
        String::from_utf8(Vec::from(slice))
    }
}

impl<const N: usize> TryFrom<&[c_char]> for Buffer<N, c_char> {
    type Error = FormatError;

    fn try_from(value: &[c_char]) -> Result<Self, Self::Error> {
        if value.len() > N {
            Err(FormatError {
                struct_size: N,
                attempted_size: value.len(),
            })
        } else {
            let mut dst = Buffer::<N, c_char>::new();
            for (idx, &byte) in value.iter().enumerate() {
                dst.as_mut_slice()[idx] = byte;
            }
            Ok(dst)
        }
    }
}

impl<const N: usize> TryFrom<&[u8]> for Buffer<N, c_char> {
    type Error = FormatError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        cast_u8_to_cchar(value).try_into()
    }
}

impl<const N: usize> TryFrom<&str> for Buffer<N, c_char> {
    type Error = FormatError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        value.as_bytes().try_into()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FormatError {
    struct_size: usize,
    attempted_size: usize,
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Buffer length mismatch! src: {} bytes, dst: {} bytes",
            self.attempted_size, self.struct_size
        )
    }
}

impl std::error::Error for FormatError {}
