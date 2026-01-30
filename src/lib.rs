//! Core TinyChain IR traits, inspired by `tc-transact`'s `Handler` and `Route` abstractions.
//!
//! These definitions intentionally mirror the behavior of the existing `tc-transact`
//! `Handler`/`Route` traits while staying agnostic to any particular runtime. They should
//! be expressive enough to back WASM sandboxes, PyO3 bindings, or the existing Rust
//! server stack without leaking lower-level implementation details.

use std::{collections::BTreeMap, fmt, future::Future, marker::PhantomData, str::FromStr};

use destream::{de, en, EncodeMap, IntoStream};

use pathlink::{Link, Path, PathBuf, PathSegment};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tc_error::{TCError, TCResult};
use tc_value::Value;

pub use tc_value::class::{Class, NativeClass};

#[cfg(feature = "pyo3-conversions")]
use pyo3::prelude::*;

/// Network time as nanoseconds since Unix epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub struct NetworkTime(u64);

impl NetworkTime {
    pub const fn from_nanos(nanos: u64) -> Self {
        Self(nanos)
    }

    pub const fn as_nanos(&self) -> u64 {
        self.0
    }
}

impl fmt::Display for NetworkTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for NetworkTime {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let nanos = s.parse().map_err(|_| "invalid NetworkTime")?;
        Ok(Self::from_nanos(nanos))
    }
}

/// The unique ID of a transaction, copied from `tc-transact` (with serde support).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub struct TxnId {
    timestamp: NetworkTime,
    nonce: u16,
    trace: [u8; 32],
}

impl TxnId {
    /// Construct a new TxnId from raw parts (timestamp in nanos + nonce).
    pub const fn from_parts(timestamp: NetworkTime, nonce: u16) -> Self {
        Self {
            timestamp,
            nonce,
            trace: [0u8; 32],
        }
    }

    /// Attach a tracing hash (host + txn) to this ID.
    pub fn with_trace(mut self, trace: [u8; 32]) -> Self {
        self.trace = trace;
        self
    }

    /// Timestamp component.
    pub const fn timestamp(&self) -> NetworkTime {
        self.timestamp
    }

    /// Nonce component used to break ties for identical timestamps.
    pub const fn nonce(&self) -> u16 {
        self.nonce
    }

    /// Tracing hash (opaque bytes).
    pub const fn trace_bytes(&self) -> &[u8; 32] {
        &self.trace
    }
}

impl fmt::Display for TxnId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.timestamp, self.nonce)
    }
}

impl FromStr for TxnId {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (ts, nonce) = s
            .split_once('-')
            .ok_or("transaction IDs must look like `<timestamp>-<nonce>`")?;

        let timestamp = NetworkTime::from_nanos(ts.parse().map_err(|_| "invalid TxnId timestamp")?);
        let nonce = nonce
            .parse()
            .map_err(|_| "invalid TxnId nonce (expected u16)")?;

        Ok(Self::from_parts(timestamp, nonce))
    }
}

/// Basic transaction context every handler receives.
pub trait Transaction: Send + Sync {
    /// Unique identifier chosen by the control plane.
    fn id(&self) -> TxnId;

    /// Consensus timestamp (deterministic per transaction).
    fn timestamp(&self) -> NetworkTime;

    /// Authorization claim scoped to this transaction.
    fn claim(&self) -> &Claim;
}

/// Serializable header that conveys transaction context across process or WASM boundaries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TxnHeader {
    id: TxnId,
    timestamp: NetworkTime,
    claim: Claim,
}

impl TxnHeader {
    pub fn new(id: TxnId, timestamp: NetworkTime, claim: Claim) -> Self {
        Self {
            id,
            timestamp,
            claim,
        }
    }

    pub fn from_transaction<T: Transaction + ?Sized>(txn: &T) -> Self {
        Self::new(txn.id(), txn.timestamp(), txn.claim().clone())
    }

    pub fn id(&self) -> TxnId {
        self.id
    }

    pub fn timestamp(&self) -> NetworkTime {
        self.timestamp
    }

    pub fn claim(&self) -> &Claim {
        &self.claim
    }
}

impl Serialize for TxnHeader {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeMap;

