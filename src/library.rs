use std::marker::PhantomData;

use destream::{de, en, EncodeMap, IntoStream};
use pathlink::Link;

use crate::{Route, Transaction};

/// Static description of a TinyChain library exposed through `/lib`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibrarySchema {
    id: Link,
    version: String,
    dependencies: Vec<Link>,
}

impl LibrarySchema {
    /// Create a new schema with the given identifier, version, and dependency links.
    pub fn new(id: Link, version: impl Into<String>, dependencies: Vec<Link>) -> Self {
        Self {
            id,
            version: version.into(),
            dependencies,
        }
    }

    /// Unique library identifier (usually a `tc://` link).
    pub fn id(&self) -> &Link {
        &self.id
    }

    /// Version string advertised to runtimes.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Dependent libraries required for this module to load.
    pub fn dependencies(&self) -> &[Link] {
        &self.dependencies
    }
}

impl de::FromStream for LibrarySchema {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: Self::Context,
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        struct SchemaVisitor;

        impl de::Visitor for SchemaVisitor {
            type Value = LibrarySchema;

            fn expecting() -> &'static str {
                "a library schema map"
            }

            async fn visit_map<A: de::MapAccess>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut id = None;
                let mut version = None;
                let mut dependencies = None;

                while let Some(key) = map.next_key::<String>(()).await? {
                    match key.as_str() {
                        "id" => {
                            if id.is_some() {
                                return Err(de::Error::custom("duplicate id field"));
                            }

                            id = Some(map.next_value::<Link>(()).await?);
                        }
                        "version" => {
                            if version.is_some() {
                                return Err(de::Error::custom("duplicate version field"));
                            }

                            version = Some(map.next_value::<String>(()).await?);
                        }
                        "dependencies" => {
                            dependencies = Some(map.next_value::<Vec<Link>>(()).await?);
                        }
                        _ => {
                            let _ = map.next_value::<de::IgnoredAny>(()).await?;
                        }
                    }
                }

                let id = id.ok_or_else(|| de::Error::custom("missing id field"))?;
                let version = version.ok_or_else(|| de::Error::custom("missing version field"))?;
                let dependencies = dependencies.unwrap_or_default();

                Ok(LibrarySchema::new(id, version, dependencies))
            }
        }

        decoder.decode_map(SchemaVisitor).await
    }
}

impl<'en> en::IntoStream<'en> for LibrarySchema {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        let Self {
            id,
            version,
            dependencies,
        } = self;

        let mut map = encoder.encode_map(Some(3))?;
        map.encode_entry("id", id)?;
        map.encode_entry("version", version)?;
        map.encode_entry("dependencies", dependencies)?;
        map.end()
    }
}

impl<'en> en::ToStream<'en> for LibrarySchema {
    fn to_stream<E: en::Encoder<'en>>(&'en self, encoder: E) -> Result<E::Ok, E::Error> {
        self.clone().into_stream(encoder)
    }
}

/// Convenience wrapper that pairs a schema with a reusable routing table.
pub struct LibraryModule<Txn: ?Sized, Routes> {
    schema: LibrarySchema,
    routes: Routes,
    _txn: PhantomData<Txn>,
}

impl<Txn: ?Sized, Routes> LibraryModule<Txn, Routes>
where
    Txn: Transaction,
    Routes: Route,
{
    /// Construct a new [`LibraryModule`].
    pub fn new(schema: LibrarySchema, routes: Routes) -> Self {
        Self {
            schema,
            routes,
            _txn: PhantomData,
        }
    }
}

impl<Txn: ?Sized, Routes> Library for LibraryModule<Txn, Routes>
where
    Txn: Transaction,
    Routes: Route,
{
    type Txn = Txn;
    type Routes = Routes;

    fn schema(&self) -> &LibrarySchema {
        &self.schema
    }

    fn routes(&self) -> &Self::Routes {
        &self.routes
    }
}

/// Backwards-compatible alias for the previous `StaticLibrary` type name.
pub type StaticLibrary<Txn, Routes> = LibraryModule<Txn, Routes>;

/// Trait implemented by every TinyChain library, whether native or WASM-backed.
pub trait Library {
    type Txn: Transaction + ?Sized;
    type Routes: Route;

    /// Schema returned by `/lib`.
    fn schema(&self) -> &LibrarySchema;

    /// Root routing table used to dispatch runtime requests.
    fn routes(&self) -> &Self::Routes;
}

