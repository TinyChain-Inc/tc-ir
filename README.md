# tc-ir

`tc-ir` defines the TinyChain intermediate representation—the transport-neutral
graph that adapters, hosts, and libraries share. Every handler, route, and
transaction compiles down to these primitives so behavior stays identical across
HTTP, PyO3, WASM, or future transports.

## Key concepts

- **`TCRef` & `Scalar`.** The building blocks for graph nodes. `Scalar::Op`
  wraps verbs (`Get`, `Put`, `Post`, `Delete`), while `Scalar::Ref` points to a
  `TCRef` in the compiled graph.
- **`Link` & `Claim`.** Lightweight descriptors for ledger references and access
  control that downstream crates (`tc-chain`, `tc-server`) rely on without
  needing host context.
- **Libraries and routes.** `LibrarySchema`, `LibraryModule`, `Dir`, and the
  `tc_library_routes!` macro make it easy to declare `/lib/...` manifests that
  compile once but run across adapters.
- **Examples.** `examples/hello_library.rs` demonstrates how to build a native
  library module and dispatch handlers without HTTP. WASM-facing examples live
  under `tc-wasm/examples`.

See `AGENTS.md` for design constraints (dependency hygiene, backward
compatibility) and `IR_INTERFACE_GUIDELINES.md` for field-by-field documentation.

## Building & testing

```bash
cargo build -p tc-ir
cargo test  -p tc-ir
cargo run   -p tc-ir --example hello_library
```

Run the example to verify route registration and handler dispatch work as
expected. When you touch serialization logic, add round-trip tests to catch
regressions early.

## Extending the IR

1. **Stay transport-neutral.** Avoid types that require host-specific context or
   global state. If a new capability is needed, encode it in terms of existing
   primitives (`Scalar`, `Link`, `Claim`) so every adapter can understand it.
2. **Version consciously.** If a schema needs an additional field, add it in a
   backward-compatible way (e.g., `Option`al fields) and document the change in
   `IR_INTERFACE_GUIDELINES.md`.
3. **Macros and helpers.** Prefer compile-time helpers like
   `tc_library_routes!` for repetitive patterns. Keep them small and well-tested
   so downstream crates can trust the generated structures.
4. **Testing discipline.** Run `cargo test -p tc-ir` and update examples when a
   change affects public APIs. The IR is a contract—the tests are the first line
   of defense against breaking other crates.

## Related references

- Workspace `ARCHITECTURE.md` – IR and adapter sections.
- `tc-server/src/library.rs` – shows how the host consumes `LibraryModule` and
  route helpers.
- `tc-wasm` – pairs this IR with WASM artifacts for distribution.