        let mut map = serializer.serialize_map(Some(3))?;
        map.serialize_entry("id", &self.id.to_string())?;
        map.serialize_entry("timestamp", &self.timestamp.as_nanos())?;
        let claim = (self.claim.link.to_string(), u32::from(self.claim.mask));
        map.serialize_entry("claim", &claim)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for TxnHeader {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::MapAccess;

        struct HeaderVisitor;

        impl<'de> serde::de::Visitor<'de> for HeaderVisitor {
            type Value = TxnHeader;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a transaction header map")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut id: Option<TxnId> = None;
                let mut timestamp: Option<NetworkTime> = None;
                let mut claim: Option<Claim> = None;

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "id" => {
                            let value = map.next_value::<String>()?;
                            let parsed = TxnId::from_str(&value)
                                .map_err(|err| serde::de::Error::custom(err.to_string()))?;
                            id = Some(parsed);
                        }
                        "timestamp" => {
                            let nanos = map.next_value::<u64>()?;
                            timestamp = Some(NetworkTime::from_nanos(nanos));
                        }
                        "claim" => {
                            let (link, mask): (String, u32) = map.next_value()?;
                            let link = Link::from_str(&link)
                                .map_err(|err| serde::de::Error::custom(err.to_string()))?;
                            let mask: umask::Mode = mask.into();
                            claim = Some(Claim::new(link, mask));
                        }
                        _ => {
                            let _ = map.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                }

                let id = id.ok_or_else(|| serde::de::Error::custom("missing id"))?;
                let timestamp =
                    timestamp.ok_or_else(|| serde::de::Error::custom("missing timestamp"))?;
                let claim = claim.ok_or_else(|| serde::de::Error::custom("missing claim"))?;

                Ok(TxnHeader::new(id, timestamp, claim))
            }
        }

        deserializer.deserialize_map(HeaderVisitor)
    }
}

impl de::FromStream for TxnHeader {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: Self::Context,
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        struct HeaderVisitor;

        impl de::Visitor for HeaderVisitor {
            type Value = TxnHeader;

            fn expecting() -> &'static str {
                "a transaction header map"
            }

            async fn visit_map<A: de::MapAccess>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut id = None;
                let mut timestamp = None;
                let mut claim = None;

                while let Some(key) = map.next_key::<String>(()).await? {
                    match key.as_str() {
                        "id" => {
                            let value = map.next_value::<String>(()).await?;
                            let parsed = TxnId::from_str(&value).map_err(de::Error::custom)?;
                            id = Some(parsed);
                        }
                        "timestamp" => {
                            let nanos = map.next_value::<u64>(()).await?;
                            timestamp = Some(NetworkTime::from_nanos(nanos));
                        }
                        "claim" => {
                            let (link, mask): (String, u32) = map.next_value(()).await?;
                            let link = Link::from_str(&link)
                                .map_err(|err| de::Error::custom(err.to_string()))?;
                            let mask: umask::Mode = mask.into();
                            claim = Some(Claim::new(link, mask));
                        }
                        _ => {
                            let _ = map.next_value::<de::IgnoredAny>(()).await?;
                        }
                    }
                }

                let id = id.ok_or_else(|| de::Error::custom("missing id"))?;
                let timestamp = timestamp.ok_or_else(|| de::Error::custom("missing timestamp"))?;
                let claim = claim.ok_or_else(|| de::Error::custom("missing claim"))?;

                Ok(TxnHeader::new(id, timestamp, claim))
            }
        }

        decoder.decode_map(HeaderVisitor).await
    }
}

impl<'en> en::IntoStream<'en> for TxnHeader {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        let mut map = encoder.encode_map(Some(3))?;
        map.encode_entry("id", self.id.to_string())?;
        map.encode_entry("timestamp", self.timestamp.as_nanos())?;
        let claim = (self.claim.link.to_string(), u32::from(self.claim.mask));
        map.encode_entry("claim", claim)?;
        map.end()
    }
}

impl<'en> en::ToStream<'en> for TxnHeader {
    fn to_stream<E: en::Encoder<'en>>(&'en self, encoder: E) -> Result<E::Ok, E::Error> {
        self.clone().into_stream(encoder)
    }
}

/// Authorization data issued by the control-plane / IAM stack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Claim {
    pub link: Link,
    pub mask: umask::Mode,
}

impl Claim {
    pub fn new(link: Link, mask: umask::Mode) -> Self {
        Self { link, mask }
    }

    /// Return true if this claim grants the required mask.
    pub fn allows(&self, link: &Link, required: umask::Mode) -> bool {
        if self.link != *link {
            return false;
        }

        let have: u32 = self.mask.into();
        let need: u32 = required.into();
        have & need == need
    }
}

impl Serialize for Claim {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let tuple = (self.link.to_string(), u32::from(self.mask) as u16);
        tuple.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Claim {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        <(String, u16)>::deserialize(deserializer).and_then(|(link, mask)| {
            let link =
                Link::from_str(&link).map_err(|err| serde::de::Error::custom(err.to_string()))?;
            Ok(Claim {
                link,
                mask: (mask as u32).into(),
            })
        })
    }
}

