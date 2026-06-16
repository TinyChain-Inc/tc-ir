use std::fmt;

use destream::{de, en, EncodeMap, EncodeSeq, IntoStream};

// NOTE: tc-ir intentionally does not define AutodiffRequest, AutodiffResult,
// DerivativeMetadata, or AutodiffError. Those types are client-owned per FR-004.

// ====== NodeId ======

/// Unique identity for a computation node in a [`TensorGraph`].
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
        self.0.fmt(f)
    }
}

impl de::FromStream for NodeId {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: (),
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        struct NodeIdVisitor;

        impl de::Visitor for NodeIdVisitor {
            type Value = NodeId;

            fn expecting() -> &'static str {
                "a node id string"
            }

            fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(NodeId(v))
            }
        }

        decoder.decode_string(NodeIdVisitor).await
    }
}

impl<'en> en::IntoStream<'en> for NodeId {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        self.0.into_stream(encoder)
    }
}

impl<'en> en::ToStream<'en> for NodeId {
    fn to_stream<E: en::Encoder<'en>>(&'en self, encoder: E) -> Result<E::Ok, E::Error> {
        self.0.as_str().into_stream(encoder)
    }
}

// ====== ValueId ======

/// Unique identity for a value (tensor) produced by a node in a [`TensorGraph`].
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
        self.0.fmt(f)
    }
}

impl de::FromStream for ValueId {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: (),
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        struct ValueIdVisitor;

        impl de::Visitor for ValueIdVisitor {
            type Value = ValueId;

            fn expecting() -> &'static str {
                "a value id string"
            }

            fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(ValueId(v))
            }
        }

        decoder.decode_string(ValueIdVisitor).await
    }
}

impl<'en> en::IntoStream<'en> for ValueId {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        self.0.into_stream(encoder)
    }
}

impl<'en> en::ToStream<'en> for ValueId {
    fn to_stream<E: en::Encoder<'en>>(&'en self, encoder: E) -> Result<E::Ok, E::Error> {
        self.0.as_str().into_stream(encoder)
    }
}

// ====== TensorDtype ======

/// Floating-point element type supported for tensor operations in Phase 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TensorDtype {
    F32,
    F64,
}

impl TensorDtype {
    fn as_str(self) -> &'static str {
        match self {
            Self::F32 => "f32",
            Self::F64 => "f64",
        }
    }
}

impl de::FromStream for TensorDtype {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: (),
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        struct DtypeVisitor;

        impl de::Visitor for DtypeVisitor {
            type Value = TensorDtype;

            fn expecting() -> &'static str {
                "a tensor dtype string (\"f32\" or \"f64\")"
            }

            fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
                match v.as_str() {
                    "f32" => Ok(TensorDtype::F32),
                    "f64" => Ok(TensorDtype::F64),
                    other => Err(de::Error::custom(format!(
                        "unknown tensor dtype: {other:?} (expected \"f32\" or \"f64\")"
                    ))),
                }
            }
        }

        decoder.decode_string(DtypeVisitor).await
    }
}

impl<'en> en::IntoStream<'en> for TensorDtype {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        self.as_str().into_stream(encoder)
    }
}

impl<'en> en::ToStream<'en> for TensorDtype {
    fn to_stream<E: en::Encoder<'en>>(&'en self, encoder: E) -> Result<E::Ok, E::Error> {
        self.as_str().into_stream(encoder)
    }
}

// ====== DimSize helper ======

/// Stream-layer newtype for a single optional dimension (null = dynamic).
struct DimSize(Option<usize>);

impl<'en> en::IntoStream<'en> for DimSize {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        match self.0 {
            None => encoder.encode_none(),
            Some(n) => encoder.encode_u64(n as u64),
        }
    }
}

impl de::FromStream for DimSize {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: (),
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        struct DimVisitor;

        impl de::Visitor for DimVisitor {
            type Value = DimSize;

