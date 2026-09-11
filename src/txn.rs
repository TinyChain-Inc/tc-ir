use std::{fmt, str::FromStr};

/// Network time as nanoseconds since Unix epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
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

/// The unique ID of a transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
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

fn decode_hex_byte(pair: &str) -> Result<u8, &'static str> {
    u8::from_str_radix(pair, 16).map_err(|_| "invalid TxnId trace")
}

impl fmt::Display for TxnId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}-", self.timestamp, self.nonce)?;

        for byte in self.trace {
            write!(f, "{byte:02x}")?;
        }

        Ok(())
    }
}

impl FromStr for TxnId {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split('-');
        let ts = parts.next().ok_or("missing TxnId timestamp")?;
        let nonce = parts.next().ok_or("missing TxnId nonce")?;
        let trace_hex = parts.next().ok_or("missing TxnId trace")?;

        if parts.next().is_some() {
            return Err("transaction IDs must look like `<timestamp>-<nonce>-<tracehex>`");
        }

        if trace_hex.len() != 64 {
            return Err("TxnId trace must be 32 bytes encoded as lowercase hex");
        }

        let timestamp = NetworkTime::from_nanos(ts.parse().map_err(|_| "invalid TxnId timestamp")?);
        let nonce = nonce
            .parse()
            .map_err(|_| "invalid TxnId nonce (expected u16)")?;
        let mut trace = [0u8; 32];

        for (index, byte) in trace.iter_mut().enumerate() {
            let offset = index * 2;
            *byte = decode_hex_byte(&trace_hex[offset..offset + 2])?;
        }

        Ok(Self::from_parts(timestamp, nonce).with_trace(trace))
    }
}

/// Basic transaction context every handler receives.
pub trait Transaction: Send + Sync {
    /// Unique identifier chosen by the control plane.
    fn id(&self) -> TxnId;
}

/// Transaction lifecycle callbacks.
pub trait Transact: Send + Sync {
    /// Commit this transaction's pending changes.
    fn commit(
        &self,
        txn_id: TxnId,
    ) -> impl std::future::Future<Output = tc_error::TCResult<()>> + Send;

    /// Roll back this transaction's pending changes.
    fn rollback(
        &self,
        txn_id: &TxnId,
    ) -> impl std::future::Future<Output = tc_error::TCResult<()>> + Send;

    /// Finalize transaction history at the given frontier.
    fn finalize(
        &self,
        txn_id: &TxnId,
    ) -> impl std::future::Future<Output = tc_error::TCResult<()>> + Send;
}
