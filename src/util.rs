pub mod __padding;
pub mod __private {
    pub trait Sealed {}
}

use std::str::FromStr;

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
