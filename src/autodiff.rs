use std::fmt;

use destream::{de, en, EncodeMap, IntoStream};

use crate::tensor::{TensorGraph, TensorOp, TensorTypeSpec, ValueId};

// ---------------------------------------------------------------------------
// AutodiffRequest
// ---------------------------------------------------------------------------

/// Request to transform a tensor graph into its derivative graph.
///
/// Wire format: a JSON map with fields:
/// `graph`, `output`, `wrt`, `seed`, `op_contract_version`, `transform_version`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutodiffRequest {
    pub graph: TensorGraph,
    pub output: ValueId,
    pub wrt: Vec<ValueId>,
    pub seed: ValueId,
    pub op_contract_version: String,
    pub transform_version: String,
}

impl AutodiffRequest {
    pub fn new(
        graph: TensorGraph,
        output: ValueId,
        wrt: Vec<ValueId>,
        seed: ValueId,
        op_contract_version: impl Into<String>,
        transform_version: impl Into<String>,
    ) -> Self {
        Self {
            graph,
            output,
            wrt,
            seed,
            op_contract_version: op_contract_version.into(),
            transform_version: transform_version.into(),
        }
    }
}

impl de::FromStream for AutodiffRequest {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: Self::Context,
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        struct Visitor;

        impl de::Visitor for Visitor {
            type Value = AutodiffRequest;

            fn expecting() -> &'static str {
                "an AutodiffRequest map"
            }

            async fn visit_map<A: de::MapAccess>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut graph = None;
                let mut output = None;
                let mut wrt = None;
                let mut seed = None;
                let mut op_contract_version = None;
                let mut transform_version = None;

                while let Some(key) = map.next_key::<String>(()).await? {
                    match key.as_str() {
                        "graph" => graph = Some(map.next_value::<TensorGraph>(()).await?),
                        "output" => output = Some(map.next_value::<ValueId>(()).await?),
                        "wrt" => wrt = Some(map.next_value::<Vec<ValueId>>(()).await?),
                        "seed" => seed = Some(map.next_value::<ValueId>(()).await?),
                        "op_contract_version" => {
                            op_contract_version = Some(map.next_value::<String>(()).await?)
                        }
                        "transform_version" => {
                            transform_version = Some(map.next_value::<String>(()).await?)
                        }
                        _ => {
                            let _ = map.next_value::<de::IgnoredAny>(()).await?;
                        }
                    }
                }

                Ok(AutodiffRequest {
                    graph: graph.ok_or_else(|| de::Error::custom("missing graph"))?,
                    output: output.ok_or_else(|| de::Error::custom("missing output"))?,
                    wrt: wrt.ok_or_else(|| de::Error::custom("missing wrt"))?,
                    seed: seed.ok_or_else(|| de::Error::custom("missing seed"))?,
                    op_contract_version: op_contract_version
                        .ok_or_else(|| de::Error::custom("missing op_contract_version"))?,
                    transform_version: transform_version
                        .ok_or_else(|| de::Error::custom("missing transform_version"))?,
                })
            }
        }

        decoder.decode_map(Visitor).await
    }
}

impl<'en> en::IntoStream<'en> for AutodiffRequest {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        let mut map = encoder.encode_map(Some(6))?;
        map.encode_entry("graph", self.graph)?;
        map.encode_entry("output", self.output)?;
        map.encode_entry("wrt", self.wrt)?;
        map.encode_entry("seed", self.seed)?;
        map.encode_entry("op_contract_version", self.op_contract_version)?;
        map.encode_entry("transform_version", self.transform_version)?;
        map.end()
    }
}

impl<'en> en::ToStream<'en> for AutodiffRequest {
    fn to_stream<E: en::Encoder<'en>>(&'en self, encoder: E) -> Result<E::Ok, E::Error> {
        self.clone().into_stream(encoder)
    }
}

// ---------------------------------------------------------------------------
// AutodiffResult
// ---------------------------------------------------------------------------

