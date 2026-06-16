use std::fmt;

use destream::{de, en, EncodeMap, EncodeSeq, IntoStream};

/// Error produced by tensor IR shape inference and validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TensorIrError(String);

impl TensorIrError {
    pub fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}

impl fmt::Display for TensorIrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for TensorIrError {}

// ---------------------------------------------------------------------------
// NodeId / ValueId
// ---------------------------------------------------------------------------

/// Opaque identity for a node in a tensor computation graph.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(String);

impl NodeId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl de::FromStream for NodeId {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: Self::Context,
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        let s = String::from_stream((), decoder).await?;
        Ok(Self(s))
    }
}

impl<'en> en::IntoStream<'en> for NodeId {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        encoder.encode_str(&self.0)
    }
}

impl<'en> en::ToStream<'en> for NodeId {
    fn to_stream<E: en::Encoder<'en>>(&'en self, encoder: E) -> Result<E::Ok, E::Error> {
        encoder.encode_str(&self.0)
    }
}

/// Opaque identity for a value (edge) in a tensor computation graph.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ValueId(String);

impl ValueId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ValueId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl de::FromStream for ValueId {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: Self::Context,
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        let s = String::from_stream((), decoder).await?;
        Ok(Self(s))
    }
}

impl<'en> en::IntoStream<'en> for ValueId {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        encoder.encode_str(&self.0)
    }
}

impl<'en> en::ToStream<'en> for ValueId {
    fn to_stream<E: en::Encoder<'en>>(&'en self, encoder: E) -> Result<E::Ok, E::Error> {
        encoder.encode_str(&self.0)
    }
}

// ---------------------------------------------------------------------------
// TensorDtype
// ---------------------------------------------------------------------------

/// Floating-point dtype for differentiable tensors.
///
/// Only `F32` and `F64` are differentiable in Phase 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TensorDtype {
    F32,
    F64,
}

impl TensorDtype {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::F32 => "f32",
            Self::F64 => "f64",
        }
    }
}

impl fmt::Display for TensorDtype {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl de::FromStream for TensorDtype {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: Self::Context,
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        let s = String::from_stream((), decoder).await?;
        match s.as_str() {
            "f32" => Ok(Self::F32),
            "f64" => Ok(Self::F64),
            other => Err(de::Error::custom(format!("unknown TensorDtype: {other}"))),
        }
    }
}

impl<'en> en::IntoStream<'en> for TensorDtype {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        encoder.encode_str(self.as_str())
    }
}

impl<'en> en::ToStream<'en> for TensorDtype {
    fn to_stream<E: en::Encoder<'en>>(&'en self, encoder: E) -> Result<E::Ok, E::Error> {
        encoder.encode_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// TensorTypeSpec
// ---------------------------------------------------------------------------

/// Type specification for a tensor value: dtype and shape.
///
/// Shape dimensions are `Option<usize>` where `None` is a dynamic (unknown) dimension.
/// Wire format: `{"dtype": "f32", "shape": [2, -1, 4]}` where -1 means dynamic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TensorTypeSpec {
    pub dtype: TensorDtype,
    pub shape: Vec<Option<usize>>,
}

impl TensorTypeSpec {
    pub fn new(dtype: TensorDtype, shape: Vec<Option<usize>>) -> Self {
        Self { dtype, shape }
    }

    pub fn rank(&self) -> usize {
        self.shape.len()
    }
}

/// Wire representation of a single shape dimension.
///
/// `None` (dynamic) is encoded as -1; known dims are encoded as their unsigned value.
struct DimSize(Option<usize>);

impl de::FromStream for DimSize {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: Self::Context,
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        struct DimVisitor;

        impl de::Visitor for DimVisitor {
            type Value = DimSize;

            fn expecting() -> &'static str {
                "a dimension size (u64) or -1 for dynamic"
            }

            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                if v < 0 {
                    Ok(DimSize(None))
                } else {
                    Ok(DimSize(Some(v as usize)))
                }
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(DimSize(Some(v as usize)))
            }
        }

        decoder.decode_any(DimVisitor).await
    }
}

