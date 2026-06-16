use tc_ir::tensor::{
    broadcast_reduce_axes, broadcast_shapes, matmul_shape, validate_perm, NodeId, TensorDtype,
    TensorGraph, TensorNode, TensorOp, TensorTypeSpec, ValueId,
};

fn encode_decode<T>(value: T) -> T
where
    T: for<'en> destream::en::IntoStream<'en>
        + destream::de::FromStream<Context = ()>
        + Clone
        + std::fmt::Debug
        + PartialEq,
{
    let encoded = destream_json::encode(value.clone()).expect("encode");
    let decoded: T =
        futures::executor::block_on(destream_json::try_decode((), encoded)).expect("decode");
    decoded
}

// ====== NodeId / ValueId ======

#[test]
fn node_id_roundtrip() {
    let id = NodeId::new("node_0");
    let decoded = encode_decode(id.clone());
    assert_eq!(decoded, id);
}

#[test]
fn value_id_roundtrip() {
    let id = ValueId::new("v_output");
    let decoded = encode_decode(id.clone());
    assert_eq!(decoded, id);
}

// ====== TensorTypeSpec ======

#[test]
fn tensor_type_spec_static_shape_roundtrip() {
    let spec = TensorTypeSpec::new(TensorDtype::F32, vec![Some(3), Some(4)]);
    let decoded = encode_decode(spec.clone());
    assert_eq!(decoded, spec);
}

#[test]
fn tensor_type_spec_dynamic_shape_roundtrip() {
    let spec = TensorTypeSpec::new(TensorDtype::F64, vec![None, Some(128), None]);
    let decoded = encode_decode(spec.clone());
    assert_eq!(decoded, spec);
}

#[test]
fn tensor_type_spec_scalar_shape_roundtrip() {
    let spec = TensorTypeSpec::new(TensorDtype::F32, vec![]);
    let decoded = encode_decode(spec.clone());
    assert_eq!(decoded, spec);
}

// ====== TensorOp variants ======

#[test]
fn tensor_op_add_roundtrip() {
    let op = TensorOp::Add {
        lhs: ValueId::new("v0"),
        rhs: ValueId::new("v1"),
    };
    assert_eq!(op.canonical_name(), "add");
    let decoded = encode_decode(op.clone());
    assert_eq!(decoded, op);
}

#[test]
fn tensor_op_broadcast_reduce_roundtrip() {
    let op = TensorOp::BroadcastReduce {
        input: ValueId::new("grad"),
        target_shape: vec![3, 4],
    };
    assert_eq!(op.canonical_name(), "broadcast_reduce");
    let decoded = encode_decode(op.clone());
    assert_eq!(decoded, op);
}

#[test]
fn tensor_op_matmul_roundtrip() {
    let op = TensorOp::Matmul {
        lhs: ValueId::new("a"),
        rhs: ValueId::new("b"),
    };
    assert_eq!(op.canonical_name(), "matmul");
    let decoded = encode_decode(op.clone());
    assert_eq!(decoded, op);
}

#[test]
fn tensor_op_transpose_roundtrip() {
    let op = TensorOp::Transpose {
        input: ValueId::new("x"),
        perm: vec![1, 0, 2],
    };
    assert_eq!(op.canonical_name(), "transpose");
    let decoded = encode_decode(op.clone());
    assert_eq!(decoded, op);
}

#[test]
fn tensor_op_canonical_names_stable() {
    assert_eq!(
        TensorOp::Add {
            lhs: ValueId::new("a"),
            rhs: ValueId::new("b"),
        }
        .canonical_name(),
        "add"
    );
    assert_eq!(
        TensorOp::BroadcastReduce {
            input: ValueId::new("g"),
            target_shape: vec![1],
        }
        .canonical_name(),
        "broadcast_reduce"
    );
    assert_eq!(
        TensorOp::Matmul {
            lhs: ValueId::new("a"),
            rhs: ValueId::new("b"),
        }
        .canonical_name(),
        "matmul"
    );
    assert_eq!(
        TensorOp::Transpose {
            input: ValueId::new("x"),
            perm: vec![0, 1],
        }
        .canonical_name(),
        "transpose"
    );
}

// ====== TensorGraph ======

