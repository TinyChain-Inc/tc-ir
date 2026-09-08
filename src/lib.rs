#![forbid(unsafe_code)]

//! TinyChain's scalar and operation algebra, plus its shared transaction,
//! routing, and view contracts.

pub use hr_id::Id;

mod txn;
pub use txn::*;

mod public;
pub use async_trait::async_trait;
pub use public::*;

mod view;
pub use view::*;

mod map;
pub use map::Map;

mod scalar;
pub use scalar::*;

mod op;
pub use op::*;

mod tcref;
pub use tcref::*;

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::str::FromStr;

    use number_general::Number;
    use pathlink::{Link, PathBuf};
    use tc_error::TCResult;
    use tc_value::Value;

    use super::*;

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

    #[derive(Clone)]
    struct HelloHandler;

    #[derive(Clone)]
    struct FakeState(String);

    impl StateInstance for FakeState {
        type Transaction = FakeTxn;
    }

    #[async_trait::async_trait]
    impl Handler<FakeState> for HelloHandler {
        async fn get(&self, _txn: &FakeTxn, request: Scalar) -> TCResult<FakeState> {
            let Scalar::Value(Value::String(request)) = request else {
                return Err(tc_error::TCError::bad_request("expected a string"));
            };
            Ok(FakeState(format!("hello {request}")))
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn handler_invocation() {
        let handler = HelloHandler;
        let claim = Claim::new(Link::from_str("/hello").unwrap(), umask::Mode::all());
        let txn = FakeTxn::new(claim);

        let out = handler
            .get(&txn, Scalar::from(Value::String("world".into())))
            .await
            .unwrap();
        assert_eq!(out.0, "hello world");
    }

    #[test]
    fn txn_id_round_trips_with_trace() {
        let txn_id = TxnId::from_parts(NetworkTime::from_nanos(7), 1).with_trace([3; 32]);
        let parsed = TxnId::from_str(&txn_id.to_string()).expect("parse txn id");

        assert_eq!(parsed, txn_id);
    }

    #[test]
    fn txn_id_rejects_partial_wire_id_without_trace() {
        assert!(TxnId::from_str("7-1").is_err());
    }

    #[test]
    fn native_routing_is_projection_free() {
        let routing = include_str!("public.rs");
        for forbidden in ["destream", "serde", "hyper", "pyo3", "wasm"] {
            assert!(
                !routing.contains(forbidden),
                "native routing must not depend on {forbidden}"
            );
        }
    }

    #[test]
    fn native_route_state_is_explicit() {
        let handler = include_str!("public.rs");
        assert!(!handler.contains("Route<State ="));
        assert!(!handler.contains("type Handler"));
        assert!(handler.contains("Box<dyn Handler<State>"));
    }

    #[test]
    fn scalar_async_hash_is_deterministic() {
        let scalar = Scalar::Map(Map::from_iter([
            ("a".parse().unwrap(), Scalar::from(7_u64)),
            ("b".parse().unwrap(), Scalar::from(Value::from("tinychain"))),
        ]));
        let digest = async_hash::Hash::<async_hash::Sha256>::hash(&scalar);
        let digest = digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(
            digest,
            "d0399510942f294e3d0df854af9cd024758ef600f0fb5b84ce63b6367866b918"
        );
    }

    #[test]
    fn conditional_refs_have_one_wire_form() {
        let legacy_symbol = ["TCREF", "_IF"].concat();
        let legacy_path = ["/state/scalar/ref", "/if"].concat();
        for source in [include_str!("scalar.rs"), include_str!("tcref.rs")] {
            assert!(!source.contains(&legacy_symbol));
            assert!(!source.contains(&legacy_path));
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn scalar_map_roundtrip() {
        let mut inner = Map::new();
        inner.insert(
            "signed".parse().expect("Id"),
            Scalar::from(Value::Number(Number::Bool(true.into()))),
        );
        inner.insert("bits".parse().expect("Id"), Scalar::from(16_u64));

        let mut outer = Map::new();
        outer.insert(
            "dtype".parse().expect("Id"),
            Scalar::from(Value::from("f32")),
        );
        outer.insert("encoding".parse().expect("Id"), Scalar::Map(inner));

        let scalar = Scalar::Map(outer);

        let encoded = destream_json::encode(scalar.clone()).expect("encode scalar map");
        let decoded: Scalar = destream_json::try_decode((), encoded)
            .await
            .expect("decode scalar map");

        assert_eq!(decoded, scalar);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn scalar_maps_reject_duplicate_input_names() {
        use bytes::Bytes;
        use futures::{future, stream};

        let source = stream::once(future::ready(Ok::<_, std::convert::Infallible>(
            Bytes::from_static(br#"{"input":1,"input":2}"#),
        )));
        let decoded: Result<Scalar, _> = destream_json::try_decode((), source).await;
        let error = decoded.expect_err("reject duplicate input name");
        assert!(error.to_string().contains("duplicate map key input"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn scalar_tuple_roundtrip() {
        let scalar = Scalar::Tuple(vec![Scalar::from(7_u64), Scalar::from(Value::from("x"))]);

        let encoded = destream_json::encode(scalar.clone()).expect("encode scalar tuple");
        let decoded: Scalar = destream_json::try_decode((), encoded)
            .await
            .expect("decode scalar tuple");

        assert_eq!(decoded, scalar);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn scalar_opref_decodes_as_ref() {
        let link = Link::from_str("/lib/acme/foo/1.0.0").expect("link");
        let op = OpRef::Get((Subject::Link(link), Scalar::default()));
        let scalar = Scalar::from(TCRef::Op(op));

        let encoded = destream_json::encode(scalar.clone()).expect("encode scalar ref");
        let decoded: Scalar = destream_json::try_decode((), encoded)
            .await
            .expect("decode scalar ref");

        assert_eq!(decoded, scalar);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn scalar_typed_opref_get_key_decodes_as_ref() {
        let subject = Subject::Link(Link::from_str("/lib/acme/foo/1.0.0").expect("link"));
        let key = Scalar::from(Value::from("k"));
        let mut encoded_map = BTreeMap::new();
        encoded_map.insert(
            PathBuf::from(OPREF_GET).to_string(),
            (subject.clone(), key.clone()),
        );

        let encoded = destream_json::encode(encoded_map).expect("encode typed opref get");
        let decoded: Scalar = destream_json::try_decode((), encoded)
            .await
            .expect("decode typed opref get as scalar");

        assert_eq!(decoded, Scalar::from(TCRef::Op(OpRef::Get((subject, key)))));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn opdef_roundtrip() {
        let form = vec![
            ("x".parse().expect("Id"), Scalar::from(7_u64)),
            ("y".parse().expect("Id"), Scalar::from(Value::from("z"))),
        ];
        let op = OpDef::Post(form);

        let encoded = destream_json::encode(op.clone()).expect("encode opdef");
        let decoded: OpDef = destream_json::try_decode((), encoded)
            .await
            .expect("decode opdef");

        assert_eq!(decoded, op);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tcref_id_roundtrip() {
        let tcref = TCRef::Id("$foo".parse().expect("IdRef"));
        let encoded = destream_json::encode(tcref.clone()).expect("encode tcref id");
        let decoded: TCRef = destream_json::try_decode((), encoded)
            .await
            .expect("decode tcref id");
        assert_eq!(decoded, tcref);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tcref_while_roundtrip() {
        let cond = Scalar::from(1_u64);
        let closure = Scalar::from(Value::from("step"));
        let state = Scalar::from(7_u64);
        let tcref = TCRef::While(Box::new(While::new(cond, closure, state)));
        let encoded = destream_json::encode(tcref.clone()).expect("encode tcref while");
        let decoded: TCRef = destream_json::try_decode((), encoded)
            .await
            .expect("decode tcref while");
        assert_eq!(decoded, tcref);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tcref_cond_roundtrip() {
        let cond = TCRef::Id("$flag".parse().expect("IdRef"));
        let then = Scalar::Op(OpDef::Post(vec![(
            "result".parse().expect("Id"),
            Scalar::from(1_u64),
        )]));
        let or_else = Scalar::Op(OpDef::Post(vec![(
            "result".parse().expect("Id"),
            Scalar::from(0_u64),
        )]));
        let tcref = TCRef::Cond(Box::new(Cond::new(cond, then, or_else)));

        let encoded = destream_json::encode(tcref.clone()).expect("encode tcref cond");
        let decoded: TCRef = destream_json::try_decode((), encoded)
            .await
            .expect("decode tcref cond");

        assert_eq!(decoded, tcref);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tcref_after_roundtrip() {
        let when = Scalar::from(TCRef::Id("$write".parse().expect("IdRef")));
        let then = Scalar::from(TCRef::Id("$read".parse().expect("IdRef")));
        let tcref = TCRef::After(Box::new(After::new(when, then)));

        let encoded = destream_json::encode(tcref.clone()).expect("encode tcref after");
        let decoded: TCRef = destream_json::try_decode((), encoded)
            .await
            .expect("decode tcref after");

        assert_eq!(decoded, tcref);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tcref_for_each_roundtrip() {
        let items = Scalar::Tuple(vec![Scalar::from(1_u64), Scalar::from(2_u64)]);
        let op = Scalar::Op(OpDef::Post(vec![(
            "result".parse().expect("Id"),
            Scalar::from(TCRef::Id("$item".parse().expect("IdRef"))),
        )]));
        let item_name = "item".parse().expect("Id");
        let tcref = TCRef::ForEach(Box::new(ForEach::new(items, op, item_name)));

        let encoded = destream_json::encode(tcref.clone()).expect("encode tcref for_each");
        let decoded: TCRef = destream_json::try_decode((), encoded)
            .await
            .expect("decode tcref for_each");

        assert_eq!(decoded, tcref);
    }

    #[test]
    fn map_require() {
        let mut map: Map<u64> = Map::new();
        map.insert("answer".parse().expect("Id"), 42);

        assert_eq!(map.require("answer").expect("require"), 42);
        assert!(map.is_empty());

        let err = map.require("answer").unwrap_err();
        assert!(err.message().contains("missing answer parameter"));
    }
}