struct ShapeEncoder(Vec<Option<usize>>);

impl<'en> en::IntoStream<'en> for ShapeEncoder {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        let mut seq = encoder.encode_seq(Some(self.0.len()))?;
        for dim in self.0 {
            match dim {
                Some(n) => seq.encode_element(n as u64)?,
                None => seq.encode_element(-1i64)?,
            }
        }
        seq.end()
    }
}

impl de::FromStream for TensorTypeSpec {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: Self::Context,
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        struct TypeSpecVisitor;

        impl de::Visitor for TypeSpecVisitor {
            type Value = TensorTypeSpec;

            fn expecting() -> &'static str {
                "a TensorTypeSpec map"
            }

            async fn visit_map<A: de::MapAccess>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut dtype = None;
                let mut shape = None;

                while let Some(key) = map.next_key::<String>(()).await? {
                    match key.as_str() {
                        "dtype" => {
                            dtype = Some(map.next_value::<TensorDtype>(()).await?);
                        }
                        "shape" => {
                            let dims = map.next_value::<Vec<DimSize>>(()).await?;
                            shape = Some(dims.into_iter().map(|d| d.0).collect::<Vec<_>>());
                        }
                        _ => {
                            let _ = map.next_value::<de::IgnoredAny>(()).await?;
                        }
                    }
                }

                Ok(TensorTypeSpec {
                    dtype: dtype.ok_or_else(|| de::Error::custom("missing dtype"))?,
                    shape: shape.unwrap_or_default(),
                })
            }
        }

        decoder.decode_map(TypeSpecVisitor).await
    }
}

impl<'en> en::IntoStream<'en> for TensorTypeSpec {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        let mut map = encoder.encode_map(Some(2))?;
        map.encode_entry("dtype", self.dtype)?;
        map.encode_entry("shape", ShapeEncoder(self.shape))?;
        map.end()
    }
}

impl<'en> en::ToStream<'en> for TensorTypeSpec {
    fn to_stream<E: en::Encoder<'en>>(&'en self, encoder: E) -> Result<E::Ok, E::Error> {
        self.clone().into_stream(encoder)
    }
}

// ---------------------------------------------------------------------------
// TensorOp
// ---------------------------------------------------------------------------

/// Canonical tensor operator identity.
///
/// Wire format — single-entry map:
/// - `{"add":       {"lhs": "<vid>", "rhs": "<vid>"}}`
/// - `{"matmul":    {"lhs": "<vid>", "rhs": "<vid>"}}`
/// - `{"transpose": {"input": "<vid>", "perm": [0, 2, 1]}}`
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TensorOp {
    Add { lhs: ValueId, rhs: ValueId },
    Matmul { lhs: ValueId, rhs: ValueId },
    Transpose { input: ValueId, perm: Vec<usize> },
}

impl TensorOp {
    pub fn variant_name(&self) -> &'static str {
        match self {
            Self::Add { .. } => "add",
            Self::Matmul { .. } => "matmul",
            Self::Transpose { .. } => "transpose",
        }
    }
}

// Helper: binary op args (shared by Add and Matmul)
struct BinaryOpArgs {
    lhs: ValueId,
    rhs: ValueId,
}

impl de::FromStream for BinaryOpArgs {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: Self::Context,
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        struct ArgsVisitor;

        impl de::Visitor for ArgsVisitor {
            type Value = BinaryOpArgs;

            fn expecting() -> &'static str {
                "a map with lhs and rhs ValueId fields"
            }

            async fn visit_map<A: de::MapAccess>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut lhs = None;
                let mut rhs = None;