            fn expecting() -> &'static str {
                "a dimension size (unsigned integer or null)"
            }

            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(DimSize(None))
            }

            fn visit_u8<E: de::Error>(self, v: u8) -> Result<Self::Value, E> {
                Ok(DimSize(Some(v as usize)))
            }

            fn visit_u16<E: de::Error>(self, v: u16) -> Result<Self::Value, E> {
                Ok(DimSize(Some(v as usize)))
            }

            fn visit_u32<E: de::Error>(self, v: u32) -> Result<Self::Value, E> {
                Ok(DimSize(Some(v as usize)))
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(DimSize(Some(v as usize)))
            }

            fn visit_i32<E: de::Error>(self, v: i32) -> Result<Self::Value, E> {
                if v < 0 {
                    Err(de::Error::custom("dimension size cannot be negative"))
                } else {
                    Ok(DimSize(Some(v as usize)))
                }
            }

            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                if v < 0 {
                    Err(de::Error::custom("dimension size cannot be negative"))
                } else {
                    Ok(DimSize(Some(v as usize)))
                }
            }
        }

        decoder.decode_any(DimVisitor).await
    }
}

// ====== ShapeSeq helper ======

struct ShapeSeq(Vec<Option<usize>>);

impl<'en> en::IntoStream<'en> for ShapeSeq {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        let mut seq = encoder.encode_seq(Some(self.0.len()))?;
        for dim in self.0 {
            seq.encode_element(DimSize(dim))?;
        }
        seq.end()
    }
}

// ====== UsizeSeq helper ======

struct UsizeSeq(Vec<usize>);

impl<'en> en::IntoStream<'en> for UsizeSeq {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        let mut seq = encoder.encode_seq(Some(self.0.len()))?;
        for n in self.0 {
            seq.encode_element(n as u64)?;
        }
        seq.end()
    }
}

// ====== TensorTypeSpec ======

/// Element type and shape of a tensor value. Dynamic dimensions are represented as `None`.
///
/// Wire format: `{"dtype": "f32", "shape": [null, 3, 4]}`.
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

impl de::FromStream for TensorTypeSpec {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: (),
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

                let dtype = dtype.ok_or_else(|| de::Error::custom("missing dtype field"))?;
                let shape = shape.ok_or_else(|| de::Error::custom("missing shape field"))?;

                Ok(TensorTypeSpec { dtype, shape })
            }
        }

        decoder.decode_map(TypeSpecVisitor).await
    }
}

impl<'en> en::IntoStream<'en> for TensorTypeSpec {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        let mut map = encoder.encode_map(Some(2))?;
        map.encode_entry("dtype", self.dtype)?;
        map.encode_entry("shape", ShapeSeq(self.shape))?;
        map.end()
    }
}

impl<'en> en::ToStream<'en> for TensorTypeSpec {
    fn to_stream<E: en::Encoder<'en>>(&'en self, encoder: E) -> Result<E::Ok, E::Error> {
        self.clone().into_stream(encoder)
    }
}

// ====== TensorOp ======

/// Canonical tensor operator. The variant set is intentionally minimal for Phase 1.
///
/// Wire format: a single-entry map keyed by [`TensorOp::canonical_name`]:
/// `{"add": {"lhs": "v1", "rhs": "v2"}}`.
///
/// ## Boundary invariant (TASK-IR-009)
///
/// `tc-ir` must never define autodiff-specific types (`AutodiffRequest`,
/// `AutodiffResult`, `DerivativeMetadata`, `AutodiffError`). Those are
/// owned by the client package per FR-004.
#[derive(Clone, Debug, PartialEq)]
pub enum TensorOp {
    Add {
        lhs: ValueId,
        rhs: ValueId,
    },
    BroadcastReduce {
        input: ValueId,
        target_shape: Vec<usize>,
    },
    Matmul {
        lhs: ValueId,
        rhs: ValueId,
    },
    Transpose {
        input: ValueId,
        perm: Vec<usize>,
    },
}

