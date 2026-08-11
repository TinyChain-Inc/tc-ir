#![forbid(unsafe_code)]

//! Core TinyChain IR traits, inspired by `tc-transact`'s `Handler` and `Route`
//! abstractions.
//!
//! These definitions intentionally mirror the behavior of the existing
//! `tc-transact` `Handler`/`Route` traits while staying agnostic to any
//! particular runtime. They should be expressive enough to back WASM sandboxes,
//! PyO3 bindings, or the existing Rust server stack without leaking lower-level
//! implementation details.

pub use hr_id::Id;
pub use tc_value::class::{Class, NativeClass};

mod txn;
pub use txn::*;

mod handler;
pub use handler::*;

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

mod dir;
pub use dir::*;

mod library;
pub use library::*;

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::str::FromStr;

    use number_general::Number;
    use pathlink::{Link, PathBuf, PathSegment};
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

    #[tokio::test(flavor = "multi_thread")]
    async fn library_schema_destream_roundtrip() {
        let schema = LibrarySchema::new(
            Link::from_str("/lib/service").expect("link"),
            "0.1.0",
            vec![
                Link::from_str("/lib/dependency").expect("dep"),
                Link::from_str("/lib/other").expect("dep"),
            ],
        );

        let encoded = destream_json::encode(schema.clone()).expect("encode schema");
        let decoded: LibrarySchema = destream_json::try_decode((), encoded)
            .await
            .expect("decode schema");

        assert_eq!(decoded, schema);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn txn_header_destream_roundtrip() {
        let claim = Claim::new(Link::from_str("/lib/service").unwrap(), umask::Mode::all());
        let header = TxnHeader::from_transaction(&FakeTxn::new(claim));

        let encoded = destream_json::encode(header.clone()).expect("encode header");
        let decoded: TxnHeader = destream_json::try_decode((), encoded)
            .await
            .expect("decode header");

        assert_eq!(decoded, header);
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

    fn segment(name: &str) -> PathSegment {
        PathSegment::from_str(name).expect("path segment")
    }

    #[test]
    fn native_routing_is_projection_free() {
        let routing = include_str!("handler.rs");
        for forbidden in ["destream", "serde", "hyper", "pyo3", "wasm"] {
            assert!(
                !routing.contains(forbidden),
                "native routing must not depend on {forbidden}"
            );
        }
    }

    #[test]
    fn native_route_state_is_explicit() {
        let handler = include_str!("handler.rs");
        assert!(!handler.contains("Route<State ="));

        let library = include_str!("library.rs");
        assert!(library.contains("Routes: Route<State>"));
        assert!(!library.contains("Routes: Route,"));
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
    async fn dir_routes_nested_handler() {
        let path = vec![segment("library"), segment("status")];
        let dir = Dir::from_routes(vec![(path.clone(), HelloHandler)]).expect("dir");

        let claim = Claim::new(Link::from_str("/lib").unwrap(), umask::Mode::all());
        let txn = FakeTxn::new(claim);

        let handler = dir.route(&path).expect("handler resolved");
        let out = handler
            .get(&txn, Scalar::from(Value::String("tinychain".to_string())))
            .await
            .expect("GET");
        assert_eq!(out.0, "hello tinychain");
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

    #[tokio::test(flavor = "multi_thread")]
    async fn macro_builds_routes() {
        let dir = tc_library_routes! {
            "/lib/status" => HelloHandler,
        }
        .expect("macro routes");

        let claim = Claim::new(Link::from_str("/lib").unwrap(), umask::Mode::all());
        let txn = FakeTxn::new(claim);
        let path = [segment("lib"), segment("status")];
        let handler = dir.route(&path).expect("handler");
        let out = handler
            .get(&txn, Scalar::from(Value::String("macro".to_string())))
            .await
            .expect("GET");
        assert_eq!(out.0, "hello macro");
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
    fn library_module_wraps_schema_and_routes() {
        let schema = LibrarySchema::new(Link::from_str("/lib/service").unwrap(), "1.0.0", vec![]);
        let routes = tc_library_routes! {
            "/lib/status" => HelloHandler,
        }
        .expect("routes");

        let lib: LibraryModule<FakeState, _> = LibraryModule::new(schema.clone(), routes);
        assert_eq!(lib.schema(), &schema);
        let path = [segment("lib"), segment("status")];
        assert!(lib.routes().route(&path).is_some());
    }

    #[test]
    fn map_require_optional() {
        let mut map: Map<u64> = Map::new();
        map.insert("answer".parse().expect("Id"), 42);

        assert_eq!(map.optional("missing").expect("optional"), None);
        assert_eq!(map.optional("answer").expect("optional"), Some(42));

        map.insert("answer".parse().expect("Id"), 42);
        assert_eq!(map.require("answer").expect("require"), 42);
        assert!(map.is_empty());

        let err = map.require("answer").unwrap_err();
        assert!(err.message().contains("missing answer parameter"));
    }
}
