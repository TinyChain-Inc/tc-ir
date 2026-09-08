//! Canonical, transport-neutral application identity and manifest contracts.

use std::fmt;
use std::str::FromStr;

use destream::{de, en, EncodeMap, IntoStream};
use pathlink::{Link, PathBuf, PathSegment};
use semver::Version;
use sha2::{Digest as _, Sha256};

use crate::{Id, Method};

/// A top-level TinyChain application namespace.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ApplicationKind {
    Class,
    Library,
    Service,
}

/// A parsed target in a top-level application namespace.
///
/// This is the only boundary which decides whether a path names a structural
/// namespace or a complete, versioned application entry. Once a semantic
/// version is encountered, every remaining segment is an application route
/// suffix.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApplicationTarget {
    Namespace {
        kind: ApplicationKind,
        segments: Box<[Id]>,
    },
    Entry {
        identity: ApplicationIdentity,
        suffix: Box<[PathSegment]>,
    },
}

impl ApplicationTarget {
    pub fn parse(value: &str) -> Result<Self, ApplicationError> {
        let value = value
            .split(['?', '#'])
            .next()
            .ok_or_else(|| ApplicationError::InvalidIdentity(value.to_string()))?;
        let link: Link = value
            .parse()
            .map_err(|_| ApplicationError::InvalidIdentity(value.to_string()))?;
        Self::from_segments(link.path())
    }

    pub fn from_segments(segments: &[PathSegment]) -> Result<Self, ApplicationError> {
        let Some(root) = segments.first() else {
            return Err(ApplicationError::InvalidIdentity("/".to_string()));
        };
        let kind = root.as_str().parse()?;

        let version_index = segments[1..]
            .iter()
            .position(|segment| Version::parse(segment.as_str()).is_ok())
            .map(|index| index + 1);
        if let Some(version_index) = version_index {
            if version_index < 3 {
                return Err(ApplicationError::InvalidIdentity(
                    "an application identity requires a publisher and resource path before its version"
                        .to_string(),
                ));
            }

            let publisher = segments[1].as_str();
            let resource = segments[2..version_index]
                .iter()
                .map(PathSegment::as_str)
                .collect::<Vec<_>>();
            let identity = ApplicationIdentity::parse(
                kind,
                publisher,
                &resource,
                segments[version_index].as_str(),
            )?;
            return Ok(Self::Entry {
                identity,
                suffix: segments[version_index + 1..].into(),
            });
        }

        let mut namespace = Vec::with_capacity(segments.len().saturating_sub(1));
        for segment in &segments[1..] {
            let segment: Id = segment.as_str().parse().map_err(|err| {
                ApplicationError::InvalidIdentity(format!(
                    "invalid application namespace segment: {err}"
                ))
            })?;
            if segment.as_str() == ".txfs" {
                return Err(ApplicationError::InvalidIdentity(
                    ".txfs is reserved for transactional storage metadata".to_string(),
                ));
            }
            namespace.push(segment);
        }

        Ok(Self::Namespace {
            kind,
            segments: namespace.into(),
        })
    }

    /// Parse a complete application identity and borrow its unmatched route suffix.
    pub fn split_segments(
        segments: &[PathSegment],
    ) -> Result<(ApplicationIdentity, &[PathSegment]), ApplicationError> {
        match Self::from_segments(segments)? {
            Self::Entry { identity, .. } => {
                let suffix = &segments[identity.resource.len() + 3..];
                Ok((identity, suffix))
            }
            Self::Namespace { .. } => Err(ApplicationError::InvalidIdentity(
                pathlink::PathBuf::from_slice(segments).to_string(),
            )),
        }
    }

    pub const fn kind(&self) -> ApplicationKind {
        match self {
            Self::Namespace { kind, .. } => *kind,
            Self::Entry { identity, .. } => identity.kind(),
        }
    }