impl TensorOp {
    /// Stable lowercase operator identity string used by the Python client mirror
    /// and tc-server handler registrations for cross-repo contract validation (FR-003 AC 6).
    pub fn canonical_name(&self) -> &'static str {
        match self {
            Self::Add { .. } => "add",
            Self::BroadcastReduce { .. } => "broadcast_reduce",
            Self::Matmul { .. } => "matmul",
            Self::Transpose { .. } => "transpose",
        }
    }
}

impl de::FromStream for TensorOp {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: (),
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
                    .ok_or_else(|| de::Error::custom("expected TensorOp variant key"))?;

                let op = match variant.as_str() {
                    "add" => {
                        let args = map.next_value::<BinaryOpArgs>(()).await?;
                        TensorOp::Add { lhs: args.lhs, rhs: args.rhs }
                    }
                    "broadcast_reduce" => {
                        let args = map.next_value::<ReduceOpArgs>(()).await?;
                        TensorOp::BroadcastReduce { input: args.input, target_shape: args.target_shape }
                    }
                    "matmul" => {
                        let args = map.next_value::<BinaryOpArgs>(()).await?;
                        TensorOp::Matmul { lhs: args.lhs, rhs: args.rhs }
                    }
                    "transpose" => {
                        let args = map.next_value::<TransposeOpArgs>(()).await?;
                        TensorOp::Transpose { input: args.input, perm: args.perm }
                    }
                    other => {
                        return Err(de::Error::custom(format!(
                            "unknown TensorOp variant: {other:?}"
                        )));
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

struct BinaryOpArgs {
    lhs: ValueId,
    rhs: ValueId,
}

impl de::FromStream for BinaryOpArgs {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: (),
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        struct V;

        impl de::Visitor for V {
            type Value = BinaryOpArgs;

            fn expecting() -> &'static str {
                "a binary op args map with lhs and rhs"
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

                let lhs = lhs.ok_or_else(|| de::Error::custom("missing lhs field"))?;
                let rhs = rhs.ok_or_else(|| de::Error::custom("missing rhs field"))?;
                Ok(BinaryOpArgs { lhs, rhs })
            }
        }

        decoder.decode_map(V).await
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

struct ReduceOpArgs {
    input: ValueId,
    target_shape: Vec<usize>,
}

impl de::FromStream for ReduceOpArgs {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: (),
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        struct V;

        impl de::Visitor for V {
            type Value = ReduceOpArgs;

            fn expecting() -> &'static str {
                "a broadcast_reduce args map with input and target_shape"
            }

            async fn visit_map<A: de::MapAccess>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut input = None;
                let mut target_shape = None;

                while let Some(key) = map.next_key::<String>(()).await? {
                    match key.as_str() {
                        "input" => input = Some(map.next_value::<ValueId>(()).await?),
                        "target_shape" => {
                            let dims = map.next_value::<Vec<DimSize>>(()).await?;
                            target_shape = Some(
                                dims.into_iter()
                                    .map(|d| {
                                        d.0.ok_or_else(|| {
                                            de::Error::custom(
                                                "target_shape dimensions must be concrete (non-null)",
                                            )
                                        })
                                    })
                                    .collect::<Result<Vec<_>, _>>()?,
                            );
                        }
                        _ => {
                            let _ = map.next_value::<de::IgnoredAny>(()).await?;
                        }
                    }
                }

                let input = input.ok_or_else(|| de::Error::custom("missing input field"))?;
                let target_shape =
                    target_shape.ok_or_else(|| de::Error::custom("missing target_shape field"))?;
                Ok(ReduceOpArgs { input, target_shape })
            }
        }

        decoder.decode_map(V).await
    }
}

impl<'en> en::IntoStream<'en> for ReduceOpArgs {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        let mut map = encoder.encode_map(Some(2))?;
        map.encode_entry("input", self.input)?;
        map.encode_entry("target_shape", UsizeSeq(self.target_shape))?;
        map.end()
    }
}

struct TransposeOpArgs {
    input: ValueId,
    perm: Vec<usize>,
}

