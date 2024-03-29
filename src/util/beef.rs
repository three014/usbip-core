use std::{
    borrow::{Borrow, Cow},
    ops::Deref,
};

pub enum Beef<'a, B>
where
    B: 'static + ?Sized + ToOwned,
{
    Borrowed(&'a B),
    Owned(<B as ToOwned>::Owned),
    Static(&'static B),
}

impl<'a, B> From<Beef<'a, B>> for Cow<'static, B>
where
    B: ?Sized + ToOwned + 'static,
{
    fn from(value: Beef<'a, B>) -> Self {
        match value {
            Beef::Borrowed(borrowed) => Cow::Owned(borrowed.to_owned()),
            Beef::Owned(owned) => Cow::Owned(owned),
            Beef::Static(staticc) => Cow::Borrowed(staticc),
        }
    }
}

impl<'a, B> Deref for Beef<'a, B>
where
    B: ?Sized + ToOwned + 'static,
{
    type Target = B;

    fn deref(&self) -> &Self::Target {
        match *self {
            Beef::Borrowed(borrowed) | Beef::Static(borrowed) => borrowed,
            Beef::Owned(ref owned) => owned.borrow(),
        }
    }
}