/// HTTP-like verbs supported by TinyChain routers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Method {
    Get,
    Put,
    Post,
    Delete,
}

/// IR analogue of `tc-transact`'s `Route` trait.
pub trait Route {
    type Handler;

    /// Resolve the handler mounted at the given path.
    fn route<'a>(&'a self, path: &'a [PathSegment]) -> Option<&'a Self::Handler>;
}

/// Marker trait implemented by every TinyChain handler.
pub trait Handler<T>: Send + Sync
where
    T: Transaction + ?Sized,
{
    fn method_not_supported(method: Method) -> TCError {
        TCError::method_not_allowed(method, std::any::type_name::<Self>())
    }
}

impl<T, H> Handler<T> for H
where
    T: Transaction + ?Sized,
    H: Send + Sync,
{
}

#[cfg(feature = "pyo3-conversions")]
pub trait FromPyRequest<'py>: Sized {
    type PyError;

    fn from_py(obj: &Bound<'py, PyAny>) -> Result<Self, Self::PyError>;
}

macro_rules! define_verb_handler {
    ($trait_name:ident, $fn_name:ident, $method:expr) => {
        pub trait $trait_name<T>: Handler<T>
        where
            T: Transaction + ?Sized,
        {
            type Request: de::FromStream<Context = Self::RequestContext>;
            type RequestContext: Send;
            type Response;
            type Error;
            type Fut<'a>: Future<Output = Result<Self::Response, Self::Error>> + Send + 'a
            where
                Self: 'a,
                T: 'a,
                Self::Request: 'a;

            fn $fn_name<'a>(
                &'a self,
                txn: &'a T,
                request: Self::Request,
            ) -> TCResult<Self::Fut<'a>> {
                let _ = (txn, request);
                Err(Self::method_not_supported($method))
            }
        }
    };
}

define_verb_handler!(HandleGet, get, Method::Get);
define_verb_handler!(HandlePut, put, Method::Put);
define_verb_handler!(HandlePost, post, Method::Post);
define_verb_handler!(HandleDelete, delete, Method::Delete);

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

/// Scalar values exchanged via the TinyChain IR.
#[derive(Clone, Debug, PartialEq)]
pub enum Scalar {
    Value(Value),
    Ref(Box<TCRef>),
}

/// A deterministic map type used by the TinyChain IR.
///
/// This is a v2 placeholder for the richer map/tuple scalar types from v1.
pub type Map<T> = BTreeMap<String, T>;

/// A reference to a named value in a scope (e.g. "$self").
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct IdRef(String);

impl IdRef {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The subject of an op.
///
/// Copied from the v1 `OpRef` model: an op may target either a concrete [`Link`] or a scoped
/// reference plus a suffix path.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Subject {
    Link(Link),
    Ref(IdRef, PathBuf),
}

/// The data defining a reference to a GET op.
pub type GetRef = (Subject, Scalar);

/// The data defining a reference to a PUT op.
pub type PutRef = (Subject, Scalar, Scalar);

/// The data defining a reference to a POST op.
pub type PostRef = (Subject, Map<Scalar>);

/// The data defining a reference to a DELETE op.
pub type DeleteRef = (Subject, Scalar);

/// A reference to an op.
///
/// This is a structural port of the v1 `OpRef` enum. Resolution/execution is implemented by the
/// host kernel and is intentionally not part of this type definition.
#[derive(Clone, Debug, PartialEq)]
pub enum OpRef {
    Get(GetRef),
    Put(PutRef),
    Post(PostRef),
    Delete(DeleteRef),
}

/// A reference to a scalar value.
///
/// v2 currently supports only op references (`TCRef::Op`). Control-flow references (`If`, `While`,
/// `Case`, etc.) will be added once the kernel has a complete ref scheduler.
#[derive(Clone, Debug, PartialEq)]
pub enum TCRef {
    Op(OpRef),
}

impl Default for Scalar {
    fn default() -> Self {
        Scalar::Value(Value::default())
    }
}

impl From<Value> for Scalar {
    fn from(value: Value) -> Self {
        Scalar::Value(value)
    }
}

impl From<TCRef> for Scalar {
    fn from(value: TCRef) -> Self {
        Scalar::Ref(Box::new(value))
    }
}

impl From<u64> for Scalar {
    fn from(value: u64) -> Self {
        Scalar::Value(Value::from(value))
    }
}

/// Directory-style router inspired by TinyChain's transactional `Dir`.
#[derive(Default)]
pub struct Dir<H> {
    entries: BTreeMap<PathSegment, DirEntry<H>>,
}

