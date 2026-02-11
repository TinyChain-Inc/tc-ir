//! Core TinyChain IR traits, inspired by `tc-transact`'s `Handler` and `Route` abstractions.
//!
//! These definitions intentionally mirror the behavior of the existing `tc-transact`
//! `Handler`/`Route` traits while staying agnostic to any particular runtime. They should
//! be expressive enough to back WASM sandboxes, PyO3 bindings, or the existing Rust
//! server stack without leaking lower-level implementation details.

pub use hr_id::Id;
pub use tc_value::class::{Class, NativeClass};

mod txn;
pub use txn::*;

mod handler;
pub use handler::*;

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
    use super::*;

    use std::{future::Future, pin::Pin, str::FromStr};

    use number_general::Number;
    use pathlink::{Link, PathSegment};
    use tc_error::TCResult;
    use tc_value::Value;

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

    struct HelloHandler;

    impl HandleGet<FakeTxn> for HelloHandler {
        type Request = String;
        type RequestContext = ();
        type Response = String;
        type Error = ();
        type Fut<'a> =
            Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send + 'a>>;

        fn get<'a>(&'a self, _txn: &'a FakeTxn, request: Self::Request) -> TCResult<Self::Fut<'a>> {
            Ok(Box::pin(async move { Ok(format!("hello {request}")) }))
        }
    }

    #[test]
    fn handler_invocation() {
        let handler = HelloHandler;
        let claim = Claim::new(Link::from_str("/hello").unwrap(), umask::Mode::all());
        let txn = FakeTxn::new(claim);

        let fut = handler.get(&txn, "world".into()).expect("GET supported");
        let out = futures::executor::block_on(fut).unwrap();
        assert_eq!(out, "hello world");
    }

    #[test]
    fn library_schema_destream_roundtrip() {
        let schema = LibrarySchema::new(
            Link::from_str("/lib/service").expect("link"),
            "0.1.0",
            vec![
                Link::from_str("/lib/dependency").expect("dep"),
                Link::from_str("/lib/other").expect("dep"),
            ],
        );

        let encoded = destream_json::encode(schema.clone()).expect("encode schema");
        let decoded: LibrarySchema =
            futures::executor::block_on(destream_json::try_decode((), encoded))
                .expect("decode schema");

        assert_eq!(decoded, schema);
    }

    #[test]
    fn txn_header_destream_roundtrip() {
        let claim = Claim::new(Link::from_str("/lib/service").unwrap(), umask::Mode::all());
        let header = TxnHeader::new(
            TxnId::from_parts(NetworkTime::from_nanos(7), 1),
            NetworkTime::from_nanos(7),
            claim,
        );

        let encoded = destream_json::encode(header.clone()).expect("encode header");
        let decoded: TxnHeader =
            futures::executor::block_on(destream_json::try_decode((), encoded))
                .expect("decode header");

        assert_eq!(decoded, header);
    }

    fn segment(name: &str) -> PathSegment {
        PathSegment::from_str(name).expect("path segment")
    }

    #[test]
    fn dir_routes_nested_handler() {
        let path = vec![segment("library"), segment("status")];
        let dir = Dir::from_routes(vec![(path.clone(), HelloHandler)]).expect("dir");

        let claim = Claim::new(Link::from_str("/lib").unwrap(), umask::Mode::all());
        let txn = FakeTxn::new(claim);

        let handler = dir.route(&path).expect("handler resolved");
        let fut = handler.get(&txn, "tinychain".into()).expect("GET");
        let out = futures::executor::block_on(fut).unwrap();
        assert_eq!(out, "hello tinychain");
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

    #[test]
    fn macro_builds_routes() {
        let dir = tc_library_routes! {
            "/lib/status" => HelloHandler,
        }
        .expect("macro routes");

        let claim = Claim::new(Link::from_str("/lib").unwrap(), umask::Mode::all());
        let txn = FakeTxn::new(claim);
        let path = [segment("lib"), segment("status")];
        let handler = dir.route(&path).expect("handler");
        let fut = handler.get(&txn, "macro".into()).expect("GET");
        let out = futures::executor::block_on(fut).unwrap();
        assert_eq!(out, "hello macro");
    }

    #[test]
    fn scalar_map_roundtrip() {
        let mut inner = Map::new();
        inner.insert(
            "signed".parse().expect("Id"),
            Scalar::from(Value::from(Number::from(true))),
        );
        inner.insert("bits".parse().expect("Id"), Scalar::from(16_u64));

        let mut outer = Map::new();
        outer.insert("dtype".parse().expect("Id"), Scalar::from(Value::from("f32")));
        outer.insert("encoding".parse().expect("Id"), Scalar::Map(inner));

        let scalar = Scalar::Map(outer);

        let encoded = destream_json::encode(scalar.clone()).expect("encode scalar map");
        let decoded: Scalar = futures::executor::block_on(destream_json::try_decode((), encoded))
            .expect("decode scalar map");

        assert_eq!(decoded, scalar);
    }

    #[test]
    fn scalar_tuple_roundtrip() {
        let scalar = Scalar::Tuple(vec![Scalar::from(7_u64), Scalar::from(Value::from("x"))]);

        let encoded = destream_json::encode(scalar.clone()).expect("encode scalar tuple");
        let decoded: Scalar = futures::executor::block_on(destream_json::try_decode((), encoded))
            .expect("decode scalar tuple");

        assert_eq!(decoded, scalar);
    }

    #[test]
    fn scalar_opref_decodes_as_ref() {
        let link = Link::from_str("/lib/acme/foo/1.0.0").expect("link");
        let op = OpRef::Get((Subject::Link(link), Scalar::default()));
        let scalar = Scalar::from(TCRef::Op(op));

        let encoded = destream_json::encode(scalar.clone()).expect("encode scalar ref");
        let decoded: Scalar = futures::executor::block_on(destream_json::try_decode((), encoded))
            .expect("decode scalar ref");

        assert_eq!(decoded, scalar);
    }

    #[test]
    fn opdef_roundtrip() {
        let form = vec![
            ("x".parse().expect("Id"), Scalar::from(7_u64)),
            ("y".parse().expect("Id"), Scalar::from(Value::from("z"))),
        ];
        let op = OpDef::Post(form);

        let encoded = destream_json::encode(op.clone()).expect("encode opdef");
        let decoded: OpDef =
            futures::executor::block_on(destream_json::try_decode((), encoded))
                .expect("decode opdef");

        assert_eq!(decoded, op);
    }

    #[test]
    fn tcref_id_roundtrip() {
        let tcref = TCRef::Id("$foo".parse().expect("IdRef"));
        let encoded = destream_json::encode(tcref.clone()).expect("encode tcref id");
        let decoded: TCRef =
            futures::executor::block_on(destream_json::try_decode((), encoded))
                .expect("decode tcref id");
        assert_eq!(decoded, tcref);
    }

    #[test]
    fn tcref_while_roundtrip() {
        let cond = Scalar::from(1_u64);
        let closure = Scalar::from(Value::from("step"));
        let state = Scalar::from(7_u64);
        let tcref = TCRef::While(Box::new(While::new(cond, closure, state)));
        let encoded = destream_json::encode(tcref.clone()).expect("encode tcref while");
        let decoded: TCRef =
            futures::executor::block_on(destream_json::try_decode((), encoded))
                .expect("decode tcref while");
        assert_eq!(decoded, tcref);
    }

    #[test]
    fn tcref_if_roundtrip() {
        let cond = TCRef::Id("$flag".parse().expect("IdRef"));
        let then = Scalar::from(Value::from("yes"));
        let or_else = Scalar::from(Value::from("no"));
        let tcref = TCRef::If(Box::new(IfRef::new(cond, then, or_else)));
        let encoded = destream_json::encode(tcref.clone()).expect("encode tcref if");
        let decoded: TCRef =
            futures::executor::block_on(destream_json::try_decode((), encoded))
                .expect("decode tcref if");
        assert_eq!(decoded, tcref);
    }

    #[test]
    fn static_library_wraps_schema_and_routes() {
        let schema = LibrarySchema::new(Link::from_str("/lib/service").unwrap(), "1.0.0", vec![]);
        let routes = tc_library_routes! {
            "/lib/status" => HelloHandler,
        }
        .expect("routes");

        let lib: StaticLibrary<FakeTxn, _> = StaticLibrary::new(schema.clone(), routes);
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

