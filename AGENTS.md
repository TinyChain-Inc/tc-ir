# tc-ir contributor notes

The [IR interface guidelines](IR_INTERFACE_GUIDELINES.md) are this crate's sole
normative contract. Do not duplicate or broaden that contract here.

- Keep dependencies transport-, storage-, runtime-, and application-neutral.
- Put intrinsic recursive behavior beside its owning algebraic form.
- Keep lexical queries owner-local: `Scalar`, `TCRef`, `OpRef`, and `OpDef`
  recursively implement `requires`, while `OpDef::validate` enforces immutable
  single assignment and no shadowing. Do not add a namespace, symbol table,
  execution plan, application lookup, or client-specific validator here.
- Keep `Handler` object-safe by selecting synchronous verb closures which alone
  return boxed futures. Do not use or re-export `async_trait`, add a native-call
  enum, or add blanket smart-pointer/reference handler implementations.
- Add symmetric codec tests for representation changes and deterministic golden
  tests for structural-hash changes.
- Run `cargo test --all-targets --all-features` before submitting a change.

- Match Value, Scalar, TCRef, State, and StateView variants directly. Codecs
  enforce 16 structural levels; trusted native composition must respect that
  bound. Conversions into Value check its 64-tuple limit, including embedded
  Values, before construction. Keep execution stack safety separate from data
  ownership.