enum DirEntry<H> {
    Dir(Box<Dir<H>>),
    Handler(H),
}

impl<H: Clone> Clone for Dir<H> {
    fn clone(&self) -> Self {
        Self {
            entries: self.entries.clone(),
        }
    }
}

impl<H: Clone> Clone for DirEntry<H> {
    fn clone(&self) -> Self {
        match self {
            Self::Dir(dir) => Self::Dir(Box::new((**dir).clone())),
            Self::Handler(handler) => Self::Handler(handler.clone()),
        }
    }
}

impl<H: fmt::Debug> fmt::Debug for Dir<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.entries.iter()).finish()
    }
}

impl<H: fmt::Debug> fmt::Debug for DirEntry<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dir(_) => f.write_str("Dir(...)"),
            Self::Handler(handler) => f.debug_tuple("Handler").field(handler).finish(),
        }
    }
}

impl<H> Dir<H> {
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    /// Build a directory from a collection of `(path, handler)` entries.
    pub fn from_routes<I>(routes: I) -> TCResult<Self>
    where
        I: IntoIterator<Item = (Vec<PathSegment>, H)>,
    {
        let mut dir = Self::new();
        for (path, handler) in routes {
            if path.is_empty() {
                return Err(TCError::bad_request("cannot mount handler at root"));
            }
            dir.insert_segments(&path, handler)?;
        }
        Ok(dir)
    }

    fn insert_segments(&mut self, path: &[PathSegment], handler: H) -> TCResult<()> {
        let (head, tail) = path
            .split_first()
            .expect("caller ensures path is non-empty");

        use std::collections::btree_map::Entry;

        if tail.is_empty() {
            match self.entries.entry(head.clone()) {
                Entry::Vacant(entry) => {
                    entry.insert(DirEntry::Handler(handler));
                    Ok(())
                }
                Entry::Occupied(_) => Err(TCError::bad_request(format!(
                    "handler already mounted at path {}",
                    format_path(path)
                ))),
            }
        } else {
            let entry = self.entries.entry(head.clone()).or_insert_with(|| {
                DirEntry::Dir(Box::new(Dir {
                    entries: BTreeMap::new(),
                }))
            });

            match entry {
                DirEntry::Dir(dir) => dir.insert_segments(tail, handler),
                DirEntry::Handler(_) => Err(TCError::bad_request(format!(
                    "cannot mount handler below a leaf handler at {}",
                    format_path(path)
                ))),
            }
        }
    }

    fn route_path<'a>(&'a self, path: &'a [PathSegment]) -> Option<&'a H> {
        let (head, tail) = path.split_first()?;
        match self.entries.get(head) {
            Some(DirEntry::Handler(handler)) if tail.is_empty() => Some(handler),
            Some(DirEntry::Dir(dir)) => dir.route_path(tail),
            _ => None,
        }
    }
}

impl<H> Route for Dir<H> {
    type Handler = H;

    fn route<'a>(&'a self, path: &'a [PathSegment]) -> Option<&'a Self::Handler> {
        self.route_path(path)
    }
}

fn format_path(path: &[PathSegment]) -> String {
    Path::from(path).to_string()
}

/// Parse a `/foo/bar`-style path into [`PathSegment`]s for use with a [`Dir`].
pub fn parse_route_path(path: &str) -> TCResult<Vec<PathSegment>> {
    if path.is_empty() {
        return Err(TCError::bad_request("route paths must not be empty"));
    }

    let trimmed = path.trim();
    let trimmed = trimmed.strip_prefix('/').unwrap_or(trimmed);
    if trimmed.is_empty() {
        return Err(TCError::bad_request(
            "route paths must contain at least one segment",
        ));
    }

    trimmed
        .split('/')
        .map(|segment| {
            PathSegment::from_str(segment).map_err(|cause| {
                TCError::bad_request(format!("invalid route segment '{segment}': {cause}"))
            })
        })
        .collect()
}