                while let Some(key) = map.next_key::<String>(()).await? {
                    match key.as_str() {
                        "lhs" => lhs = Some(map.next_value::<ValueId>(()).await?),
                        "rhs" => rhs = Some(map.next_value::<ValueId>(()).await?),
                        _ => {
                            let _ = map.next_value::<de::IgnoredAny>(()).await?;
                        }
                    }
                }

                Ok(BinaryOpArgs {
                    lhs: lhs.ok_or_else(|| de::Error::custom("missing lhs"))?,
                    rhs: rhs.ok_or_else(|| de::Error::custom("missing rhs"))?,
                })
            }
        }

        decoder.decode_map(ArgsVisitor).await
    }
}

impl<'en> en::IntoStream<'en> for BinaryOpArgs {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        let mut map = encoder.encode_map(Some(2))?;
        map.encode_entry("lhs", self.lhs)?;
        map.encode_entry("rhs", self.rhs)?;
        map.end()
    }
}

// Helper: transpose args
struct TransposeArgs {
    input: ValueId,
    perm: Vec<usize>,
}

impl de::FromStream for TransposeArgs {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: Self::Context,
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        struct ArgsVisitor;

        impl de::Visitor for ArgsVisitor {
            type Value = TransposeArgs;

            fn expecting() -> &'static str {
                "a map with input and perm fields"
            }

            async fn visit_map<A: de::MapAccess>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut input = None;
                let mut perm: Option<Vec<usize>> = None;

                while let Some(key) = map.next_key::<String>(()).await? {
                    match key.as_str() {
                        "input" => input = Some(map.next_value::<ValueId>(()).await?),
                        "perm" => {
                            let raw = map.next_value::<Vec<u64>>(()).await?;
                            perm = Some(raw.into_iter().map(|n| n as usize).collect());
                        }
                        _ => {
                            let _ = map.next_value::<de::IgnoredAny>(()).await?;
                        }
                    }
                }

                Ok(TransposeArgs {
                    input: input.ok_or_else(|| de::Error::custom("missing input"))?,
                    perm: perm.ok_or_else(|| de::Error::custom("missing perm"))?,
                })
            }
        }

        decoder.decode_map(ArgsVisitor).await
    }
}

struct PermEncoder(Vec<usize>);

impl<'en> en::IntoStream<'en> for PermEncoder {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        let mut seq = encoder.encode_seq(Some(self.0.len()))?;
        for axis in self.0 {
            seq.encode_element(axis as u64)?;
        }
        seq.end()
    }
}

impl<'en> en::IntoStream<'en> for TransposeArgs {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        let mut map = encoder.encode_map(Some(2))?;
        map.encode_entry("input", self.input)?;
        map.encode_entry("perm", PermEncoder(self.perm))?;
        map.end()
    }
}

impl de::FromStream for TensorOp {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: Self::Context,
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        struct TensorOpVisitor;

        impl de::Visitor for TensorOpVisitor {
            type Value = TensorOp;

            fn expecting() -> &'static str {
                "a TensorOp single-entry map"
            }

            async fn visit_map<A: de::MapAccess>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let variant = map
                    .next_key::<String>(())
                    .await?
                    .ok_or_else(|| de::Error::custom("expected TensorOp variant, got empty map"))?;

                let op = match variant.as_str() {
                    "add" => {
                        let args = map.next_value::<BinaryOpArgs>(()).await?;
                        TensorOp::Add {
                            lhs: args.lhs,
                            rhs: args.rhs,
                        }
                    }
                    "matmul" => {
                        let args = map.next_value::<BinaryOpArgs>(()).await?;
                        TensorOp::Matmul {
                            lhs: args.lhs,
                            rhs: args.rhs,
                        }
                    }
                    "transpose" => {
                        let args = map.next_value::<TransposeArgs>(()).await?;
                        TensorOp::Transpose {
                            input: args.input,
                            perm: args.perm,
                        }
                    }
                    other => {
                        return Err(de::Error::custom(format!("unknown TensorOp: {other}")));
                    }
                };

