# TinyChain IR interface guidelines

This is the sole normative contract for `tc-ir`. The crate owns transport-neutral
syntax and the minimal invocation and lifecycle interfaces shared by native
runtimes. Application, storage, execution, authorization, and transport policy
belongs downstream.

## Ownership

| Concern | Owner |
| --- | --- |
| Scalar, reference, and operation representation | `tc-ir` |
| Intrinsic syntactic queries and structural hashing | `tc-ir` |
| Shared routing, invocation, view, and transaction interfaces | `tc-ir` |
| Universal State interpretation, native routing, and Classes | `tc-state` |
| Application grammar, installation, dependency policy, digests, and graph scheduling | `tc-server` |

## Algebra

`Scalar`, `TCRef`, `OpRef`, and `OpDef` are the canonical graph vocabulary.
Maps and tuples compose these forms recursively. Control flow is expressed by
ordinary `TCRef` variants; no adapter or application-specific scalar variant is
permitted.

Free-ID discovery and referenced-method visitation are intrinsic recursive
queries on these owning values. Free IDs describe syntax, not an execution
plan. The reference visitor reports individual Link/verb occurrences without
classification or aggregation; they are not application dependencies or
authorization grants. Scheduling, missing-provider and cycle errors, bounded
concurrency, deadlines, and `OpDef` execution belong to the runtime which
consumes them.

## Native routing

`Route<State>` resolves a suffix to one `Handler<State>`. `Handler<State>`
exposes GET, PUT, POST, and DELETE over native values and an explicit
transaction capability; unsupported verbs use its default structured error.
`Public<State>` is the shared route-and-invoke helper.

`Transaction` is a native capability, not a serializable header. Real wire
boundaries carry the canonical `TxnId` through their protocol-defined channel;
authorization stays in that boundary's authenticated claim mechanism. Do not
mirror transaction identity, time, or claims in an IR envelope.

Handlers never receive codecs, HTTP bodies, Python objects, WASM memory, or host
storage. Native composition passes `State` directly. Serialization occurs only
at a transport, persistence, sandbox, or foreign-runtime boundary. `Arc<H>`
delegates `Handler<State>` for heterogeneous recursive routing without a second
dispatch enum.

## Hashing and codecs

The algebra implements `async_hash::Hash` directly. Maps use deterministic key
order and sequences preserve their semantic order. These are structural hashes
of decoded forms, not application digest policy. Hashing must not depend on JSON
whitespace or transport encoding.

Every `FromStream` implementation accepts the form emitted by its `IntoStream`
implementation. Encoding must not implement equality, hashing, cloning,
validation, routing, or local delegation.

## Boundary rule

A one-entry application literal is decoded at the host or client boundary into
an ordinary PUT whose key is its identity `Link` and whose value is its ordinary
definition `State`. It is not an IR variant or envelope. Application roots,
semantic versions, installation rules, digest composition, immutable-version
policy, dependency enforcement, and graph scheduling are deliberately absent
from `tc-ir`.

## Review checklist

1. Is the change part of the reusable IR algebra or its native public contract?
2. Is recursive behavior implemented beside its owning value?
3. Is native execution independent of serialization and transport?
4. Are codecs symmetric and hashes deterministic?
5. Could this policy instead belong to a concrete runtime or adapter boundary?
