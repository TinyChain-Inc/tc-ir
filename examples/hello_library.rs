// Example `Library` that routes `/hello` to a friendly handler.
//
// Run with:
// ```
// cargo run --example hello_library
// ```

use std::{future::Future, pin::Pin, str::FromStr};

use futures::executor::block_on;
use pathlink::Link;
use tc_error::TCResult;
use tc_ir::{
    parse_route_path, tc_library_routes, Claim, HandleGet, Library, LibraryModule, LibrarySchema,
    NetworkTime, Route, Transaction, TxnId,
};
use umask::Mode;

#[derive(Clone)]
struct ExampleTxn {
    claim: Claim,
}

impl ExampleTxn {
    fn new(claim: Claim) -> Self {
        Self { claim }
    }
}

impl Transaction for ExampleTxn {
    fn id(&self) -> TxnId {
        TxnId::from_parts(NetworkTime::from_nanos(42), 0)
    }

    fn timestamp(&self) -> NetworkTime {
        NetworkTime::from_nanos(42)
    }

    fn claim(&self) -> &Claim {
        &self.claim
    }
}

struct HelloHandler;

impl HandleGet<ExampleTxn> for HelloHandler {
    type Request = String;
    type RequestContext = ();
    type Response = String;
    type Error = tc_error::TCError;
    type Fut<'a> = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send + 'a>>;

    fn get<'a>(&'a self, _txn: &'a ExampleTxn, name: Self::Request) -> TCResult<Self::Fut<'a>> {
        Ok(Box::pin(async move { Ok(format!("Hello, {name}!")) }))
    }
}

#[cfg_attr(test, allow(dead_code))]
fn main() -> TCResult<()> {
    run_example()
}

pub fn run_example() -> TCResult<()> {
    // This schema would typically match the manifest you publish via `/lib`.
    let schema = LibrarySchema::new(
        Link::from_str("/lib/examples/hello").expect("example link"),
        "0.1.0",
        vec![],
    );

    // Mount `/hello` to the HelloHandler to build a reusable route directory.
    let routes = tc_library_routes! {
        "/hello" => HelloHandler,
    }?;

    let library: LibraryModule<ExampleTxn, _> = LibraryModule::new(schema, routes);
    let path = parse_route_path("/hello")?;
    let handler = library
        .routes()
        .route(&path)
        .expect("handler registered at /hello");

    let txn = ExampleTxn::new(Claim::new(
        Link::from_str("/lib/examples/hello").expect("claim link"),
        Mode::all(),
    ));

    // Dispatch the GET handler with a simple request body.
    let fut = handler.get(&txn, "TinyChain".to_string())?;
    let greeting = block_on(fut)?;
    println!("{greeting}");

    Ok(())
}