                while map.next_key::<de::IgnoredAny>(()).await?.is_some() {
                    let _ = map.next_value::<de::IgnoredAny>(()).await?;
                }

                Ok(op)
            }
        }

        decoder.decode_map(TensorOpVisitor).await
    }
}

impl<'en> en::IntoStream<'en> for TensorOp {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        let mut map = encoder.encode_map(Some(1))?;
        match self {
            TensorOp::Add { lhs, rhs } => {
                map.encode_entry("add", BinaryOpArgs { lhs, rhs })?;
            }
            TensorOp::Matmul { lhs, rhs } => {
                map.encode_entry("matmul", BinaryOpArgs { lhs, rhs })?;
            }
            TensorOp::Transpose { input, perm } => {
                map.encode_entry("transpose", TransposeArgs { input, perm })?;
            }
        }
        map.end()
    }
}

impl<'en> en::ToStream<'en> for TensorOp {
    fn to_stream<E: en::Encoder<'en>>(&'en self, encoder: E) -> Result<E::Ok, E::Error> {
        self.clone().into_stream(encoder)
    }
}

// ---------------------------------------------------------------------------
// TensorNode
// ---------------------------------------------------------------------------

/// A single node in a typed tensor computation graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TensorNode {
    pub id: NodeId,
    pub output: ValueId,
    pub op: TensorOp,
    pub output_type: TensorTypeSpec,
}

impl TensorNode {
    pub fn new(id: NodeId, output: ValueId, op: TensorOp, output_type: TensorTypeSpec) -> Self {
        Self {
            id,
            output,
            op,
            output_type,
        }
    }
}

impl de::FromStream for TensorNode {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: Self::Context,
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        struct NodeVisitor;

        impl de::Visitor for NodeVisitor {
            type Value = TensorNode;

            fn expecting() -> &'static str {
                "a TensorNode map"
            }

            async fn visit_map<A: de::MapAccess>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut id = None;
                let mut output = None;
                let mut op = None;
                let mut output_type = None;

                while let Some(key) = map.next_key::<String>(()).await? {
                    match key.as_str() {
                        "id" => id = Some(map.next_value::<NodeId>(()).await?),
                        "output" => output = Some(map.next_value::<ValueId>(()).await?),
                        "op" => op = Some(map.next_value::<TensorOp>(()).await?),
                        "output_type" => {
                            output_type = Some(map.next_value::<TensorTypeSpec>(()).await?)
                        }
                        _ => {
                            let _ = map.next_value::<de::IgnoredAny>(()).await?;
                        }
                    }
                }

                Ok(TensorNode {
                    id: id.ok_or_else(|| de::Error::custom("missing id"))?,
                    output: output.ok_or_else(|| de::Error::custom("missing output"))?,
                    op: op.ok_or_else(|| de::Error::custom("missing op"))?,
                    output_type: output_type
                        .ok_or_else(|| de::Error::custom("missing output_type"))?,
                })
            }
        }

        decoder.decode_map(NodeVisitor).await
    }
}

impl<'en> en::IntoStream<'en> for TensorNode {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        let mut map = encoder.encode_map(Some(4))?;
        map.encode_entry("id", self.id)?;
        map.encode_entry("output", self.output)?;
        map.encode_entry("op", self.op)?;
        map.encode_entry("output_type", self.output_type)?;
        map.end()
    }
}

impl<'en> en::ToStream<'en> for TensorNode {
    fn to_stream<E: en::Encoder<'en>>(&'en self, encoder: E) -> Result<E::Ok, E::Error> {
        self.clone().into_stream(encoder)
    }
}

// ---------------------------------------------------------------------------
// TensorGraph
// ---------------------------------------------------------------------------