/// Result of a successful autodiff transform: gradient values and their types.
///
/// Wire format: `{"gradients": [{"value_id": "...", "type_spec": {...}}, ...]}`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutodiffResult {
    pub gradients: Vec<(ValueId, TensorTypeSpec)>,
}

impl AutodiffResult {
    pub fn new(gradients: Vec<(ValueId, TensorTypeSpec)>) -> Self {
        Self { gradients }
    }
}

struct GradientEntryEncoder {
    vid: ValueId,
    tspec: TensorTypeSpec,
}

impl<'en> en::IntoStream<'en> for GradientEntryEncoder {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        let mut map = encoder.encode_map(Some(2))?;
        map.encode_entry("value_id", self.vid)?;
        map.encode_entry("type_spec", self.tspec)?;
        map.end()
    }
}

struct GradientEntry {
    value_id: ValueId,
    type_spec: TensorTypeSpec,
}

impl de::FromStream for GradientEntry {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: Self::Context,
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        struct GradVisitor;

        impl de::Visitor for GradVisitor {
            type Value = GradientEntry;

            fn expecting() -> &'static str {
                "a gradient entry map with value_id and type_spec"
            }

            async fn visit_map<A: de::MapAccess>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut value_id = None;
                let mut type_spec = None;

                while let Some(key) = map.next_key::<String>(()).await? {
                    match key.as_str() {
                        "value_id" => value_id = Some(map.next_value::<ValueId>(()).await?),
                        "type_spec" => {
                            type_spec = Some(map.next_value::<TensorTypeSpec>(()).await?)
                        }
                        _ => {
                            let _ = map.next_value::<de::IgnoredAny>(()).await?;
                        }
                    }
                }

                Ok(GradientEntry {
                    value_id: value_id.ok_or_else(|| de::Error::custom("missing value_id"))?,
                    type_spec: type_spec.ok_or_else(|| de::Error::custom("missing type_spec"))?,
                })
            }
        }

        decoder.decode_map(GradVisitor).await
    }
}

struct GradientsEncoder(Vec<(ValueId, TensorTypeSpec)>);

impl<'en> en::IntoStream<'en> for GradientsEncoder {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        use destream::en::EncodeSeq;
        let mut seq = encoder.encode_seq(Some(self.0.len()))?;
        for (vid, tspec) in self.0 {
            seq.encode_element(GradientEntryEncoder { vid, tspec })?;
        }
        seq.end()
    }
}

impl de::FromStream for AutodiffResult {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: Self::Context,
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        struct Visitor;

        impl de::Visitor for Visitor {
            type Value = AutodiffResult;

            fn expecting() -> &'static str {
                "an AutodiffResult map"
            }

            async fn visit_map<A: de::MapAccess>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut gradients: Option<Vec<GradientEntry>> = None;

                while let Some(key) = map.next_key::<String>(()).await? {
                    match key.as_str() {
                        "gradients" => {
                            gradients = Some(map.next_value::<Vec<GradientEntry>>(()).await?);
                        }
                        _ => {
                            let _ = map.next_value::<de::IgnoredAny>(()).await?;
                        }
                    }
                }

                let gradients = gradients
                    .unwrap_or_default()
                    .into_iter()
                    .map(|e| (e.value_id, e.type_spec))
                    .collect();

                Ok(AutodiffResult { gradients })
            }
        }

        decoder.decode_map(Visitor).await
    }
}

impl<'en> en::IntoStream<'en> for AutodiffResult {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        let mut map = encoder.encode_map(Some(1))?;
        map.encode_entry("gradients", GradientsEncoder(self.gradients))?;
        map.end()
    }
}

impl<'en> en::ToStream<'en> for AutodiffResult {
    fn to_stream<E: en::Encoder<'en>>(&'en self, encoder: E) -> Result<E::Ok, E::Error> {
        self.clone().into_stream(encoder)
    }
}