    /// The validated structural segments below the application-kind root.
    pub fn structural_segments(&self) -> &[Id] {
        match self {
            Self::Namespace { segments, .. } => segments,
            Self::Entry { identity, .. } => identity.structural_segments(),
        }
    }

    pub fn suffix(&self) -> &[PathSegment] {
        match self {
            Self::Entry { suffix, .. } => suffix,
            Self::Namespace { .. } => &[],
        }
    }

    pub fn path(&self) -> PathBuf {
        match self {
            Self::Namespace { kind, segments } => std::iter::once(
                kind.root()
                    .parse()
                    .expect("application kind is a valid segment"),
            )
            .chain(segments.iter().cloned())
            .collect(),
            Self::Entry {
                identity, suffix, ..
            } => identity
                .link()
                .path()
                .iter()
                .cloned()
                .chain(suffix.iter().cloned())
                .collect(),
        }
    }
}

impl ApplicationKind {
    pub const fn root(self) -> &'static str {
        match self {
            Self::Class => "class",
            Self::Library => "lib",
            Self::Service => "service",
        }
    }
}

impl fmt::Display for ApplicationKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.root())
    }
}

impl FromStr for ApplicationKind {
    type Err = ApplicationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "class" => Ok(Self::Class),
            "lib" => Ok(Self::Library),
            "service" => Ok(Self::Service),
            other => Err(ApplicationError::InvalidKind(other.to_string())),
        }
    }
}

/// A validated, versioned application identity.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ApplicationIdentity {
    kind: ApplicationKind,
    publisher: Id,
    resource: Box<[Id]>,
    version: Version,
    segments: Box<[Id]>,
}

impl ApplicationIdentity {
    pub fn new(
        kind: ApplicationKind,
        publisher: Id,
        resource: impl Into<Box<[Id]>>,
        version: Version,
    ) -> Result<Self, ApplicationError> {
        let resource = resource.into();
        if publisher.as_str() == ".txfs" {
            return Err(ApplicationError::InvalidIdentity(
                ".txfs is reserved for transactional storage metadata".to_string(),
            ));
        }
        if resource.is_empty() {
            return Err(ApplicationError::InvalidIdentity(
                "an application resource path cannot be empty".to_string(),
            ));
        }

        for segment in resource.iter() {
            let segment = segment.as_str();
            if segment == ".txfs" {
                return Err(ApplicationError::InvalidIdentity(
                    ".txfs is reserved for transactional storage metadata".to_string(),
                ));
            }
            if Version::parse(segment).is_ok() {
                return Err(ApplicationError::InvalidIdentity(format!(
                    "semantic version {segment} is reserved for the terminal application version"
                )));
            }
        }

        let segments = std::iter::once(publisher.clone())
            .chain(resource.iter().cloned())
            .chain(std::iter::once(
                version
                    .to_string()
                    .parse()
                    .expect("a semantic version is a valid path segment"),
            ))
            .collect();
        Ok(Self {
            kind,
            publisher,
            resource,
            version,
            segments,
        })
    }