/// A typed tensor computation graph.
///
/// - `inputs`: declared input values with their types.
/// - `outputs`: ordered output value IDs.
/// - `nodes`: topologically ordered computation nodes.
///
/// Wire format: `{"inputs": [{"value_id": "...", "type_spec": {...}}], "outputs": [...], "nodes": [...]}`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TensorGraph {
    pub inputs: Vec<(ValueId, TensorTypeSpec)>,
    pub outputs: Vec<ValueId>,
    pub nodes: Vec<TensorNode>,
}

impl TensorGraph {
    pub fn new(
        inputs: Vec<(ValueId, TensorTypeSpec)>,
        outputs: Vec<ValueId>,
        nodes: Vec<TensorNode>,
    ) -> Self {
        Self {
            inputs,
            outputs,
            nodes,
        }
    }
}

struct InputPairEncoder {
    vid: ValueId,
    tspec: TensorTypeSpec,
}

impl<'en> en::IntoStream<'en> for InputPairEncoder {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        let mut map = encoder.encode_map(Some(2))?;
        map.encode_entry("value_id", self.vid)?;
        map.encode_entry("type_spec", self.tspec)?;
        map.end()
    }
}

struct GraphInputsEncoder(Vec<(ValueId, TensorTypeSpec)>);

impl<'en> en::IntoStream<'en> for GraphInputsEncoder {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        let mut seq = encoder.encode_seq(Some(self.0.len()))?;
        for (vid, tspec) in self.0 {
            seq.encode_element(InputPairEncoder { vid, tspec })?;
        }
        seq.end()
    }
}

struct InputPair {
    value_id: ValueId,
    type_spec: TensorTypeSpec,
}

impl de::FromStream for InputPair {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: Self::Context,
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        struct PairVisitor;

        impl de::Visitor for PairVisitor {
            type Value = InputPair;

            fn expecting() -> &'static str {
                "an input pair map with value_id and type_spec"
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

                Ok(InputPair {
                    value_id: value_id.ok_or_else(|| de::Error::custom("missing value_id"))?,
                    type_spec: type_spec.ok_or_else(|| de::Error::custom("missing type_spec"))?,
                })
            }
        }

        decoder.decode_map(PairVisitor).await
    }
}

impl de::FromStream for TensorGraph {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: Self::Context,
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        struct GraphVisitor;

        impl de::Visitor for GraphVisitor {
            type Value = TensorGraph;

            fn expecting() -> &'static str {
                "a TensorGraph map"
            }

            async fn visit_map<A: de::MapAccess>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut inputs: Option<Vec<InputPair>> = None;
                let mut outputs = None;
                let mut nodes = None;

                while let Some(key) = map.next_key::<String>(()).await? {
                    match key.as_str() {
                        "inputs" => {
                            inputs = Some(map.next_value::<Vec<InputPair>>(()).await?);
                        }
                        "outputs" => {
                            outputs = Some(map.next_value::<Vec<ValueId>>(()).await?);
                        }
                        "nodes" => {
                            nodes = Some(map.next_value::<Vec<TensorNode>>(()).await?);
                        }
                        _ => {
                            let _ = map.next_value::<de::IgnoredAny>(()).await?;
                        }
                    }
                }

                let inputs = inputs
                    .unwrap_or_default()
                    .into_iter()
                    .map(|p| (p.value_id, p.type_spec))
                    .collect();

                Ok(TensorGraph {
                    inputs,
                    outputs: outputs.unwrap_or_default(),
                    nodes: nodes.unwrap_or_default(),
                })
            }
        }

        decoder.decode_map(GraphVisitor).await
    }
}

impl<'en> en::IntoStream<'en> for TensorGraph {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        let mut map = encoder.encode_map(Some(3))?;
        map.encode_entry("inputs", GraphInputsEncoder(self.inputs))?;
        map.encode_entry("outputs", self.outputs)?;
        map.encode_entry("nodes", self.nodes)?;
        map.end()
    }
}

impl<'en> en::ToStream<'en> for TensorGraph {
    fn to_stream<E: en::Encoder<'en>>(&'en self, encoder: E) -> Result<E::Ok, E::Error> {
        self.clone().into_stream(encoder)
    }
}