// ---------------------------------------------------------------------------
// DerivativeMetadata
// ---------------------------------------------------------------------------

/// Metadata describing the derivative contract for a specific operator and wrt signature.
///
/// Wire format: a JSON map with fields:
/// `source_library_id`, `source_op`, `transform_version`, `wrt_signature`, `seed_contract`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DerivativeMetadata {
    pub source_library_id: String,
    pub source_op: TensorOp,
    pub transform_version: String,
    pub wrt_signature: Vec<TensorTypeSpec>,
    pub seed_contract: TensorTypeSpec,
}

impl DerivativeMetadata {
    pub fn new(
        source_library_id: impl Into<String>,
        source_op: TensorOp,
        transform_version: impl Into<String>,
        wrt_signature: Vec<TensorTypeSpec>,
        seed_contract: TensorTypeSpec,
    ) -> Self {
        Self {
            source_library_id: source_library_id.into(),
            source_op,
            transform_version: transform_version.into(),
            wrt_signature,
            seed_contract,
        }
    }
}

impl de::FromStream for DerivativeMetadata {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: Self::Context,
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        struct Visitor;

        impl de::Visitor for Visitor {
            type Value = DerivativeMetadata;

            fn expecting() -> &'static str {
                "a DerivativeMetadata map"
            }

            async fn visit_map<A: de::MapAccess>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut source_library_id = None;
                let mut source_op = None;
                let mut transform_version = None;
                let mut wrt_signature = None;
                let mut seed_contract = None;

                while let Some(key) = map.next_key::<String>(()).await? {
                    match key.as_str() {
                        "source_library_id" => {
                            source_library_id = Some(map.next_value::<String>(()).await?)
                        }
                        "source_op" => source_op = Some(map.next_value::<TensorOp>(()).await?),
                        "transform_version" => {
                            transform_version = Some(map.next_value::<String>(()).await?)
                        }
                        "wrt_signature" => {
                            wrt_signature =
                                Some(map.next_value::<Vec<TensorTypeSpec>>(()).await?)
                        }
                        "seed_contract" => {
                            seed_contract = Some(map.next_value::<TensorTypeSpec>(()).await?)
                        }
                        _ => {
                            let _ = map.next_value::<de::IgnoredAny>(()).await?;
                        }
                    }
                }

                Ok(DerivativeMetadata {
                    source_library_id: source_library_id
                        .ok_or_else(|| de::Error::custom("missing source_library_id"))?,
                    source_op: source_op
                        .ok_or_else(|| de::Error::custom("missing source_op"))?,
                    transform_version: transform_version
                        .ok_or_else(|| de::Error::custom("missing transform_version"))?,
                    wrt_signature: wrt_signature.unwrap_or_default(),
                    seed_contract: seed_contract
                        .ok_or_else(|| de::Error::custom("missing seed_contract"))?,
                })
            }
        }

        decoder.decode_map(Visitor).await
    }
}

impl<'en> en::IntoStream<'en> for DerivativeMetadata {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        let mut map = encoder.encode_map(Some(5))?;
        map.encode_entry("source_library_id", self.source_library_id)?;
        map.encode_entry("source_op", self.source_op)?;
        map.encode_entry("transform_version", self.transform_version)?;
        map.encode_entry("wrt_signature", self.wrt_signature)?;
        map.encode_entry("seed_contract", self.seed_contract)?;
        map.end()
    }
}

impl<'en> en::ToStream<'en> for DerivativeMetadata {
    fn to_stream<E: en::Encoder<'en>>(&'en self, encoder: E) -> Result<E::Ok, E::Error> {
        self.clone().into_stream(encoder)
    }
}

// ---------------------------------------------------------------------------
// AutodiffError — all 14 transform-time error categories (spec section 19.1)
// ---------------------------------------------------------------------------

