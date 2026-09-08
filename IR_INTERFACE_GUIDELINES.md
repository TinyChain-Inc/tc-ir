# TinyChain IR Interface Guidelines

This document defines the stable boundaries exposed by `tc-ir`. Concrete
storage, execution, authorization, and transport policy belongs to downstream
owners.

## Native routing

`Route<State>` resolves a path to one `Handler<State>`. `Handler<State>` exposes
GET, PUT, POST, and DELETE over native values and the explicit transaction
capability; unsupported verbs use its default structured error. `Public<State>`
is the shared route-and-invoke helper.

Handlers never receive codecs, HTTP bodies, Python objects, WASM memory, or host
storage. In-process composition passes native `State` directly. Serialization
occurs only at a transport, persistence, sandbox, or foreign-runtime boundary.
`Arc<H>` delegates `Handler<State>` so recursive directories can contain one
heterogeneous handler tree without another dispatch enum.

## Application identity and definitions

`ApplicationDefinition<T>` contains exactly one mapping from a canonical
application identity to its complete definition:

```text
/{kind}/{publisher}/{resource...}/{semantic-version} -> definition
```

There is at least one resource segment. `.txfs` and semantic-version-shaped
resource segments are reserved. `ApplicationTarget` parses namespace targets,
complete identities, and unmatched route suffixes once; downstream code must
not reconstruct or rescan the path.

Definitions have no generic package, artifact, schema, version, digest, or
payload envelope. Semantic digests and application requirements are derived
from the definition. Unsupported persisted or wire forms fail closed.

## Graph analysis

`Scalar`, `TCRef`, `OpRef`, and `OpDef` are the canonical graph vocabulary.
`OpPlan::compile` validates providers, detects cycles, and returns deterministic
dependency levels. `OpDef::free_ids` and `Scalar::application_requirements`
reuse the same structural traversal.

Application requirements are keyed deterministically by identity and aggregate
the required native methods. Conflicting authorities for one identity are
invalid. Resolution of a requirement to a committed digest and executable
authority belongs to the host, not the IR.

Execution follows data dependencies, not lexical source order. Independent
nodes may run concurrently. A side effect that requires ordering must use an
explicit dependency such as `After`; bindings must not infer sequencing from
source order.

## Control references

Conditional, sequential, parameter, `while`, and `for_each` forms are ordinary
`TCRef` variants. Their codecs are symmetric and their traversal order is
deterministic. They carry graph semantics only; deadline enforcement,
scheduling, admission, and `OpDef` execution belong to the runtime.

## Semantic hashing and codecs

Semantic hashing uses the shared format-neutral visitor with explicit domain
tags, collection lengths, and deterministic map ordering. It must not depend on
JSON whitespace or transport encoding.

Every `FromStream` implementation accepts exactly the form emitted by its
`IntoStream` implementation. Codec changes require valid and malformed fixtures
which other language bindings can consume. Encoding is never used internally
for equality, hashing, cloning, validation, or routing.

## Review checklist

A change to `tc-ir` should answer:

1. Is this the lowest shared owner of the semantic contract?
2. Does it reuse the existing application parser and structural visitor?
3. Is native execution still independent of serialization and transport?
4. Are ordering and failure behavior deterministic?
5. Do symmetric codecs and cross-language fixtures cover the change?
