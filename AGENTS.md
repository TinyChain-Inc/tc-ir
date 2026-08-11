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
- Native `Route` and handler traits exchange runtime values only. They must not
  expose encoders, decoders, response bodies, adapter contexts, or generalized
  serialization-result envelopes. `IntoView` is a separate terminal capability,
  never part of route resolution or handler execution.
- `Public<State>` is the one uniform verb dispatcher: it owns route lookup and
  method-not-allowed behavior, while resolved `Handler<State>` implementations
  receive only typed native requests and `State::Transaction`.
- Native state is part of the route contract and must always be explicit. Define
  `Route<State>`, `Handler<State>`, and library route tables with the canonical native
  state type; never default `State` to `()` or infer it through a phantom transaction.
- Do not add per-verb traits with associated request, response, error, or future
  types. `Handler<State>` is the sole native verb contract; boundary ABIs may
  define their own explicitly boundary-local call interfaces.
- Local native composition must preserve values and the exact transaction
  capability end-to-end. Calling another installed component or nested `OpDef`
  must not encode, decode, clone through a wire shape, or materialize a stream.
- Native route futures must preserve downstream readiness and cancellation.
  Handlers may return structured resource exhaustion, but must not hide pressure
  behind a route-specific queue, detached task, eager buffer, or retry loop.
- `Transaction` is the one protocol identity contract. Lower crates may define
  orthogonal, semantically named capabilities delegated by a transaction, but must
  not define another trait named `Transaction` or duplicate its identity fields.
- `Transact` is uniformly fallible: commit, rollback, and finalize return
  `TCResult<()>`, and structural implementations propagate child failures.

## Testing and documentation

- Run `cargo test -p tc-ir` after modifying IR structures or macros. Add unit tests for
  serialization and round-tripping rather than layering fallbacks.
- Document new IR fields or macros in `IR_INTERFACE_GUIDELINES.md` so downstream
  adapters and library authors stay aligned.
