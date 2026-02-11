use std::{fmt, str::FromStr};

use destream::{de, en, EncodeMap, IntoStream};
use pathlink::Link;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
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