impl de::FromStream for TransposeOpArgs {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: (),
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        struct V;

        impl de::Visitor for V {
            type Value = TransposeOpArgs;

            fn expecting() -> &'static str {
                "a transpose args map with input and perm"
            }

            async fn visit_map<A: de::MapAccess>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut input = None;
                let mut perm = None;

                while let Some(key) = map.next_key::<String>(()).await? {
                    match key.as_str() {
                        "input" => input = Some(map.next_value::<ValueId>(()).await?),
                        "perm" => {
                            let dims = map.next_value::<Vec<DimSize>>(()).await?;
                            perm = Some(
                                dims.into_iter()
                                    .map(|d| {
                                        d.0.ok_or_else(|| {
                                            de::Error::custom(
                                                "perm elements must be concrete (non-null)",
                                            )
                                        })
                                    })
                                    .collect::<Result<Vec<_>, _>>()?,
                            );
                        }
                        _ => {
                            let _ = map.next_value::<de::IgnoredAny>(()).await?;
                        }
                    }
                }

                let input = input.ok_or_else(|| de::Error::custom("missing input field"))?;
                let perm = perm.ok_or_else(|| de::Error::custom("missing perm field"))?;
                Ok(TransposeOpArgs { input, perm })
            }
        }

        decoder.decode_map(V).await
    }
}

impl<'en> en::IntoStream<'en> for TransposeOpArgs {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        let mut map = encoder.encode_map(Some(2))?;
        map.encode_entry("input", self.input)?;
        map.encode_entry("perm", UsizeSeq(self.perm))?;
        map.end()
    }
}

impl<'en> en::IntoStream<'en> for TensorOp {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        let mut map = encoder.encode_map(Some(1))?;

        match self {
            TensorOp::Add { lhs, rhs } => {
                map.encode_entry("add", BinaryOpArgs { lhs, rhs })?;
            }
            TensorOp::BroadcastReduce { input, target_shape } => {
                map.encode_entry("broadcast_reduce", ReduceOpArgs { input, target_shape })?;
            }
            TensorOp::Matmul { lhs, rhs } => {
                map.encode_entry("matmul", BinaryOpArgs { lhs, rhs })?;
            }
            TensorOp::Transpose { input, perm } => {
                map.encode_entry("transpose", TransposeOpArgs { input, perm })?;
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

// ====== TensorNode ======

/// A single computation step in a [`TensorGraph`]: associates a [`NodeId`] with an
/// operator, its input [`ValueId`]s (embedded in the op), and its output [`ValueId`] and type.
#[derive(Clone, Debug, PartialEq)]
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
        _context: (),
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        struct TensorNodeVisitor;

        impl de::Visitor for TensorNodeVisitor {
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

                let id = id.ok_or_else(|| de::Error::custom("missing id field"))?;
                let output = output.ok_or_else(|| de::Error::custom("missing output field"))?;
                let op = op.ok_or_else(|| de::Error::custom("missing op field"))?;
                let output_type =
                    output_type.ok_or_else(|| de::Error::custom("missing output_type field"))?;

                Ok(TensorNode {
                    id,
                    output,
                    op,
                    output_type,
                })
            }
        }

        decoder.decode_map(TensorNodeVisitor).await
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

// ====== InputPair helper ======

struct InputPair(ValueId, TensorTypeSpec);

impl de::FromStream for InputPair {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: (),
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        struct InputPairVisitor;

        impl de::Visitor for InputPairVisitor {
            type Value = InputPair;

            fn expecting() -> &'static str {
                "an input pair [value_id, type_spec]"
            }

            async fn visit_seq<A: de::SeqAccess>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                let vid = seq
                    .next_element::<ValueId>(())
                    .await?
                    .ok_or_else(|| de::Error::custom("expected value_id in input pair"))?;
                let ts = seq
                    .next_element::<TensorTypeSpec>(())
                    .await?
                    .ok_or_else(|| de::Error::custom("expected type_spec in input pair"))?;

                while seq.next_element::<de::IgnoredAny>(()).await?.is_some() {}

                Ok(InputPair(vid, ts))
            }
        }