// ---------------------------------------------------------------------------
// Shape inference
// ---------------------------------------------------------------------------

/// Broadcasting shape inference per NumPy/ONNX right-aligned semantics.
///
/// Rules:
/// - Shapes are aligned on the right; the shorter one is extended with leading 1s.
/// - Dimensions are compatible if: equal, one is 1, or at least one is dynamic (`None`).
/// - Returns `Err` when two concrete dims are both > 1 and differ.
pub fn broadcast_shapes(
    a: &[Option<usize>],
    b: &[Option<usize>],
) -> Result<Vec<Option<usize>>, TensorIrError> {
    let out_rank = a.len().max(b.len());
    let mut result = Vec::with_capacity(out_rank);

    for i in 0..out_rank {
        let pad_a = out_rank - a.len();
        let pad_b = out_rank - b.len();

        let da = if i < pad_a { Some(1usize) } else { a[i - pad_a] };
        let db = if i < pad_b { Some(1usize) } else { b[i - pad_b] };

        let dim = match (da, db) {
            (None, _) | (_, None) => None,
            (Some(x), Some(y)) => {
                if x == y {
                    Some(x)
                } else if x == 1 {
                    Some(y)
                } else if y == 1 {
                    Some(x)
                } else {
                    return Err(TensorIrError::new(format!(
                        "shapes are not broadcastable: dim {x} vs {y} at output axis {i}"
                    )));
                }
            }
        };

        result.push(dim);
    }

    Ok(result)
}

/// Batched matmul output shape inference (spec section 13.3).
///
/// Requirements:
/// - Both operands must have rank ≥ 2.
/// - Inner dims `a[rank-1]` and `b[rank-2]` must match (or at least one is dynamic).
/// - Batch dims (all but last two) are broadcast-compatible.
/// - Result shape: `broadcast(a_batch, b_batch) ++ [a[-2], b[-1]]`.
pub fn matmul_output_shape(
    a: &[Option<usize>],
    b: &[Option<usize>],
) -> Result<Vec<Option<usize>>, TensorIrError> {
    if a.len() < 2 {
        return Err(TensorIrError::new(format!(
            "matmul: left operand must have rank ≥ 2, got rank {}",
            a.len()
        )));
    }
    if b.len() < 2 {
        return Err(TensorIrError::new(format!(
            "matmul: right operand must have rank ≥ 2, got rank {}",
            b.len()
        )));
    }

    let a_inner = a[a.len() - 1];
    let b_inner = b[b.len() - 2];

    if let (Some(x), Some(y)) = (a_inner, b_inner) {
        if x != y {
            return Err(TensorIrError::new(format!(
                "matmul: inner dimensions do not match: {x} vs {y}"
            )));
        }
    }

    let a_batch = &a[..a.len() - 2];
    let b_batch = &b[..b.len() - 2];
    let mut out = broadcast_shapes(a_batch, b_batch)?;

    out.push(a[a.len() - 2]);
    out.push(b[b.len() - 1]);

    Ok(out)
}

/// Permutation validation for transpose (spec section 13.4).
///
/// A valid permutation must:
/// - Have exactly `rank` elements.
/// - Contain each axis index in `0..rank` exactly once.
pub fn validate_permutation(perm: &[usize], rank: usize) -> Result<(), TensorIrError> {
    if perm.len() != rank {
        return Err(TensorIrError::new(format!(
            "permutation length {} does not match rank {}",
            perm.len(),
            rank
        )));
    }

    let mut seen = vec![false; rank];
    for &axis in perm {
        if axis >= rank {
            return Err(TensorIrError::new(format!(
                "permutation axis {axis} out of range for rank {rank}"
            )));
        }
        if seen[axis] {
            return Err(TensorIrError::new(format!(
                "axis {axis} appears more than once in permutation"
            )));
        }
        seen[axis] = true;
    }

    Ok(())
}
