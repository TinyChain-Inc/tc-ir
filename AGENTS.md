# tc-ir Agent Notes

`tc-ir` is the boundary contract for adapters and hosts. Keep it dependency-light and
stable so other crates can depend on it without pulling in runtime-specific baggage.

## API expectations

- Favor small, composable IR primitives (`Link`, `Claim`, library manifests) that
  remain serializable and deserializable without host context. Do not introduce types
  that require kernel state to construct or validate.
- Keep changes backwards-compatible with v1 HTTP semantics and manifest formats. If a
  schema changes, provide an adapter layer or versioned field rather than breaking
  existing clients.
- Resist adding feature-flagged behaviors that fork the IR surface; shared envelopes
  should remain transport-agnostic.

## Testing and documentation

- Run `cargo test -p tc-ir` after modifying IR structures or macros. Add unit tests for
  serialization and round-tripping rather than layering fallbacks.
- Document new IR fields or macros in `IR_INTERFACE_GUIDELINES.md` so downstream
  adapters and library authors stay aligned.