/// All transform-time error categories for the autodiff IR transform.
///
/// Encoded as a string (snake_case variant name).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AutodiffError {
    /// Operator has no VJP rule registered.
    UnsupportedOperator,
    /// Derivative IR is absent from the registry for this operator/version.
    MissingDerivativeIr,
    /// The tensor dtype is not differentiable (e.g. integer or boolean).
    DtypeNotDifferentiable,
    /// Output and seed shapes are incompatible.
    ShapeMismatch,
    /// A requested `wrt` value is not an input to the graph.
    WrtNotInGraph,
    /// The requested `output` value is not produced by any node.
    OutputNotInGraph,
    /// Seed type does not match the declared output type.
    SeedTypeMismatch,
    /// Graph contains a cycle (non-DAG).
    CycleInGraph,
    /// Transpose permutation is invalid (wrong length or duplicate axis).
    InvalidPermutation,
    /// Operand shapes are not broadcastable.
    InvalidBroadcast,
    /// Operand rank is too low for matmul (must be ≥ 2).
    RankTooLow,
    /// Matmul inner dimensions do not match.
    InnerDimMismatch,
    /// The `wrt` list is empty — no differentiation targets.
    EmptyWrt,
    /// Operator contract version does not match transform version.
    ContractVersionMismatch,
}

impl AutodiffError {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::UnsupportedOperator => "unsupported_operator",
            Self::MissingDerivativeIr => "missing_derivative_ir",
            Self::DtypeNotDifferentiable => "dtype_not_differentiable",
            Self::ShapeMismatch => "shape_mismatch",
            Self::WrtNotInGraph => "wrt_not_in_graph",
            Self::OutputNotInGraph => "output_not_in_graph",
            Self::SeedTypeMismatch => "seed_type_mismatch",
            Self::CycleInGraph => "cycle_in_graph",
            Self::InvalidPermutation => "invalid_permutation",
            Self::InvalidBroadcast => "invalid_broadcast",
            Self::RankTooLow => "rank_too_low",
            Self::InnerDimMismatch => "inner_dim_mismatch",
            Self::EmptyWrt => "empty_wrt",
            Self::ContractVersionMismatch => "contract_version_mismatch",
        }
    }
}

impl fmt::Display for AutodiffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::error::Error for AutodiffError {}

impl de::FromStream for AutodiffError {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: Self::Context,
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        let s = String::from_stream((), decoder).await?;
        match s.as_str() {
            "unsupported_operator" => Ok(Self::UnsupportedOperator),
            "missing_derivative_ir" => Ok(Self::MissingDerivativeIr),
            "dtype_not_differentiable" => Ok(Self::DtypeNotDifferentiable),
            "shape_mismatch" => Ok(Self::ShapeMismatch),
            "wrt_not_in_graph" => Ok(Self::WrtNotInGraph),
            "output_not_in_graph" => Ok(Self::OutputNotInGraph),
            "seed_type_mismatch" => Ok(Self::SeedTypeMismatch),
            "cycle_in_graph" => Ok(Self::CycleInGraph),
            "invalid_permutation" => Ok(Self::InvalidPermutation),
            "invalid_broadcast" => Ok(Self::InvalidBroadcast),
            "rank_too_low" => Ok(Self::RankTooLow),
            "inner_dim_mismatch" => Ok(Self::InnerDimMismatch),
            "empty_wrt" => Ok(Self::EmptyWrt),
            "contract_version_mismatch" => Ok(Self::ContractVersionMismatch),
            other => Err(de::Error::custom(format!("unknown AutodiffError: {other}"))),
        }
    }
}

impl<'en> en::IntoStream<'en> for AutodiffError {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        encoder.encode_str(self.as_str())
    }
}

impl<'en> en::ToStream<'en> for AutodiffError {
    fn to_stream<E: en::Encoder<'en>>(&'en self, encoder: E) -> Result<E::Ok, E::Error> {
        encoder.encode_str(self.as_str())
    }
}