#[test]
fn tensor_graph_all_ops_roundtrip() {
    let graph = TensorGraph::new(
        vec![(
            ValueId::new("v0"),
            TensorTypeSpec::new(TensorDtype::F32, vec![Some(3), Some(4)]),
        )],
        vec![ValueId::new("v2")],
        vec![
            TensorNode::new(
                NodeId::new("n0"),
                ValueId::new("v1"),
                TensorOp::Add {
                    lhs: ValueId::new("v0"),
                    rhs: ValueId::new("v0"),
                },
                TensorTypeSpec::new(TensorDtype::F32, vec![Some(3), Some(4)]),
            ),
            TensorNode::new(
                NodeId::new("n1"),
                ValueId::new("v_r"),
                TensorOp::BroadcastReduce {
                    input: ValueId::new("v1"),
                    target_shape: vec![1, 4],
                },
                TensorTypeSpec::new(TensorDtype::F32, vec![Some(1), Some(4)]),
            ),
            TensorNode::new(
                NodeId::new("n2"),
                ValueId::new("v_mm"),
                TensorOp::Matmul {
                    lhs: ValueId::new("v1"),
                    rhs: ValueId::new("w"),
                },
                TensorTypeSpec::new(TensorDtype::F32, vec![Some(3), Some(5)]),
            ),
            TensorNode::new(
                NodeId::new("n3"),
                ValueId::new("v2"),
                TensorOp::Transpose {
                    input: ValueId::new("v_mm"),
                    perm: vec![1, 0],
                },
                TensorTypeSpec::new(TensorDtype::F32, vec![Some(5), Some(3)]),
            ),
        ],
    );

    let decoded = encode_decode(graph.clone());
    assert_eq!(decoded, graph);
}

// ====== broadcast_shapes ======

#[test]
fn broadcast_same_shape() {
    let a = vec![Some(3), Some(4)];
    let b = vec![Some(3), Some(4)];
    assert_eq!(broadcast_shapes(&a, &b).unwrap(), vec![Some(3), Some(4)]);
}

#[test]
fn broadcast_right_aligned() {
    let a = vec![Some(3), Some(1), Some(4)];
    let b = vec![Some(2), Some(4)];
    assert_eq!(
        broadcast_shapes(&a, &b).unwrap(),
        vec![Some(3), Some(2), Some(4)]
    );
}

#[test]
fn broadcast_missing_leading_dims() {
    let a = vec![Some(4)];
    let b = vec![Some(3), Some(4)];
    assert_eq!(
        broadcast_shapes(&a, &b).unwrap(),
        vec![Some(3), Some(4)]
    );
}

#[test]
fn broadcast_incompatible_dims() {
    let a = vec![Some(3), Some(4)];
    let b = vec![Some(3), Some(5)];
    assert!(broadcast_shapes(&a, &b).is_err());
}

#[test]
fn broadcast_dynamic_dim_propagates() {
    let a = vec![None, Some(4)];
    let b = vec![Some(3), Some(4)];
    let result = broadcast_shapes(&a, &b).unwrap();
    assert_eq!(result, vec![None, Some(4)]);
}

#[test]
fn broadcast_scalar_with_vector() {
    let a = vec![];
    let b = vec![Some(5)];
    assert_eq!(broadcast_shapes(&a, &b).unwrap(), vec![Some(5)]);
}

// ====== broadcast_reduce_axes ======

#[test]
fn reduce_axes_leading_dims() {
    let input = vec![3usize, 4];
    let target = vec![1usize, 4];
    assert_eq!(broadcast_reduce_axes(&input, &target).unwrap(), vec![0]);
}

#[test]
fn reduce_axes_inner_dim_one() {
    let input = vec![3usize, 4];
    let target = vec![3usize, 1];
    assert_eq!(broadcast_reduce_axes(&input, &target).unwrap(), vec![1]);
}

#[test]
fn reduce_axes_missing_leading() {
    let input = vec![3usize, 4];
    let target = vec![4usize];
    assert_eq!(broadcast_reduce_axes(&input, &target).unwrap(), vec![0]);
}

#[test]
fn reduce_axes_no_reduction_needed() {
    let input = vec![3usize, 4];
    let target = vec![3usize, 4];
    assert_eq!(broadcast_reduce_axes(&input, &target).unwrap(), vec![]);
}

