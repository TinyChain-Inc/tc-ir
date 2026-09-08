//! A deterministic map type used by the TinyChain IR.

use std::{
    collections::BTreeMap,
    iter::FromIterator,
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

use async_hash::{Digest, Hash, Output};
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

    /// Remove and return the parameter with the given `name`, or a "not found" error.
    pub fn require(&mut self, name: &str) -> TCResult<T> {
        let id: Id = name
            .parse()
            .map_err(|err| TCError::bad_request(format!("invalid map key id {name:?}: {err}")))?;

        self.remove(&id)
            .ok_or_else(|| TCError::not_found(format!("missing {name} parameter")))
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

impl<D: Digest, T: Hash<D>> Hash<D> for Map<T> {
    fn hash(self) -> Output<D> {
        Hash::<D>::hash(self.inner)
    }
}

impl<'a, D, T> Hash<D> for &'a Map<T>
where
    D: Digest,
    &'a T: Hash<D>,
{
    fn hash(self) -> Output<D> {
        Hash::<D>::hash(&self.inner)
    }
}

impl<T> de::FromStream for Map<T>
where
    T: de::FromStream<Context = ()>,
{
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: Self::Context,
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        struct MapVisitor<T>(PhantomData<T>);

        impl<T> de::Visitor for MapVisitor<T>
        where
            T: de::FromStream<Context = ()>,
        {
            type Value = Map<T>;

            fn expecting() -> &'static str {
                "a map with unique TinyChain IDs"
            }

            async fn visit_map<A: de::MapAccess>(
                self,
                mut access: A,
            ) -> Result<Self::Value, A::Error> {
                let mut inner = BTreeMap::new();
                while let Some(key) = access.next_key::<Id>(()).await? {
                    let value = access.next_value::<T>(()).await?;
                    if inner.insert(key.clone(), value).is_some() {
                        return Err(de::Error::custom(format!("duplicate map key {key}")));
                    }
                }
                Ok(Map { inner })
            }
        }

        decoder.decode_map(MapVisitor(PhantomData)).await
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
