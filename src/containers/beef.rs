use std::{
    borrow::{Borrow, Cow},
    fmt::Display,
    ops::Deref,
};

/// Like [`Cow`], but differentiates between borrowed items
/// and `'static` items.
///
/// Currently doesn't support mutation due to the project
/// not requiring it at the moment.
pub enum Beef<'a, B>
where
    B: 'static + ?Sized + ToOwned,
{
    Borrowed(&'a B),
    Owned(<B as ToOwned>::Owned),
    Static(&'static B),
}

impl<'a, B> Clone for Beef<'a, B>
where
    B: ?Sized + ToOwned,
    <B as ToOwned>::Owned: Clone,
{
    fn clone(&self) -> Self {
        match *self {
            Beef::Borrowed(borrowed) => Beef::Borrowed(borrowed),
            Beef::Static(staticc) => Beef::Static(staticc),
            Beef::Owned(ref owned) => Beef::Owned(owned.clone())
        }
    }
}

impl<'a, B> std::fmt::Debug for Beef<'a, B>
where
    B: 'static + ?Sized + ToOwned + std::fmt::Debug,
    <B as ToOwned>::Owned: core::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.deref().fmt(f)
    }
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

impl<'a, B> Display for Beef<'a, B>
where
    B: ?Sized + ToOwned + Display + 'static,
    <B as ToOwned>::Owned: Display,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.deref().fmt(f)
    }
}


