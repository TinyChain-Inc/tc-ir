mod hello_example {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/examples/hello_library.rs"
    ));
}

#[test]
fn hello_example_runs() {
    hello_example::run_example().expect("hello example should run");
}
