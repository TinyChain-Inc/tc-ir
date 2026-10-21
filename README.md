# tc-ir

`tc-ir` owns TinyChain's transport-neutral scalar, reference, and operation
algebra and the small native contracts shared by runtimes.

## Owned contracts

- `Scalar`, `TCRef`, `OpRef`, `OpDef`, `Map`, and `Id` describe values and
  deferred computation.
- `Transaction` and `Transact` describe protocol identity and resource
  lifecycle.
- `Method`, `Handler`, `Route`, and `Public` form the native verb and routing
  boundary. A handler exposes only its supported verb closures.
- `IntoView` acquires a transaction-consistent native view independently of
  wire encoding.

The crate does not interpret `State`, classify applications, schedule graphs,
resolve dependencies, define installation payloads, or own runtime and adapter
policy. The normative boundary is defined by the
[IR interface guidelines](IR_INTERFACE_GUIDELINES.md).

`Scalar`, `TCRef`, `OpRef`, and `OpDef` are ordinary enums. Match their
variants directly. Clone, comparison, hashing, formatting, and destruction
follow ordinary recursive ownership; codecs retain streaming traversal.

Decoding accepts at most 16 structural levels, counting nested containers,
references, and operations across decoder delegation, including embedded Values.
Wire envelopes and sibling counts do not consume structural depth. The limit is
reduced from 64 to permit typed recursive decoding with normal-stack headroom;
deeper requests are rejected. Native constructors and runtime composition must
respect this contract: codec limits
do not bound program-generated structures, whose recursive operations can
otherwise overflow the stack. Invocation and execution budgets belong to the runtime.

Converting Scalar into Value still checks combined tuple depth before construction.

## Development

```bash
cargo test --all-targets --all-features
```

Changes to an IR form require symmetric codec tests. Changes to hashing require
deterministic golden tests. See the [crate notes](AGENTS.md). The TinyChain
[workspace architecture](https://github.com/TinyChain-Inc/tcv2/blob/main/ARCHITECTURE.md)
is non-normative integration context.

Unit-test implementations live under `tests/unit/`, linked from their owning
modules to preserve private access and test names. Shared test fixtures belong
under `tests/common/`; production types must not acquire fixture fields.