        decoder.decode_seq(InputPairVisitor).await
    }
}

impl<'en> en::IntoStream<'en> for InputPair {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        let mut seq = encoder.encode_seq(Some(2))?;
        seq.encode_element(self.0)?;
        seq.encode_element(self.1)?;
        seq.end()
    }
}

// ====== TensorGraph ======

/// A complete typed tensor computation graph: named inputs with type specs, a topologically
/// ordered list of computation nodes, and named output values.
///
/// Wire format:
/// ```json
/// {
///   "inputs":  [["v0", {"dtype": "f32", "shape": [3, 4]}]],
///   "outputs": ["v2"],
///   "nodes":   [...]
/// }
/// ```
#[derive(Clone, Debug, PartialEq)]
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

impl de::FromStream for TensorGraph {
    type Context = ();

    async fn from_stream<D: de::Decoder>(
        _context: (),
        decoder: &mut D,
    ) -> Result<Self, D::Error> {
        struct TensorGraphVisitor;

        impl de::Visitor for TensorGraphVisitor {
            type Value = TensorGraph;

            fn expecting() -> &'static str {
                "a TensorGraph map"
            }

            async fn visit_map<A: de::MapAccess>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut inputs = None;
                let mut outputs = None;
                let mut nodes = None;

                while let Some(key) = map.next_key::<String>(()).await? {
                    match key.as_str() {
                        "inputs" => {
                            let pairs = map.next_value::<Vec<InputPair>>(()).await?;
                            inputs =
                                Some(pairs.into_iter().map(|p| (p.0, p.1)).collect::<Vec<_>>());
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

                let inputs = inputs.unwrap_or_default();
                let outputs = outputs.unwrap_or_default();
                let nodes = nodes.unwrap_or_default();

                Ok(TensorGraph {
                    inputs,
                    outputs,
                    nodes,
                })
            }
        }

        decoder.decode_map(TensorGraphVisitor).await
    }
}

impl<'en> en::IntoStream<'en> for TensorGraph {
    fn into_stream<E: en::Encoder<'en>>(self, encoder: E) -> Result<E::Ok, E::Error> {
        let mut map = encoder.encode_map(Some(3))?;

