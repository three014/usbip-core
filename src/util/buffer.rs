use std::fmt;
use serde::{de::DeserializeOwned, Deserialize, Serialize};

pub mod serde_helpers;

#[repr(transparent)]
#[derive(Serialize, Deserialize, Debug)]
pub struct Buffer<const N: usize, T>(#[serde(with = "serde_helpers")] [T; N])
where
    T: DeserializeOwned + Serialize;

impl<const N: usize, T> Buffer<N, T> 
where
    T: DeserializeOwned + Serialize
{
    pub fn new() -> Buffer<N, i8> {
        Buffer([0; N])
    }
}

impl<const N: usize> Buffer<N, i8> {
    pub fn as_bytes(&self) -> &[i8] {
        &self.0
    }

    pub fn as_mut_bytes(&mut self) -> &mut [i8] {
        &mut self.0
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

impl<const N: usize> TryFrom<&[i8]> for Buffer<N, i8> {
    type Error = FormatError;

    fn try_from(value: &[i8]) -> Result<Self, Self::Error> {
        if value.len() > N {
            Err(FormatError {
                struct_size: N,
                attempted_size: value.len(),
            })
        } else {
            let mut dst = Buffer::<N, i8>::new();
            for (idx, &byte) in value.iter().enumerate() {
                dst.as_mut_bytes()[idx] = byte;
            }
            Ok(dst)
        }
    }
}

impl<const N: usize> TryFrom<&[u8]> for Buffer<N, i8> {
    type Error = FormatError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        crate::util::cast_u8_to_i8_slice(value).try_into()
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