/// Build a [`Dir`] from string routes with minimal boilerplate.
#[macro_export]
macro_rules! tc_library_routes {
    ($($path:expr => $handler:expr),+ $(,)?) => {{
        (|| -> tc_error::TCResult<_> {
            let routes = vec![
                $(
                    ($crate::parse_route_path($path)?, $handler)
                ),+
            ];
            $crate::Dir::from_routes(routes)
        })()
    }};
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::Pin;

    #[derive(Clone)]
    struct FakeTxn {
        claim: Claim,
    }

    impl FakeTxn {
        fn new(claim: Claim) -> Self {
            Self { claim }
        }
    }

    impl Transaction for FakeTxn {
        fn id(&self) -> TxnId {
            TxnId::from_parts(NetworkTime::from_nanos(42), 7)
        }

        fn timestamp(&self) -> NetworkTime {
            NetworkTime::from_nanos(42)
        }

        fn claim(&self) -> &Claim {
            &self.claim
        }
    }

    struct HelloHandler;

    impl HandleGet<FakeTxn> for HelloHandler {
        type Request = String;
        type RequestContext = ();
        type Response = String;
        type Error = ();
        type Fut<'a> =
            Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send + 'a>>;

        fn get<'a>(&'a self, _txn: &'a FakeTxn, request: Self::Request) -> TCResult<Self::Fut<'a>> {
            Ok(Box::pin(async move { Ok(format!("hello {request}")) }))
        }
    }

    #[test]
    fn handler_invocation() {
        let handler = HelloHandler;
        let claim = Claim::new(Link::from_str("/hello").unwrap(), umask::Mode::all());
        let txn = FakeTxn::new(claim);

        let fut = handler.get(&txn, "world".into()).expect("GET supported");
        let out = futures::executor::block_on(fut).unwrap();
        assert_eq!(out, "hello world");
    }

    #[test]
    fn library_schema_destream_roundtrip() {
        let schema = LibrarySchema::new(
            Link::from_str("/lib/service").expect("link"),
            "0.1.0",
            vec![
                Link::from_str("/lib/dependency").expect("dep"),
                Link::from_str("/lib/other").expect("dep"),
            ],
        );

        let encoded = destream_json::encode(schema.clone()).expect("encode schema");
        let decoded: LibrarySchema =
            futures::executor::block_on(destream_json::try_decode((), encoded))
                .expect("decode schema");

        assert_eq!(decoded, schema);
    }

    #[test]
    fn txn_header_destream_roundtrip() {
        let claim = Claim::new(Link::from_str("/lib/service").unwrap(), umask::Mode::all());
        let header = TxnHeader::new(
            TxnId::from_parts(NetworkTime::from_nanos(7), 1),
            NetworkTime::from_nanos(7),
            claim,
        );

        let encoded = destream_json::encode(header.clone()).expect("encode header");
        let decoded: TxnHeader =
            futures::executor::block_on(destream_json::try_decode((), encoded))
                .expect("decode header");

        assert_eq!(decoded, header);
    }

    fn segment(name: &str) -> PathSegment {
        PathSegment::from_str(name).expect("path segment")
    }

    #[test]
    fn dir_routes_nested_handler() {
        let path = vec![segment("library"), segment("status")];
        let dir = Dir::from_routes(vec![(path.clone(), HelloHandler)]).expect("dir");

        let claim = Claim::new(Link::from_str("/lib").unwrap(), umask::Mode::all());
        let txn = FakeTxn::new(claim);

        let handler = dir.route(&path).expect("handler resolved");
        let fut = handler.get(&txn, "tinychain".into()).expect("GET");
        let out = futures::executor::block_on(fut).unwrap();
        assert_eq!(out, "hello tinychain");
    }

    #[test]
    fn dir_detects_conflicts() {
        let path = vec![segment("library"), segment("status")];

        match Dir::from_routes(vec![
            (path.clone(), HelloHandler),
            (path.clone(), HelloHandler),
        ]) {
            Ok(_) => panic!("expected conflict inserting duplicate handler"),
            Err(err) => assert!(err.message().contains("already mounted")),
        }
    }

    #[test]
    fn macro_builds_routes() {
        let dir = tc_library_routes! {
            "/lib/status" => HelloHandler,
        }
        .expect("macro routes");

        let claim = Claim::new(Link::from_str("/lib").unwrap(), umask::Mode::all());
        let txn = FakeTxn::new(claim);
        let path = [segment("lib"), segment("status")];
        let handler = dir.route(&path).expect("handler");
        let fut = handler.get(&txn, "macro".into()).expect("GET");
        let out = futures::executor::block_on(fut).unwrap();
        assert_eq!(out, "hello macro");
    }

    #[test]
    fn static_library_wraps_schema_and_routes() {
        let schema = LibrarySchema::new(Link::from_str("/lib/service").unwrap(), "1.0.0", vec![]);
        let routes = tc_library_routes! {
            "/lib/status" => HelloHandler,
        }
        .expect("routes");

        let lib: StaticLibrary<FakeTxn, _> = StaticLibrary::new(schema.clone(), routes);
        assert_eq!(lib.schema(), &schema);
        let path = [segment("lib"), segment("status")];
        assert!(lib.routes().route(&path).is_some());
    }
}