        let input_pairs: Vec<InputPair> = self
            .inputs
            .into_iter()
            .map(|(v, t)| InputPair(v, t))
            .collect();
        map.encode_entry("inputs", input_pairs)?;
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

// ====== Shape inference ======

/// Error type for shape inference failures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShapeError(pub String);

impl fmt::Display for ShapeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for ShapeError {}

fn shape_err(msg: impl Into<String>) -> ShapeError {
    ShapeError(msg.into())
}

/// Compute the broadcast result shape of two shapes, right-aligning them.
///
/// Returns an error if the shapes are incompatible (a non-1 dimension paired with a
/// different non-1 dimension).
pub fn broadcast_shapes(
    a: &[Option<usize>],
    b: &[Option<usize>],
) -> Result<Vec<Option<usize>>, ShapeError> {
    let rank = a.len().max(b.len());
    let mut result = Vec::with_capacity(rank);

    for i in 0..rank {
        let a_idx = (a.len() as isize) - (rank as isize) + (i as isize);
        let b_idx = (b.len() as isize) - (rank as isize) + (i as isize);

        let a_dim = if a_idx >= 0 { a[a_idx as usize] } else { Some(1) };
        let b_dim = if b_idx >= 0 { b[b_idx as usize] } else { Some(1) };

        let out = match (a_dim, b_dim) {
            (None, _) | (_, None) => None,
            (Some(1), Some(b)) => Some(b),
            (Some(a), Some(1)) => Some(a),
            (Some(a), Some(b)) if a == b => Some(a),
            (Some(a), Some(b)) => {
                return Err(shape_err(format!(
                    "incompatible broadcast dimensions: {a} vs {b} at axis {i}"
                )));
            }
        };

        result.push(out);
    }

    Ok(result)
}

/// Compute which axes of `input_shape` must be summed to reduce the tensor to
/// `target_shape`.
///
/// The `target_shape` must be broadcast-compatible with `input_shape` (i.e. every
/// concrete target dimension either matches the corresponding input dimension or is 1,
/// right-aligned; all leading input dimensions without a corresponding target dimension
/// are also reduction axes).
///
/// Returns the list of reduction axis indices in ascending order (indexing into
/// `input_shape`).
pub fn broadcast_reduce_axes(
    input_shape: &[usize],
    target_shape: &[usize],
) -> Result<Vec<usize>, ShapeError> {
    let in_rank = input_shape.len();
    let tgt_rank = target_shape.len();

    if tgt_rank > in_rank {
        return Err(shape_err(format!(
            "target_shape rank {tgt_rank} exceeds input_shape rank {in_rank}"
        )));
    }

    let mut axes = Vec::new();

    for i in 0..in_rank {
        let tgt_idx = (tgt_rank as isize) - (in_rank as isize) + (i as isize);

        if tgt_idx < 0 {
            axes.push(i);
        } else {
            let t = target_shape[tgt_idx as usize];
            let inp = input_shape[i];

            if t == inp {
            } else if t == 1 {
                axes.push(i);
            } else {
                return Err(shape_err(format!(
                    "target_shape[{tgt_idx}]={t} is incompatible with input_shape[{i}]={inp} \
                     (expected equal or 1)"
                )));
            }
        }
    }

    Ok(axes)
}

/// Infer the output shape of a batched matrix multiplication `A @ B`.
///
/// Both shapes must have rank ≥ 2. The inner dimensions (`A[-1]` and `B[-2]`) must
/// match when both are concrete. Batch dimensions (all but the last two) are
/// broadcast-pairwise.
///
/// Dynamic (`None`) dimensions are propagated without error.
pub fn matmul_shape(
    a: &[Option<usize>],
    b: &[Option<usize>],
) -> Result<Vec<Option<usize>>, ShapeError> {
    if a.len() < 2 {
        return Err(shape_err(format!(
            "matmul requires rank ≥ 2 for lhs, got rank {}",
            a.len()
        )));
    }
    if b.len() < 2 {
        return Err(shape_err(format!(
            "matmul requires rank ≥ 2 for rhs, got rank {}",
            b.len()
        )));
    }

    let a_inner = a[a.len() - 1];
    let b_inner = b[b.len() - 2];

    match (a_inner, b_inner) {
        (Some(a_k), Some(b_k)) if a_k != b_k => {
            return Err(shape_err(format!(
                "matmul inner dimension mismatch: lhs[-1]={a_k} != rhs[-2]={b_k}"
            )));
        }
        _ => {}
    }

    let a_batch = &a[..a.len() - 2];
    let b_batch = &b[..b.len() - 2];
    let mut batch = broadcast_shapes(a_batch, b_batch)?;

    batch.push(a[a.len() - 2]);
    batch.push(b[b.len() - 1]);

    Ok(batch)
}

/// Validate that `perm` is a valid permutation for a tensor of the given `rank`.
///
/// A valid permutation has exactly `rank` elements and each axis index in `[0, rank)`
/// appears exactly once.
pub fn validate_perm(perm: &[usize], rank: usize) -> Result<(), ShapeError> {
    if perm.len() != rank {
        return Err(shape_err(format!(
            "permutation length {} does not match tensor rank {}",
            perm.len(),
            rank
        )));
    }

    let mut seen = vec![false; rank];

    for &axis in perm {
        if axis >= rank {
            return Err(shape_err(format!(
                "permutation axis {axis} out of range for rank {rank}"
            )));
        }

        if seen[axis] {
            return Err(shape_err(format!(
                "axis {axis} appears more than once in permutation"
            )));
        }

        seen[axis] = true;
    }

    Ok(())
}
