# tc-ir

`tc-ir` owns TinyChain's transport-neutral intermediate representation and the
native routing contracts shared by hosts, state, libraries, and adapters.

## Owned contracts

- `Scalar`, `TCRef`, `OpRef`, and `OpDef` describe deferred computation.
- `OpPlan` deterministically validates and schedules graph dependencies.
- `ApplicationIdentity`, `ApplicationTarget`, and
  `ApplicationDefinition<T>` define the canonical application URI and literal
  one-entry definition.
- The shared structural visitor provides semantic hashing, free-ID discovery,
  and application-reference analysis without parallel walks.
- `Route<State>`, `Handler<State>`, and `Public<State>` are the single native
  routing and verb-dispatch boundary.
- `Link`, `Claim`, `Transaction`, and `Transact` carry protocol identity,
  authority, and resource lifecycle semantics without host state.

The crate does not own graph execution, storage, HTTP, PyO3, WASM execution,
application installation, or authorization policy. Those layers consume these
contracts without changing their representation.

## Applications

An application definition has exactly one entry:

```text
/{class|lib|service}/{publisher}/{resource...}/{version} -> definition
```

`ApplicationTarget` is the sole parser for application namespace, identity, and
route suffixes. Digests and dependency requirements are derived from canonical
IR; they are not supplied in a package or metadata envelope.

## Development

```bash
cargo test --all-targets --all-features
```

Changes to a wire or semantic contract require symmetric codec tests,
deterministic analysis tests, and fixtures usable by other language bindings.
See [the interface guidelines](IR_INTERFACE_GUIDELINES.md), the
[crate invariants](AGENTS.md), and the workspace
[architecture](../ARCHITECTURE.md).