    pub fn parse(
        kind: ApplicationKind,
        publisher: &str,
        resource: &[&str],
        version: &str,
    ) -> Result<Self, ApplicationError> {
        let publisher = publisher.parse().map_err(|err| {
            ApplicationError::InvalidIdentity(format!("invalid publisher: {err}"))
        })?;
        let resource = resource
            .iter()
            .map(|segment| {
                segment.parse().map_err(|err| {
                    ApplicationError::InvalidIdentity(format!(
                        "invalid application resource segment: {err}"
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let version = Version::parse(version)
            .map_err(|err| ApplicationError::InvalidVersion(err.to_string()))?;
        Self::new(kind, publisher, resource, version)
    }

    pub const fn kind(&self) -> ApplicationKind {
        self.kind
    }

    pub fn publisher(&self) -> &Id {
        &self.publisher
    }

    pub fn resource(&self) -> &[Id] {
        &self.resource
    }

    pub fn version(&self) -> &Version {
        &self.version
    }

    pub fn structural_segments(&self) -> &[Id] {
        &self.segments
    }

    pub fn link(&self) -> Link {
        Link::from_str(&self.to_string()).expect("a validated application identity")
    }
}

impl fmt::Display for ApplicationIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "/{}/{}", self.kind, self.publisher)?;
        for segment in &self.resource {
            write!(f, "/{segment}")?;
        }
        write!(f, "/{}", self.version)
    }
}

impl FromStr for ApplicationIdentity {
    type Err = ApplicationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.contains("://") || value.contains(['?', '#']) {
            return Err(ApplicationError::InvalidIdentity(value.to_string()));
        }

        let segments = value
            .strip_prefix('/')
            .ok_or_else(|| ApplicationError::InvalidIdentity(value.to_string()))?
            .split('/')
            .collect::<Vec<_>>();
        if segments.len() < 4 {
            return Err(ApplicationError::InvalidIdentity(value.to_string()));
        }
        let kind = segments[0];
        let publisher = segments[1];
        let version = segments[segments.len() - 1];
        let resource = &segments[2..segments.len() - 1];

        Self::parse(kind.parse()?, publisher, resource, version)
    }
}

impl de::FromStream for ApplicationIdentity {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: Self::Context,
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        let value = String::from_stream((), decoder).await?;
        value.parse().map_err(de::Error::custom)
    }
}

impl<'en> en::IntoStream<'en> for ApplicationIdentity {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        self.to_string().into_stream(encoder)
    }
}

impl<'en> en::ToStream<'en> for ApplicationIdentity {
    fn to_stream<E: en::Encoder<'en>>(&'en self, encoder: E) -> Result<E::Ok, E::Error> {
        self.to_string().into_stream(encoder)
    }
}

/// A content-digest algorithm carried on the wire with its digest.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum DigestAlgorithm {
    Sha256,
}

impl fmt::Display for DigestAlgorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("sha256")
    }
}

impl FromStr for DigestAlgorithm {
    type Err = ApplicationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "sha256" => Ok(Self::Sha256),
            other => Err(ApplicationError::UnsupportedDigest(other.to_string())),
        }
    }
}

/// An algorithm-tagged, immutable content digest.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Digest {
    algorithm: DigestAlgorithm,
    bytes: [u8; 32],
}

impl Digest {
    pub fn sha256(bytes: impl AsRef<[u8]>) -> Self {
        Self {
            algorithm: DigestAlgorithm::Sha256,
            bytes: Sha256::digest(bytes.as_ref()).into(),
        }
    }

    pub(crate) const fn from_sha256(bytes: [u8; 32]) -> Self {
        Self {
            algorithm: DigestAlgorithm::Sha256,
            bytes,
        }
    }

    pub fn parse(algorithm: DigestAlgorithm, hex: &str) -> Result<Self, ApplicationError> {
        if hex.len() != 64 {
            return Err(ApplicationError::InvalidDigest(hex.to_string()));
        }

        let mut bytes = [0; 32];
        for (index, byte) in bytes.iter_mut().enumerate() {
            let offset = index * 2;
            *byte = u8::from_str_radix(&hex[offset..offset + 2], 16)
                .map_err(|_| ApplicationError::InvalidDigest(hex.to_string()))?;
        }

        Ok(Self { algorithm, bytes })
    }

    pub const fn algorithm(&self) -> DigestAlgorithm {
        self.algorithm
    }

    pub const fn bytes(&self) -> &[u8; 32] {
        &self.bytes
    }

    pub fn hex(&self) -> String {
        self.bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    pub fn verify(&self, bytes: impl AsRef<[u8]>) -> bool {
        *self == Self::sha256(bytes)
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.algorithm, self.hex())
    }
}

impl FromStr for Digest {
    type Err = ApplicationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (algorithm, digest) = value
            .split_once(':')
            .ok_or_else(|| ApplicationError::InvalidDigest(value.to_string()))?;
        Self::parse(algorithm.parse()?, digest)
    }
}

