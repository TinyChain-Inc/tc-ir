# tc-ir contributor notes

The [IR interface guidelines](IR_INTERFACE_GUIDELINES.md) are this crate's sole
normative contract. Do not duplicate or broaden that contract here.

- Keep dependencies transport-, storage-, runtime-, and application-neutral.
- Put intrinsic recursive behavior beside its owning algebraic form.
- Use `async_trait` only for the object-safe heterogeneous handler boundary.
- Add symmetric codec tests for representation changes and deterministic golden
  tests for structural-hash changes.
- Run `cargo test --all-targets --all-features` before submitting a change.