#[test]
fn reduce_axes_incompatible_target() {
    let input = vec![3usize, 4];
    let target = vec![3usize, 5];
    assert!(broadcast_reduce_axes(&input, &target).is_err());
}

#[test]
fn reduce_axes_target_rank_exceeds_input_rank() {
    let input = vec![4usize];
    let target = vec![3usize, 4];
    assert!(broadcast_reduce_axes(&input, &target).is_err());
}

// ====== matmul_shape ======

#[test]
fn matmul_basic_2d() {
    let a = vec![Some(3), Some(4)];
    let b = vec![Some(4), Some(5)];
    assert_eq!(matmul_shape(&a, &b).unwrap(), vec![Some(3), Some(5)]);
}

#[test]
fn matmul_batched() {
    let a = vec![Some(2), Some(3), Some(4)];
    let b = vec![Some(2), Some(4), Some(5)];
    assert_eq!(
        matmul_shape(&a, &b).unwrap(),
        vec![Some(2), Some(3), Some(5)]
    );
}

#[test]
fn matmul_batch_broadcast() {
    let a = vec![Some(1), Some(3), Some(4)];
    let b = vec![Some(7), Some(4), Some(5)];
    assert_eq!(
        matmul_shape(&a, &b).unwrap(),
        vec![Some(7), Some(3), Some(5)]
    );
}

#[test]
fn matmul_inner_dim_mismatch() {
    let a = vec![Some(3), Some(4)];
    let b = vec![Some(5), Some(6)];
    assert!(matmul_shape(&a, &b).is_err());
}

#[test]
fn matmul_rank_too_low_lhs() {
    let a = vec![Some(4)];
    let b = vec![Some(4), Some(5)];
    assert!(matmul_shape(&a, &b).is_err());
}

#[test]
fn matmul_dynamic_inner() {
    let a = vec![Some(3), None];
    let b = vec![None, Some(5)];
    let result = matmul_shape(&a, &b).unwrap();
    assert_eq!(result, vec![Some(3), Some(5)]);
}

// ====== validate_perm ======

#[test]
fn perm_valid_identity() {
    assert!(validate_perm(&[0, 1, 2], 3).is_ok());
}

#[test]
fn perm_valid_swap() {
    assert!(validate_perm(&[1, 0], 2).is_ok());
}

#[test]
fn perm_valid_full_permutation() {
    assert!(validate_perm(&[2, 0, 1], 3).is_ok());
}

#[test]
fn perm_wrong_length() {
    assert!(validate_perm(&[0, 1], 3).is_err());
}

#[test]
fn perm_axis_out_of_range() {
    assert!(validate_perm(&[0, 3, 2], 3).is_err());
}

#[test]
fn perm_duplicate_axis() {
    assert!(validate_perm(&[0, 0, 1], 3).is_err());
}

#[test]
fn tensor_node_standalone_roundtrip() {
    let node = TensorNode::new(
        NodeId::new("n0"),
        ValueId::new("v1"),
        TensorOp::Matmul {
            lhs: ValueId::new("v1"),
            rhs: ValueId::new("v1"),
        },
        TensorTypeSpec::new(TensorDtype::F32, vec![Some(2), Some(3)]),
    );
    let decoded = encode_decode(node.clone());
    assert_eq!(decoded, node);
}

#[test]
fn tensor_graph_empty_roundtrip() {
    let graph = TensorGraph::new(vec![], vec![], vec![]);
    let decoded = encode_decode(graph.clone());
    assert_eq!(decoded, graph);
}

#[test]
fn tensor_op_unknown_variant_decode_error() {
    struct UnknownOpJson;
    impl<'en> destream::en::IntoStream<'en> for UnknownOpJson {
        fn into_stream<E: destream::en::Encoder<'en>>(
            self,
            encoder: E,
        ) -> Result<E::Ok, E::Error> {
            use destream::en::EncodeMap;
            let mut map = encoder.encode_map(Some(1))?;
            map.encode_entry("unknown_op", "")?;
            map.end()
        }
    }
    let raw = destream_json::encode(UnknownOpJson).expect("encode unknown op json");
    let result: Result<TensorOp, _> =
        futures::executor::block_on(destream_json::try_decode((), raw));
    assert!(result.is_err());
}
