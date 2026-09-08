# tc-ir

`tc-ir` owns TinyChain's transport-neutral scalar, reference, and operation
algebra and the small native contracts shared by runtimes.

## Owned contracts

- `Scalar`, `TCRef`, `OpRef`, `OpDef`, `Map`, and `Id` describe values and
  deferred computation.
- `Transaction` and `Transact` describe protocol identity and resource
  lifecycle.
- `MethodCall`, `Handler`, `Route`, and `Public` form the native verb and
  routing boundary.
- `IntoView` acquires a transaction-consistent native view independently of
  wire encoding.

The crate does not interpret `State`, classify applications, schedule graphs,
resolve dependencies, define installation payloads, or own runtime and adapter
policy. The normative boundary is defined by the
[IR interface guidelines](IR_INTERFACE_GUIDELINES.md).

## Development

```bash
cargo test --all-targets --all-features
```

Changes to an IR form require symmetric codec tests. Changes to hashing require
deterministic golden tests. See the [crate notes](AGENTS.md) and workspace
[architecture](../ARCHITECTURE.md).
