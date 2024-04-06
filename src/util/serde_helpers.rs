use std::marker::PhantomData;

use serde::{
    de::{self, DeserializeOwned, SeqAccess, Visitor},
    ser::SerializeTuple,
    Deserializer, Serialize, Serializer,
};

pub fn serialize<const N: usize, S, T>(t: &[T; N], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    T: Serialize,
{
    let mut ser_tuple = serializer.serialize_tuple(N)?;
    for elem in t {
        ser_tuple.serialize_element(elem)?;
    }
    ser_tuple.end()
}

pub fn deserialize<'de, const N: usize, D, T>(deserializer: D) -> Result<[T; N], D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    struct ArrayVisitor<const N: usize, T>(PhantomData<T>);
    impl<'de, const N: usize, T> Visitor<'de> for ArrayVisitor<N, T>
    where
        T: DeserializeOwned,
    {
        type Value = [T; N];

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_fmt(format_args!("an array of length {N}"))
        }

        #[inline]
        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut data = Vec::with_capacity(N);
            for _ in 0..N {
                match (seq.next_element())? {
                    Some(val) => data.push(val),
                    None => return Err(de::Error::invalid_length(N, &self)),
                }
            }

            match data.try_into() {
                Ok(arr) => Ok(arr),
                Err(_) => unreachable!(),
            }
        }
    }

    deserializer.deserialize_tuple(N, ArrayVisitor::<N, T>(PhantomData))
}
