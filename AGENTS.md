# tc-ir Agent Notes

`tc-ir` is the lowest shared owner of TinyChain application identity, graph
structure, semantic analysis, and native handler contracts.

- Keep it transport-, storage-, runtime-, and policy-neutral. An IR value must
  not require a kernel, filesystem, HTTP request, Python object, or WASM engine
  to construct or validate.
- Define each semantic shape once. Reuse the shared visitor for hashing,
  dependency discovery, and reflection; do not add a second recursive walk in
  an adapter or host.
- `ApplicationTarget` is the sole application-path parser.
  `ApplicationDefinition<T>` remains a literal one-entry definition. A breaking
  canonical change replaces the old form deliberately; do not add optional
  compatibility fields or decoders by default.
- Native routes exchange only `State` and its explicit transaction capability.
  `Handler<State>` is the sole GET/PUT/POST/DELETE contract, and `Route<State>`
  returns the owning handler. Views, codecs, and adapter envelopes do not belong
  in routing.
- Use `async_trait` only for the object-safe heterogeneous handler boundary.
  Prefer native async traits for statically dispatched capabilities.
- `Transaction` is the protocol identity contract and `Transact` is the
  uniformly fallible resource lifecycle. Do not define competing transaction
  traits or default transaction parameters.
- Keep native composition serialization-free and preserve readiness and
  cancellation through every returned future.

Run `cargo test --all-targets --all-features` after changing IR, codecs,
application parsing, analysis, or handler contracts. Update
[IR_INTERFACE_GUIDELINES.md](IR_INTERFACE_GUIDELINES.md) only for stable public
semantics, not proposed implementation designs.