impl de::FromStream for Digest {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: Self::Context,
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        let (algorithm, digest): (String, String) =
            <(String, String)>::from_stream((), decoder).await?;
        let algorithm = algorithm.parse().map_err(de::Error::custom)?;
        Self::parse(algorithm, &digest).map_err(de::Error::custom)
    }
}

impl<'en> en::IntoStream<'en> for Digest {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        (self.algorithm.to_string(), self.hex()).into_stream(encoder)
    }
}

impl<'en> en::ToStream<'en> for Digest {
    fn to_stream<E: en::Encoder<'en>>(&'en self, encoder: E) -> Result<E::Ok, E::Error> {
        (self.algorithm.to_string(), self.hex()).into_stream(encoder)
    }
}

/// A dependency pinned to one immutable application revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DependencyLock {
    identity: ApplicationIdentity,
    digest: Digest,
    methods: std::collections::BTreeSet<Method>,
}

impl DependencyLock {
    pub fn new(
        identity: ApplicationIdentity,
        digest: Digest,
        methods: impl IntoIterator<Item = Method>,
    ) -> Self {
        Self {
            identity,
            digest,
            methods: methods.into_iter().collect(),
        }
    }

    pub fn identity(&self) -> &ApplicationIdentity {
        &self.identity
    }

    pub fn digest(&self) -> &Digest {
        &self.digest
    }

    pub fn methods(&self) -> &std::collections::BTreeSet<Method> {
        &self.methods
    }

    pub fn allows(&self, method: Method) -> bool {
        self.methods.contains(&method)
    }
}

impl de::FromStream for DependencyLock {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: Self::Context,
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        let (identity, digest, methods) =
            <(ApplicationIdentity, Digest, Vec<String>)>::from_stream((), decoder).await?;
        let methods = methods
            .into_iter()
            .map(|method| method.parse().map_err(de::Error::custom))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self::new(identity, digest, methods))
    }
}

impl<'en> en::IntoStream<'en> for DependencyLock {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        let methods = self
            .methods
            .into_iter()
            .map(|method| method.as_str().to_string())
            .collect::<Vec<_>>();
        (self.identity, self.digest, methods).into_stream(encoder)
    }
}

impl<'en> en::ToStream<'en> for DependencyLock {
    fn to_stream<E: en::Encoder<'en>>(&'en self, encoder: E) -> Result<E::Ok, E::Error> {
        self.clone().into_stream(encoder)
    }
}

/// One canonical application URI mapped directly to its complete definition.
///
/// The URI key owns identity and versioning; the value is the application
/// itself, with no manifest or payload envelope.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplicationDefinition<T> {
    identity: ApplicationIdentity,
    definition: T,
}

impl<T> ApplicationDefinition<T> {
    pub fn new(identity: ApplicationIdentity, definition: T) -> Self {
        Self {
            identity,
            definition,
        }
    }

    pub fn identity(&self) -> &ApplicationIdentity {
        &self.identity
    }

    pub fn definition(&self) -> &T {
        &self.definition
    }

    pub fn into_parts(self) -> (ApplicationIdentity, T) {
        (self.identity, self.definition)
    }
}

impl<T> de::FromStream for ApplicationDefinition<T>
where
    T: de::FromStream,
    T::Context: Clone + Send + Sync,
{
    type Context = T::Context;

    async fn from_stream<D: de::Decoder>(
        context: Self::Context,
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        struct DefinitionVisitor<T: de::FromStream> {
            context: T::Context,
            marker: std::marker::PhantomData<T>,
        }

        impl<T> de::Visitor for DefinitionVisitor<T>
        where
            T: de::FromStream,
            T::Context: Clone + Send + Sync,
        {
            type Value = ApplicationDefinition<T>;

            fn expecting() -> &'static str {
                "one canonical application URI mapped directly to its definition"
            }

            async fn visit_map<A: de::MapAccess>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let identity = map
                    .next_key::<String>(())
                    .await?
                    .ok_or_else(|| de::Error::custom("empty application definition"))?
                    .parse()
                    .map_err(de::Error::custom)?;
                let definition = map.next_value(self.context).await?;
                if map.next_key::<de::IgnoredAny>(()).await?.is_some() {
                    return Err(de::Error::custom(
                        "an application definition must contain exactly one URI",
                    ));
                }
                Ok(ApplicationDefinition::new(identity, definition))
            }
        }

        decoder
            .decode_map(DefinitionVisitor {
                context,
                marker: std::marker::PhantomData,
            })
            .await
    }
}

