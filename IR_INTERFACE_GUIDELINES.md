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

Lexical-requirement discovery and referenced-method visitation are intrinsic
recursive queries on these owning values. `requires` reports identifiers used
but not bound by the inspected form; it neither creates nor mutates a runtime
namespace and is not an execution plan. The reference visitor reports
individual Link/verb occurrences without classification or aggregation; they
are not application dependencies or authorization grants. Scheduling,
missing-provider and cycle errors, bounded concurrency, deadlines, and `OpDef`
execution belong to the runtime which consumes them.

## Lexical scope

An `OpDef` is an immutable, single-assignment lexical graph. Invocation inputs
and an optional bound `$self` form its outer frame; form entries are immutable
providers. Every provider is visible throughout its form, so forward references
are valid. Resolution never escapes this frame into an application, filesystem,
process-global, or mutable namespace.

Every `OpDef` must contain at least one provider; the last provider is its
capture. Empty read and write operations are rejected uniformly rather than
having verb-dependent defaults. Independent ready providers may execute
concurrently. Any required ordering, including ordering between effects, must be
represented by a lexical dependency or `After`.

| Form | Bindings introduced |
| --- | --- |
| GET | key parameter |
| PUT | key and value parameters |
| POST | request-map entries supplied at invocation |
| DELETE | key parameter |
| Form entry | one provider, visible throughout that form |
| `ForEach` | `item_name`, visible only in its body |
| `While` | reserved `$state`, visible only in its condition and body |
| `Cond`, `After` | none |

`$self` is reserved and exists only for bound execution. Explicit parameter,
provider, and control-binder names must be unique across every visible enclosing
scope: rebinding and shadowing are invalid. Sibling scopes do not see one
another's private bindings. Client compilers may generate deterministic unique
temporary names, but only before emitting IR; generated names obey the same IR
rules afterward.

Nested operations may capture visible enclosing bindings. Standalone operations
may retain unresolved captures for a later invocation context. `requires(&mut
BTreeSet<Id>)` adds only syntactically referenced names not bound by the inspected
form. Mutating this caller-owned accumulator is dependency collection, not
namespace mutation. `ForEach` subtracts its item binding from body requirements;
`While` subtracts `$state` from its callbacks while retaining requirements of
its initial-state expression. Implementations must traverse untrusted recursive
syntax with an explicit work stack rather than relying on the native call stack.

At POST invocation, every required input must be present. Generic IR permits
additional request-map entries because a concrete handler may intentionally
accept them; an owning handler may impose a stricter schema. Before scheduling,
the executor rejects duplicate inputs, duplicate providers, input/provider
overlap, missing names, and cycles. Each resolved ID is cached once, and each
ready set observes an immutable snapshot of prior results.

## Native routing

`Route<State>` resolves a suffix to one `Handler<State>`. A handler
synchronously selects an optional GET, PUT, POST, or DELETE closure; only that
selected closure erases its asynchronous future. An absent closure is the
structured unsupported-verb result. `Public<State>` is the shared
route-and-invoke helper. There is no second call enum carrying a method and its
arguments.

`Public` calls `Route::route` exactly once for the selected verb invocation.
After routing selects a `Handler`, that handler is terminal for the current
request: its verb closure executes the operation or delegates directly to an
already-selected leaf closure, but must not route the same path again. Recursive
namespace traversal belongs in `Route`. A genuinely new nested invocation uses
the runtime executor so its target, transaction scope, and authorization are
preserved.

Represent each selected operation with its concrete terminal handler. Such a
handler may borrow the common route owner; it does not need to clone or wrap an
owned copy of that resource. Do not replace these terminal types with an
operation tag or aggregate handler, because selecting that tag inside a verb
method creates a second dispatcher after `Route`.

`Transaction` is a native capability, not a serializable header. Real wire
boundaries carry the canonical `TxnId` through their protocol-defined channel;
authorization claims are server-owned policy and stay in that boundary's
authenticated mechanism. The shared transaction capability exposes only its
`TxnId`; it does not expose authorization, wall-clock time, or protocol state. Do not
mirror transaction identity, time, or claims in an IR envelope.

Handlers never receive codecs, HTTP bodies, Python objects, WASM memory, or host
storage. Native composition passes `State` directly. Serialization occurs only
at a transport, persistence, sandbox, or foreign-runtime boundary. The concrete
route owner constructs its handler directly. Blanket smart-pointer
or reference implementations are prohibited because they obscure the owning
route and duplicate delegation.

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
6. Does every emitted operation obey single assignment and lexical no-shadowing?
