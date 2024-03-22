use std::marker::PhantomData;

use serde::{de::Visitor, ser::SerializeTuple, Deserialize, Serialize};

#[derive(Debug)]
pub struct Padding<T>(PhantomData<T>);
impl<T> Padding<T> {
    const SIZE: usize = std::mem::size_of::<T>();
}

impl<T> Serialize for Padding<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut tup = serializer.serialize_tuple(Padding::<T>::SIZE)?;
        for _ in 0..Padding::<T>::SIZE {
            tup.serialize_element(&0x00_u8)?;
        }
        tup.end()
    }
}

impl<'de, T> Deserialize<'de> for Padding<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct PaddingVisitor<T>(PhantomData<T>);
        impl<'de, T> Visitor<'de> for PaddingVisitor<T> {
            type Value = Padding<T>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(formatter, "{} byte(s)", Padding::<T>::SIZE)
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                for _ in 0..Padding::<T>::SIZE {
                    seq.next_element::<u8>()?;
                }
                Ok(Padding(PhantomData))
            }
        }

        deserializer.deserialize_tuple(Padding::<T>::SIZE, PaddingVisitor(PhantomData))
    }
}
