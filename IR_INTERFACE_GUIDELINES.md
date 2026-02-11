# TinyChain IR Access Guidelines

These guidelines describe the interaction patterns TinyChain IR must support so multiple bindings (WASM, PyO3, etc.) can share consistent semantics. Detailed trait definitions and signatures will be specified later.

## Interaction patterns

- **Stateless operations:** Support simple request/response logic that reads transactional state without mutating it.
- **Stateful operations:** Allow methods that mutate collections or resources while holding an exclusive transactional lock.
- **Asynchronous tasks:** Provide a way to run longer-lived or IO-bound work under scheduler control, with explicit cancellation and retry semantics.
- **Per-method registration:** Each handler declares its HTTP verb/path support at compile time via dedicated `get/put/post/delete` methods. The default implementations return `Err(TCError::method_not_allowed(..))`, so handlers only override the verbs they support while routers get structured errors for unsupported verbs.
- **Typed inputs/outputs:** Handler impls should operate on concrete Rust types; the L0 runtime (e.g. `tc-server`) is responsible for deserializing ingress requests into those types and serializing responses back to `State`/`Value` at the boundary, so handler graphs stay fully typed internally.
- **Zero-cost sync support:** Even though handlers use async-friendly futures (GATs), a purely synchronous handler can set `type Fut<'a> = core::future::Ready<Result<...>>` (or another concrete future) and return `future::ready(...)`, avoiding heap allocations entirely. Reserve boxed futures for handlers that truly need dynamic dispatch.
- **Reusable handler instances:** Handlers are expected to be long-lived structs registered at compile time. Once constructed, they should be callable many times (even inside tight loops) without cloning or rerouting through HTTP-style dispatch. Compose ops by invoking handlers/functions directly with their typed inputs rather than re-routing to `/state/<collection>/add` on each iteration.
- **Method-not-supported signaling:** The per-verb methods return a `TCResult`; the default implementations yield `TCError::method_not_allowed`, so handler implementations only override the verbs they actually serve.

### Library helpers

- Use the provided `tc_ir::StaticLibrary` when you want to bundle a `LibrarySchema` with a reusable routing table. It implements the `Library` trait directly, so runtimes can return it from factory methods without extra boilerplate.
- Build route tables with the `tc_library_routes!` macro. It accepts string paths (e.g., `"/hello/world"`) and produces a validated `Dir` so you don’t have to manage `PathSegment` vectors manually.
- See `tc-wasm/src/lib.rs`’s `example` module for a complete snippet (`hello_library`) that composes these helpers and can serve as a starting point for WASM crates.

## Context requirements

Every IR invocation should implicitly receive at least:

- A transactional guard to allow ordering.
- The set of permissions/capability bits granted to the caller.
- The latest deterministic host health snapshot applicable to the shard.

Bindings should hide these details from user code but must honor them under the hood.

## Determinism & purity guidelines

- Handlers may not rely on local wall-clock time; they should use transaction-provided timestamps.
- Inputs and outputs must be serializable with a `destream`.

## Op-graph payloads

TinyChain v2 supports a transport-neutral op-graph payload intended to make workloads **inspectable**
as data (deterministic DAG + fixed-count `Repeat`), so libraries/services can perform static analysis
(metrics, conservative bounds, safety checks) without baking policy (billing, pricing, risk) into the IR.

The proposed payload and its design constraints live in `tc-ir/OP_GRAPH_IR.md`.

## Scalar reference control flow

- `TCRef::While` is encoded as `/state/scalar/ref/while` with a three-element tuple
  `[cond, closure, state]`, mirroring v1 semantics.
- `TCRef::If` is encoded as `/state/scalar/ref/if` with `[cond, then, or_else]`, where
  `cond` is a scalar ref and the branches are arbitrary scalars.
- The loop condition and closure are OpDefs, executed with a loop-carried `state` input.

## Error & backpressure expectations

- Handlers report standardized error categories (authorization, validation, transient, etc.) so callers can take consistent action.
- Asynchronous/streaming handlers must signal when they need to yield or when backpressure should be applied, without leaking implementation-specific types.

## Validation guidance

`tc-ir` will eventually ship compliance tests to confirm that a binding:

1. Declares which interaction patterns each handler uses.
2. Produces deterministic results when invoked repeatedly with identical inputs.
3. Enforces capability masks consistently across all patterns.
4. Cooperates with the global scheduler for asynchronous and streaming workloads.

## Authorization alignment

- Authorization data will be the same used by the upstream control plane (e.g., the a16z server reference implementation). To stay in sync:
  - Control-plane services issue short-lived tokens that embed principal ID, tenant ID, capability bits, and quota hints. Bindings consume these tokens via the implicit authorization context, not by parsing headers manually.
  - Trait implementors must treat capability bits as the sole source of truth for what an operation may do; no handler should hard-code policy independent of the control plane.
  - When the control plane updates capability definitions or tenant policies, bindings must be able to reload the new policy bundle without code changes.
- The IR guidelines here define how handlers *consume* authorization; the actual issuance, validation, and rotation flows remain centralized in the control-plane/a16z server stack. Any divergence between the two must be treated as a compatibility bug.