impl<'en, T> en::IntoStream<'en> for ApplicationDefinition<T>
where
    T: en::IntoStream<'en> + 'en,
{
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        let mut map = encoder.encode_map(Some(1))?;
        map.encode_entry(self.identity.to_string(), self.definition)?;
        map.end()
    }
}

impl<'en, T> en::ToStream<'en> for ApplicationDefinition<T>
where
    T: Clone + en::IntoStream<'en> + 'en,
{
    fn to_stream<E: en::Encoder<'en>>(&'en self, encoder: E) -> Result<E::Ok, E::Error> {
        self.clone().into_stream(encoder)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApplicationError {
    InvalidDigest(String),
    InvalidIdentity(String),
    InvalidKind(String),
    InvalidVersion(String),
    UnsupportedDigest(String),
}

impl fmt::Display for ApplicationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDigest(value) => write!(f, "invalid digest {value}"),
            Self::InvalidIdentity(value) => write!(f, "invalid application identity {value}"),
            Self::InvalidKind(value) => write!(f, "invalid application kind {value}"),
            Self::InvalidVersion(value) => write!(f, "invalid semantic version {value}"),
            Self::UnsupportedDigest(value) => write!(f, "unsupported digest algorithm {value}"),
        }
    }
}

impl std::error::Error for ApplicationError {}

#[cfg(test)]
mod tests {
    use crate::Scalar;
    use futures::TryStreamExt;
    use tc_value::Value;

    use super::*;

    fn identity() -> ApplicationIdentity {
        "/lib/example-devco/math/arithmetic/1.2.3"
            .parse()
            .expect("identity")
    }

    #[test]
    fn identity_is_canonical() {
        let identity = identity();
        assert_eq!(identity.kind(), ApplicationKind::Library);
        assert_eq!(
            identity.resource(),
            &[
                "math".parse::<Id>().expect("resource"),
                "arithmetic".parse::<Id>().expect("resource")
            ]
        );
        assert_eq!(
            identity.to_string(),
            "/lib/example-devco/math/arithmetic/1.2.3"
        );
        assert!("/lib/example-devco/math"
            .parse::<ApplicationIdentity>()
            .is_err());
        assert!("/lib/example-devco/1.2.3"
            .parse::<ApplicationIdentity>()
            .is_err());
        assert!("/data/example-devco/math/1.2.3"
            .parse::<ApplicationIdentity>()
            .is_err());
        assert!("/lib/example-devco/math/latest"
            .parse::<ApplicationIdentity>()
            .is_err());
    }

    #[test]
    fn identity_splits_from_its_unmatched_route_suffix() {
        let path: Link = "/lib/example-devco/math/arithmetic/1.2.3/add/left"
            .parse()
            .expect("application route");
        let (identity, suffix) =
            ApplicationTarget::split_segments(path.path()).expect("application route");
        assert_eq!(identity, self::identity());
        assert_eq!(
            suffix.iter().map(ToString::to_string).collect::<Vec<_>>(),
            ["add", "left"]
        );
        let path: Link = "/lib/example-devco/math/add".parse().unwrap();
        assert!(ApplicationTarget::split_segments(path.path()).is_err());
        assert!("/lib/example-devco/1.2.3/math/2.0.0"
            .parse::<ApplicationIdentity>()
            .is_err());
        assert!("/lib/example-devco/.txfs/2.0.0"
            .parse::<ApplicationIdentity>()
            .is_err());
        assert!("/lib/.txfs/math/2.0.0"
            .parse::<ApplicationIdentity>()
            .is_err());
    }

