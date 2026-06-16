use tc_ir::autodiff::{AutodiffError, AutodiffRequest, AutodiffResult, DerivativeMetadata};
use tc_ir::tensor::{
    broadcast_shapes, matmul_output_shape, validate_permutation, NodeId, TensorDtype, TensorGraph,
    TensorNode, TensorOp, TensorTypeSpec, ValueId,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn f32_spec(shape: Vec<Option<usize>>) -> TensorTypeSpec {
    TensorTypeSpec::new(TensorDtype::F32, shape)
}

// ---------------------------------------------------------------------------
// Step 1-2: NodeId, ValueId, TensorDtype, TensorTypeSpec round-trips
// ---------------------------------------------------------------------------

#[test]
fn node_id_roundtrip() {
    let id = NodeId::new("n0");
    let enc = destream_json::encode(id.clone()).expect("encode NodeId");
    let dec: NodeId =
        futures::executor::block_on(destream_json::try_decode((), enc)).expect("decode NodeId");
    assert_eq!(dec, id);
}

#[test]
fn value_id_roundtrip() {
    let id = ValueId::new("v42");
    let enc = destream_json::encode(id.clone()).expect("encode ValueId");
    let dec: ValueId =
        futures::executor::block_on(destream_json::try_decode((), enc)).expect("decode ValueId");
    assert_eq!(dec, id);
}

#[test]
fn tensor_dtype_f32_roundtrip() {
    let enc = destream_json::encode(TensorDtype::F32).expect("encode");
    let dec: TensorDtype =
        futures::executor::block_on(destream_json::try_decode((), enc)).expect("decode");
    assert_eq!(dec, TensorDtype::F32);
}

#[test]
fn tensor_dtype_f64_roundtrip() {
    let enc = destream_json::encode(TensorDtype::F64).expect("encode");
    let dec: TensorDtype =
        futures::executor::block_on(destream_json::try_decode((), enc)).expect("decode");
    assert_eq!(dec, TensorDtype::F64);
}

#[test]
fn tensor_type_spec_static_shape_roundtrip() {
    let spec = f32_spec(vec![Some(2), Some(3)]);
    let enc = destream_json::encode(spec.clone()).expect("encode");
    let dec: TensorTypeSpec =
        futures::executor::block_on(destream_json::try_decode((), enc)).expect("decode");
    assert_eq!(dec, spec);
}

#[test]
fn tensor_type_spec_dynamic_dim_roundtrip() {
    let spec = f32_spec(vec![Some(2), None, Some(4)]);
    let enc = destream_json::encode(spec.clone()).expect("encode");
    let dec: TensorTypeSpec =
        futures::executor::block_on(destream_json::try_decode((), enc)).expect("decode");
    assert_eq!(dec, spec);
}

// ---------------------------------------------------------------------------
// Step 3: TensorOp round-trips
// ---------------------------------------------------------------------------

#[test]
fn tensor_op_add_roundtrip() {
    let op = TensorOp::Add {
        lhs: ValueId::new("v0"),
        rhs: ValueId::new("v1"),
    };
    let enc = destream_json::encode(op.clone()).expect("encode");
    let dec: TensorOp =
        futures::executor::block_on(destream_json::try_decode((), enc)).expect("decode");
    assert_eq!(dec, op);
}

#[test]
fn tensor_op_matmul_roundtrip() {
    let op = TensorOp::Matmul {
        lhs: ValueId::new("v0"),
        rhs: ValueId::new("v1"),
    };
    let enc = destream_json::encode(op.clone()).expect("encode");
    let dec: TensorOp =
        futures::executor::block_on(destream_json::try_decode((), enc)).expect("decode");
    assert_eq!(dec, op);
}

#[test]
fn tensor_op_transpose_roundtrip() {
    let op = TensorOp::Transpose {
        input: ValueId::new("v0"),
        perm: vec![0, 2, 1],
    };
    let enc = destream_json::encode(op.clone()).expect("encode");
    let dec: TensorOp =
        futures::executor::block_on(destream_json::try_decode((), enc)).expect("decode");
    assert_eq!(dec, op);
}

// ---------------------------------------------------------------------------
// Step 4: TensorGraph round-trip
// ---------------------------------------------------------------------------

#[test]
fn tensor_graph_roundtrip() {
    let v0 = ValueId::new("v0");
    let v1 = ValueId::new("v1");
    let v2 = ValueId::new("v2");

    let op = TensorOp::Add {
        lhs: v0.clone(),
        rhs: v1.clone(),
    };
    let spec = f32_spec(vec![Some(2), Some(3)]);
    let node = TensorNode::new(NodeId::new("n0"), v2.clone(), op, spec.clone());

    let graph = TensorGraph::new(
        vec![(v0, spec.clone()), (v1, spec.clone())],
        vec![v2],
        vec![node],
    );

    let enc = destream_json::encode(graph.clone()).expect("encode TensorGraph");
    let dec: TensorGraph =
        futures::executor::block_on(destream_json::try_decode((), enc)).expect("decode TensorGraph");
    assert_eq!(dec, graph);
}

// ---------------------------------------------------------------------------
// Step 5: Broadcasting shape inference
// ---------------------------------------------------------------------------

#[test]
fn broadcast_same_shape() {
    let a = vec![Some(2), Some(3)];
    let b = vec![Some(2), Some(3)];
    assert_eq!(broadcast_shapes(&a, &b).unwrap(), vec![Some(2), Some(3)]);
}

#[test]
fn broadcast_right_aligned() {
    let a = vec![Some(4), Some(1), Some(3)];
    let b = vec![Some(2), Some(3)];
    assert_eq!(
        broadcast_shapes(&a, &b).unwrap(),
        vec![Some(4), Some(2), Some(3)]
    );
}

#[test]
fn broadcast_leading_ones() {
    let a = vec![Some(1), Some(1), Some(5)];
    let b = vec![Some(3), Some(5)];
    assert_eq!(
        broadcast_shapes(&a, &b).unwrap(),
        vec![Some(1), Some(3), Some(5)]
    );
}

#[test]
fn broadcast_dynamic_dim_propagates() {
    let a = vec![None, Some(3)];
    let b = vec![Some(2), Some(3)];
    let result = broadcast_shapes(&a, &b).unwrap();
    assert_eq!(result, vec![None, Some(3)]);
}

#[test]
fn broadcast_incompatible_dims_errors() {
    let a = vec![Some(2), Some(3)];
    let b = vec![Some(2), Some(4)];
    assert!(broadcast_shapes(&a, &b).is_err());
}

// ---------------------------------------------------------------------------
// Step 6: Batched matmul shape inference
// ---------------------------------------------------------------------------

#[test]
fn matmul_2d_basic() {
    let a = vec![Some(3), Some(4)];
    let b = vec![Some(4), Some(5)];
    assert_eq!(matmul_output_shape(&a, &b).unwrap(), vec![Some(3), Some(5)]);
}

#[test]
fn matmul_3d_batched() {
    let a = vec![Some(2), Some(3), Some(4)];
    let b = vec![Some(2), Some(4), Some(5)];
    assert_eq!(
        matmul_output_shape(&a, &b).unwrap(),
        vec![Some(2), Some(3), Some(5)]
    );
}

#[test]
fn matmul_batch_broadcast() {
    let a = vec![Some(1), Some(3), Some(4)];
    let b = vec![Some(2), Some(4), Some(5)];
    assert_eq!(
        matmul_output_shape(&a, &b).unwrap(),
        vec![Some(2), Some(3), Some(5)]
    );
}

#[test]
fn matmul_dynamic_inner_allowed() {
    let a = vec![Some(3), None];
    let b = vec![None, Some(5)];
    assert_eq!(
        matmul_output_shape(&a, &b).unwrap(),
        vec![Some(3), Some(5)]
    );
}

#[test]
fn matmul_rank_too_low_errors() {
    let a = vec![Some(3)];
    let b = vec![Some(3), Some(5)];
    assert!(matmul_output_shape(&a, &b).is_err());
}

#[test]
fn matmul_inner_mismatch_errors() {
    let a = vec![Some(3), Some(4)];
    let b = vec![Some(5), Some(6)];
    assert!(matmul_output_shape(&a, &b).is_err());
}

// ---------------------------------------------------------------------------
// Step 7: Permutation validation
// ---------------------------------------------------------------------------

#[test]
fn permutation_identity_valid() {
    assert!(validate_permutation(&[0, 1, 2], 3).is_ok());
}

#[test]
fn permutation_valid_transposed() {
    assert!(validate_permutation(&[0, 2, 1], 3).is_ok());
}

#[test]
fn permutation_wrong_length_errors() {
    assert!(validate_permutation(&[0, 1], 3).is_err());
}

#[test]
fn permutation_duplicate_axis_errors() {
    assert!(validate_permutation(&[0, 0, 2], 3).is_err());
}

#[test]
fn permutation_out_of_range_errors() {
    assert!(validate_permutation(&[0, 1, 5], 3).is_err());
}

// ---------------------------------------------------------------------------
// Step 8-11: AutodiffRequest, AutodiffResult, DerivativeMetadata round-trips
// ---------------------------------------------------------------------------

fn make_graph() -> TensorGraph {
    let v0 = ValueId::new("v0");
    let v1 = ValueId::new("v1");
    let v2 = ValueId::new("v2");
    let spec = f32_spec(vec![Some(2), Some(2)]);
    let op = TensorOp::Add {
        lhs: v0.clone(),
        rhs: v1.clone(),
    };
    let node = TensorNode::new(NodeId::new("n0"), v2.clone(), op, spec.clone());
    TensorGraph::new(
        vec![(v0, spec.clone()), (v1, spec.clone())],
        vec![v2],
        vec![node],
    )
}

#[test]
fn autodiff_request_roundtrip() {
    let req = AutodiffRequest::new(
        make_graph(),
        ValueId::new("v2"),
        vec![ValueId::new("v0"), ValueId::new("v1")],
        ValueId::new("seed"),
        "1.0.0",
        "1.0.0",
    );

    let enc = destream_json::encode(req.clone()).expect("encode AutodiffRequest");
    let dec: AutodiffRequest =
        futures::executor::block_on(destream_json::try_decode((), enc))
            .expect("decode AutodiffRequest");
    assert_eq!(dec, req);
}

#[test]
fn autodiff_result_roundtrip() {
    let result = AutodiffResult::new(vec![
        (ValueId::new("grad_v0"), f32_spec(vec![Some(2), Some(2)])),
        (ValueId::new("grad_v1"), f32_spec(vec![Some(2), Some(2)])),
    ]);

    let enc = destream_json::encode(result.clone()).expect("encode AutodiffResult");
    let dec: AutodiffResult =
        futures::executor::block_on(destream_json::try_decode((), enc))
            .expect("decode AutodiffResult");
    assert_eq!(dec, result);
}

#[test]
fn derivative_metadata_roundtrip() {
    let meta = DerivativeMetadata::new(
        "/lib/mylib/1.0.0",
        TensorOp::Add {
            lhs: ValueId::new("x"),
            rhs: ValueId::new("y"),
        },
        "1.0.0",
        vec![
            f32_spec(vec![Some(2), Some(2)]),
            f32_spec(vec![Some(2), Some(2)]),
        ],
        f32_spec(vec![Some(2), Some(2)]),
    );

    let enc = destream_json::encode(meta.clone()).expect("encode DerivativeMetadata");
    let dec: DerivativeMetadata =
        futures::executor::block_on(destream_json::try_decode((), enc))
            .expect("decode DerivativeMetadata");
    assert_eq!(dec, meta);
}

#[test]
fn autodiff_error_all_variants_roundtrip() {
    let errors = [
        AutodiffError::UnsupportedOperator,
        AutodiffError::MissingDerivativeIr,
        AutodiffError::DtypeNotDifferentiable,
        AutodiffError::ShapeMismatch,
        AutodiffError::WrtNotInGraph,
        AutodiffError::OutputNotInGraph,
        AutodiffError::SeedTypeMismatch,
        AutodiffError::CycleInGraph,
        AutodiffError::InvalidPermutation,
        AutodiffError::InvalidBroadcast,
        AutodiffError::RankTooLow,
        AutodiffError::InnerDimMismatch,
        AutodiffError::EmptyWrt,
        AutodiffError::ContractVersionMismatch,
    ];
    assert_eq!(errors.len(), 14, "must have exactly 14 error categories");

    for err in errors {
        let enc = destream_json::encode(err.clone()).expect("encode AutodiffError");
        let dec: AutodiffError =
            futures::executor::block_on(destream_json::try_decode((), enc))
                .expect("decode AutodiffError");
        assert_eq!(dec, err);
    }
}
