//! A deterministic map type used by the TinyChain IR.

use std::{
    collections::BTreeMap,
    fmt,
    iter::FromIterator,
    ops::{Deref, DerefMut},
};

use destream::{de, en};
use tc_error::{TCError, TCResult};

use crate::Id;

/// A deterministic map type used by the TinyChain IR.
#[derive(Clone, Debug, PartialEq)]
pub struct Map<T> {
    inner: BTreeMap<Id, T>,
}

impl<T> Map<T> {
    /// Construct a new [`Map`].
    pub fn new() -> Self {
        Self {
            inner: BTreeMap::new(),
        }
    }

    /// Construct a new [`Map`] with a single entry.
    pub fn one(key: impl Into<Id>, value: T) -> Self {
        let mut map = Self::new();
        map.insert(key.into(), value);
        map
    }

    /// Return an error if this [`Map`] is not empty.
    pub fn expect_empty(self) -> TCResult<()>
    where
        T: fmt::Debug,
    {
        if self.is_empty() {
            Ok(())
        } else {
            Err(TCError::unexpected(self, "no parameters"))
        }
    }

    /// Retrieve this [`Map`]'s underlying [`BTreeMap`].
    pub fn into_inner(self) -> BTreeMap<Id, T> {
        self.inner
    }

    /// Remove and return the parameter with the given `name`, or `None` if not present.
    pub fn optional(&mut self, name: &str) -> TCResult<Option<T>> {
        let id: Id = name.parse().map_err(|err| {
            TCError::bad_request(format!("invalid map key id {name:?}: {err}"))
        })?;

        Ok(self.remove(&id))
    }

    /// Remove and return the parameter with the given `name`, or a "not found" error.
    pub fn require(&mut self, name: &str) -> TCResult<T> {
        let id: Id = name.parse().map_err(|err| {
            TCError::bad_request(format!("invalid map key id {name:?}: {err}"))
        })?;

        self.remove(&id)
            .ok_or_else(|| TCError::not_found(format!("missing {name} parameter")))
    }

    /// Remove and return the parameter with the given `name`, or panic if missing.
    pub fn expect(&mut self, name: &str) -> T
    where
        T: fmt::Debug,
    {
        match self.require(name) {
            Ok(value) => value,
            Err(err) => panic!("{err}"),
        }
    }
}

impl<T> Default for Map<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Deref for Map<T> {
    type Target = BTreeMap<Id, T>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T> DerefMut for Map<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<T> Extend<(Id, T)> for Map<T> {
    fn extend<I: IntoIterator<Item = (Id, T)>>(&mut self, iter: I) {
        for (key, value) in iter.into_iter() {
            self.insert(key, value);
        }
    }
}

impl<T> IntoIterator for Map<T> {
    type Item = (Id, T);
    type IntoIter = <BTreeMap<Id, T> as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.into_iter()
    }
}

impl<'a, T> IntoIterator for &'a Map<T> {
    type Item = (&'a Id, &'a T);
    type IntoIter = <&'a BTreeMap<Id, T> as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.iter()
    }
}

impl<T> FromIterator<(Id, T)> for Map<T> {
    fn from_iter<I: IntoIterator<Item = (Id, T)>>(iter: I) -> Self {
        let inner = BTreeMap::from_iter(iter);
        Self { inner }
    }
}

impl<T> From<BTreeMap<Id, T>> for Map<T> {
    fn from(inner: BTreeMap<Id, T>) -> Self {
        Self { inner }
    }
}

impl<T> From<Map<T>> for BTreeMap<Id, T> {
    fn from(map: Map<T>) -> Self {
        map.inner
    }
}

impl<T> de::FromStream for Map<T>
where
    T: de::FromStream<Context = ()>,
{
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        context: Self::Context,
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        let inner = BTreeMap::<Id, T>::from_stream(context, decoder).await?;
        Ok(Self { inner })
    }
}

impl<'en, T: en::IntoStream<'en> + 'en> en::IntoStream<'en> for Map<T> {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        self.inner.into_stream(encoder)
    }
}

impl<'en, T: en::ToStream<'en> + 'en> en::ToStream<'en> for Map<T> {
    fn to_stream<E: en::Encoder<'en>>(&'en self, encoder: E) -> Result<E::Ok, E::Error> {
        self.inner.to_stream(encoder)
    }
}