    #[test]
    fn target_distinguishes_namespaces_entries_and_suffixes() {
        assert_eq!(
            ApplicationTarget::parse("/lib").unwrap(),
            ApplicationTarget::Namespace {
                kind: ApplicationKind::Library,
                segments: Box::new([]),
            }
        );
        assert_eq!(
            ApplicationTarget::parse("/class/example-devco/models").unwrap(),
            ApplicationTarget::Namespace {
                kind: ApplicationKind::Class,
                segments: vec!["example-devco".parse().unwrap(), "models".parse().unwrap()].into(),
            }
        );

        let target =
            ApplicationTarget::parse("/service/example-devco/catalog/search/1.2.3/items/current")
                .unwrap();
        assert_eq!(
            target.path().to_string(),
            "/service/example-devco/catalog/search/1.2.3/items/current"
        );
        assert_eq!(
            target
                .structural_segments()
                .iter()
                .map(Id::as_str)
                .collect::<Vec<_>>(),
            ["example-devco", "catalog", "search", "1.2.3"]
        );
        let ApplicationTarget::Entry {
            identity, suffix, ..
        } = target
        else {
            panic!("expected an application entry")
        };
        assert_eq!(
            identity.to_string(),
            "/service/example-devco/catalog/search/1.2.3"
        );
        assert_eq!(
            suffix.iter().map(PathSegment::as_str).collect::<Vec<_>>(),
            ["items", "current"]
        );
    }

    #[test]
    fn target_rejects_reserved_or_prefix_ambiguous_segments() {
        assert!(ApplicationTarget::parse("/lib/example-devco/.txfs").is_err());
        assert!(ApplicationTarget::parse("/lib/example-devco/1.2.3").is_err());
        assert!(ApplicationTarget::parse("/lib/example-devco/models/1.2.3/2.0.0").is_ok());
    }

    #[test]
    fn digest_is_algorithm_tagged_and_verified() {
        let digest = Digest::sha256(b"definition");
        assert!(digest.verify(b"definition"));
        assert!(!digest.verify(b"different"));
        assert_eq!(digest.to_string().parse(), Ok(digest.clone()));
        assert_eq!(
            Digest::parse(DigestAlgorithm::Sha256, &digest.hex()),
            Ok(digest)
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn application_definition_is_the_literal_value() {
        let mut members = crate::Map::new();
        members.insert(
            "add".parse().expect("member"),
            Scalar::from(Value::String("sum".into())),
        );
        let definition = ApplicationDefinition::new(identity(), members);
        let encoded = destream_json::encode(definition.clone())
            .expect("encode")
            .try_collect::<Vec<_>>()
            .await
            .expect("collect")
            .concat();
        assert_eq!(
            String::from_utf8(encoded).expect("utf8"),
            include_str!("../fixtures/application_definition.json").trim_end()
        );

        let encoded = destream_json::encode(definition.clone()).expect("encode");
        let decoded: ApplicationDefinition<crate::Map<Scalar>> =
            destream_json::try_decode((), encoded)
                .await
                .expect("decode");
        assert_eq!(decoded, definition);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn application_definition_rejects_empty_multiple_and_metadata_envelopes() {
        for invalid in [
            br#"{}"#.as_slice(),
            br#"{"/lib/example-devco/a/1.0.0":{},"/lib/example-devco/b/1.0.0":{}}"#,
            br#"{"identity":"/lib/example-devco/a/1.0.0","definition":{}}"#,
        ] {
            let input = futures::stream::iter([Ok::<_, std::io::Error>(
                bytes::Bytes::copy_from_slice(invalid),
            )]);
            assert!(
                destream_json::try_decode::<_, _, ApplicationDefinition<Scalar>>((), input)
                    .await
                    .is_err()
            );
        }
    }
}
