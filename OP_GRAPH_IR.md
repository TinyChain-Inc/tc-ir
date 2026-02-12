# TinyChain v2 Op-Graph IR (design)

This document proposes a **transport-neutral, deterministic, statically analyzable** op-graph IR (“Op-graph IR”) for TinyChain v2.

It is designed as a general TinyChain primitive (not an ML-only subsystem) which can express:

- straight-line DAG workloads (numeric, ETL, parsing, feature engineering, etc.)
- fixed-count iteration (`Repeat`) for statically bounded workloads
- **analysis-friendly workloads**: deterministic, conservative analysis of resource usage and safety properties *without Monte Carlo*

Numeric/ML is a motivating example, but **domain-specific operations are not hard-coded into `tc-ir`**. Instead, ops are defined by libraries/services/classes and referenced by canonical URI.

The graph structure is assumed **public**; inputs/params may be **encrypted**.

---

## 0. Design summary (recommended approach)

### Integration choice

Pick **Option C** (a dedicated op-graph payload type) with a deliberate separation of concerns:

- Add a new `tc_ir::op_graph` module defining an `OpGraph` data model and its wire encoding.
- Execute and analyze graphs by routing them to a TinyChain **library/service handler** (which may be provided by TinyChain or by a publisher), which:
  1. validates the graph (topology, declared types, bounded loops),
  2. runs one or more analyzers (bounds, cost/metrics, safety checks) using *declarative operator contracts*,
  3. executes by invoking operator implementations registered by installed libraries/services/classes.

This keeps graphs analyzable without smuggling semantics behind opaque `/lib/...` routes, while building directly on TinyChain’s existing **Library/Service/Class** direction:

- `tc-ir` defines the **container** (`OpGraph`) and deterministic encoding.
- Operators are **declared** as `Class` definitions (shipped by libraries/services) and referenced by canonical URI.
- Analysis relies on **operator-provided analysis contracts** (rules for metrics/bounds/invariants), not on platform-specific “special cases”.

### Separation of concerns (important)

`OpGraph` does **not** define billing policy.

Instead, it enables metering by making workloads *inspectable*:

- deterministic graph structure and explicit operator invocations (`OpRef`),
- explicit type parameters (`TypeSpec`) needed for analysis (e.g., tensor shapes),
- operator-level “analysis contracts” (metric rules, bounds rules, invariants) declared by operator
  libraries and evaluated without executing publisher code.

Billing/metering is a **consumer** of analysis results (outside `tc-ir`), so deployments can apply
their own units, pricing, and risk policies without changing the IR.

### Mapping to `Library` / `Service` / `Class` (explicit)

This design uses the existing TinyChain model as follows:

- **`Class` = operator identity + metadata.**
  - Each operator is a class (or class method) identified by canonical URI.
  - A class may optionally ship a *data-only analysis contract* describing how analyzers should compute
    metrics/bounds/invariants for calls to that operator.

- **`Library` / `Service` = distribution + dependencies + implementation.**
  - A library/service bundles and versions its operator classes under the publisher namespace.
  - The manifest-declared dependency set remains the kernel’s egress allowlist: if a graph node targets an
    operator owned by another component, that dependency must be declared.

- **`OpGraph` = a value type used to compose operators as data.**
  - The op-graph payload is not an API surface; it is a portable value that can be submitted to any analyzer
    or executor service.

- **Analyzer = a service (often Python).**
  - Static analysis is implemented by a service route (or external tool) which consumes `{graph, analysis_inputs, profile, options}`
    and returns a report.
  - Metered FLOP billing is one possible analysis profile among many.

### Why not the other options?

- **Option A (new `TCRef` variant like `TCRef::Graph`)** is the endgame for full composability, but v2 `TCRef` is intentionally minimal today (`TCRef::Op` only). Adding full graph nodes + control-flow refs prematurely risks locking in scheduler details before the kernel’s ref scheduler is complete.
- **Option B (encode compute as standardized `OpRef`s to reserved subjects/paths)** keeps v1 `OpRef` shape, but pushes too much implicit typing and policy into endpoint semantics. Analysis needs explicit type parameters (e.g., shapes) in the payload, not inferred from “what URI was called”.

---

## 1) IR model (Rust): `tc_ir::op_graph`

### Goals for the model

- **Deterministic:** byte-for-byte stable encoding given the same logical graph.
- **Analyzable:** enough metadata is present to compute metrics and check properties, *provided the referenced operators declare analysis contracts*.
- **Host-independent:** the payload is valid without kernel context; the host validates *well-formedness* and resolves referenced operator contracts by URI.
- **Composable:** supports straight-line DAGs plus fixed-count loops (`Repeat`).

### Core types (sketch)

```rust
/// Stable identifier for a value produced by an input or node output.
pub struct ValueId(pub u32);

/// Stable identifier for a node in a graph.
pub struct NodeId(pub u32);

/// A type declaration for a value flowing through a graph.
///
/// This is intentionally **not** a closed enum in `tc-ir`. Instead:
///
/// - `class` points at a canonical type/class path (e.g. `/state/scalar/value/number`,
///   `/state/collection/tensor`, or a publisher-defined `/class/...`).
/// - `params` is a deterministic map of small, serializable parameters (shape, dtype,
///   encoding policy, etc.) whose meaning is defined by the referenced class.
///
/// This keeps `tc-ir` minimal while letting domains (numeric, text, records, etc.)
/// define their own type parameter conventions.
///
/// Note: we intentionally do **not** reuse the v1 “typed-map scalar” encoding here (e.g.
/// `{"<type_path>": <value>}`) because v2 `Scalar` currently interprets unknown `"/..."` maps
/// as `OpRef` (it does not yet support general map/tuple scalar values). `TypeSpec` avoids
/// that ambiguity while staying transport-neutral and dependency-light.
pub struct TypeSpec {
    pub class: tc_ir::Link,
    pub params: Map<Scalar>,
}

pub struct Input {
    pub name: String,
    pub vtype: TypeSpec,
    pub value: ValueId,
}

pub struct Output {
    pub name: String,
    pub value: ValueId,
}

pub struct Node {
    pub id: NodeId,
    pub op: Op,
    pub inputs: Vec<ValueId>,
    pub outputs: Vec<ValueId>, // v0: usually 1; multi-output reserved for v1
    pub output_types: Vec<TypeSpec>,
}
```

### Ops

The op-graph IR does not hard-code a domain-specific op list. Instead, it provides:

- a minimal set of *structural* ops (`Invoke`, `Repeat`)
- a generic `Invoke` which calls an operator expressed as a v1-shaped `OpRef`

Domain libraries define their own operator classes (and optional analysis contracts), and graphs reference them by URI.

```rust
pub enum Op {
    /// Invoke a declared operator via the v1-shaped `OpRef` encoding.
    ///
    /// This intentionally reuses TinyChain's existing op-call envelope to align with the
    /// Library/Service/Class plan:
    ///
    /// - The operator identity is the `OpRef` subject (a canonical URI, typically under `/class/...`).
    /// - The operator parameters are the `OpRef` arguments (`POST` map params or `GET` key).
    ///
/// v0 constraints for analyzability/static analysis:
    ///
    /// - `OpRef` subject must be `Subject::Link` (no `Subject::Ref` / scoped capture).
    /// - `OpRef` should be `POST` for operators with attrs; `GET` may be allowed for pure
    ///   zero-arg operators if needed.
    /// - The referenced operator must have an analyzer-visible contract (see §4.0).
    ///
    /// The v0 IR requires nodes to declare their `output_types` explicitly; type inference
    /// can be added later as an operator contract feature.
    Invoke { op: tc_ir::OpRef },

    // Fixed-count loop with explicit loop-carried state.
    Repeat(Repeat),
}

pub struct Repeat {
    pub count: u64,

    /// Values passed into the body graph each iteration.
    ///
    /// Convention: the first `k` inputs are loop-carried state; the remaining inputs are invariants.
    pub body: OpGraph,

    /// Mapping from outer values into the body's input values.
    pub bind: Vec<(ValueId /* body input */, ValueId /* outer value */)>,

    /// Mapping from body outputs back to outer outputs for the loop-carried values.
    pub carry: Vec<(ValueId /* outer value */, ValueId /* body output */)>,
}

pub struct OpGraph {
    pub version: String,            // IR schema version (semver)
    pub inputs: Vec<Input>,         // deterministic order
    pub nodes: Vec<Node>,           // must be topologically ordered
    pub outputs: Vec<Output>,       // deterministic order
}
```

---

## 1.x) Lazy conditional evaluation (v1-aligned design note)

### Problem statement

Autograph lowers `if` branches into a single `OpDef` where branch expressions are evaluated eagerly. This
forces non-total operations (e.g. tuple `get(0)`, `head`, `tail`) to execute even when the branch is not
taken, causing runtime failures. v1 users expect `if` branches to be lazy.

### v1 continuity choices

- Keep the existing eager `IfRef` (`/state/scalar/ref/if`) for cases where both branches are total.
- Introduce a **lazy** conditional ref for branch OpDefs to preserve v1 expectations of branch semantics.
- Follow v1 naming patterns for boolean ops (`and`, `or`, `not`, `xor`) and avoid reusing bitwise dunders.

### New IR ref: `CondOp` (lazy)

**Name:** `CondOp` (lazy conditional)

**Encoding:** `/state/scalar/ref/cond`

**Shape:** `(cond_ref, then_opdef, else_opdef)`

- `cond_ref`: a `TCRef` that resolves to a boolean scalar (same as v1 boolean ops).
- `then_opdef`: `OpDef` executed only when condition is true.
- `else_opdef`: `OpDef` executed only when condition is false.

**Semantics:**
1. Resolve `cond_ref`.
2. Execute **only** the selected branch OpDef.
3. Return the branch `result` scalar.

This mirrors v1’s behavioral expectation (branch expressions are not evaluated unless selected),
without changing the eager `IfRef` behavior.

### Autograph lowering (v1-aligned)

Autograph should lower `if` assignments into `CondOp` rather than eager `IfRef` when:
- The branch body contains non-total ops (indexing, head/tail, slice), or
- The branch is generated from Python control flow (default).

**Design choice (ab initio minimal):**
- **Lower each `if` block to a single `CondOp` that returns a map of all assigned names.**
- Both branch `OpDef`s return the same map keys, even if one branch computes a value via a
  default/identity expression.
- After the `CondOp`, bind each assigned name by `get` from the returned map.

Rationale: a single lazy `CondOp` per `if` block is the minimal general form that guarantees
branch-local evaluation while keeping the IR surface area small. Per-assignment `CondOp`s
either re-evaluate conditions or force eager evaluation of dependencies, which breaks v1
branch semantics and complicates scheduling. A map result preserves topological order and
keeps name-binding explicit.

### Host execution requirements

Host resolvers should:
- Support `/state/scalar/ref/cond` by executing only the selected branch `OpDef`.
- Preserve eager `IfRef` for total expressions (legacy use).

### Compatibility & migration

- Existing graphs using `IfRef` remain valid.
- Autograph will prefer `CondOp` for correctness; manual authors can still use `IfRef` for total expressions.

### Validation

When validating Autograph + `CondOp` changes, run `cargo test -p tc-ir` and the
relevant downstream integration tests (e.g., Python client flows) for the affected
paths.


#### Determinism constraints

An `OpGraph` is well-formed iff:

- `nodes` are in **topological order** (all inputs to a node refer to:
  - graph inputs, or
  - outputs of strictly earlier nodes).
- `ValueId` names are unique and stable; a value is defined exactly once.
- `Repeat` bodies are well-formed graphs with their own `ValueId` space (no cross-graph capture).
- Any type parameters required for analysis (e.g., tensor shapes/dtypes) are either:
  - concrete in the graph `TypeSpec.params`, or
  - symbolic but resolved/bounded by the analysis input.

---

## 2) Serialization / wire format

### Encoding goals

- **v1-inspired typed payloads:** keep the “type-tagged map” convention so payloads can be carried as JSON values without out-of-band schema negotiation.
- **destream-compatible:** round-trippable via `destream_json` in Rust.
- **medium-graph efficiency:** compact arrays for nodes/values; avoid deeply nested maps in hot paths.

### Proposed JSON shape (v0)

Use a single-entry typed map:

```json
{
  "<OP_GRAPH_TYPE_TAG>": {
    "version": "0.1.0",
    "inputs": [
      {
        "name": "x",
        "type": {
          "class": "<TENSOR_CLASS_URI>",
          "params": {"dtype": "f32", "shape": ["N", "D"], "encoding": "plain"}
        },
        "value": 0
      }
    ],
    "nodes": [
      {
        "id": 0,
        "op": {"<OPERATOR_CLASS_URI>": {"transpose_a": false, "transpose_b": false}},
        "inputs": [0, 1],
        "outputs": [2],
        "output_types": [
          {
            "class": "<TENSOR_CLASS_URI>",
            "params": {"dtype": "f32", "shape": ["N", "H"], "encoding": "plain"}
          }
        ]
      }
    ],
    "outputs": [
      {"name": "y", "value": 2}
    ]
  }
}
```

Notes:

- The outer key `<OP_GRAPH_TYPE_TAG>` is a **type tag**, not a routable endpoint.
- `inputs`, `nodes`, and `outputs` are arrays to preserve deterministic ordering.
- Optional fields must be additive; unknown fields are ignored (forward compatibility).
- Each node’s `"op"` field is encoded using the v1 `OpRef` JSON convention (a single-entry map),
  but nested inside the op-graph payload rather than used as a top-level scalar ref. This reuses
  canonical operator URIs and keeps operator calls aligned across transports.

### Compatibility with v1 `Op` representation

- The Op-graph IR **does not change** `OpRef`/`TCRef` encoding.
- This IR is a separate typed payload intended for analysis/execution by a library/service handler.

---

## 3) Execution semantics (host/kernel)

### Pure function semantics

- Purity is a property of **operators**, not of the container IR.
- `Repeat` is structurally pure given fixed `count`; any effects come from invoked operators.
- For analysis, operators must declare whether they are pure and what effects they may have. v0 analyzers should default-deny any operator which is not explicitly marked analyzable for the requested profile.

### Type/shape checking

The host validates before execution:

- Type parameter well-formedness for referenced `TypeSpec.class` values (e.g., tensor rank is an integer, shapes are non-negative, etc.).
- Operator-specific constraints (shape compatibility, dtype compatibility, required numeric policy boundaries) as defined by the operator class/contract.

### Encrypted vs plaintext constants

Op-graph does not treat “constants” as a special-case IR primitive. A constant is expressed as a
normal operator invocation (an `Invoke` node with zero inputs) whose `OpRef` parameters contain:

- small inline literals (numbers/strings) when safe, and/or
- canonical `/state/media/...` URIs (as strings) for large tensors or ciphertext blobs.

Graph-level optimizations (e.g., eliminate multiply-by-1) are analysis-layer behavior and are only
permitted when the analyzer can prove the operator parameters represent an identity constant for the
target operator semantics.

### Where execution lives

Recommended: ship an analyzer/executor as a library/service handler which exposes:

- `analyze(graph, analysis_inputs, profile, options) -> report`
- `run(graph, inputs, policy?) -> outputs` (optional for v0; can be a stub)

Implementation-wise, execution can be:

- native kernel capability (fast path), or
- a standard library module installed under `/lib/...` with routes.

Either way, the **payload** and **response** stay transport-neutral and compatible with HTTP/PyO3/WASM.

---

## 4) Analysis (enabled by Op-graph; not part of the IR)

Analysis is intentionally **not** built into `OpGraph`. Instead, it is a library/service
concern which consumes:

- `OpGraph` (including fixed `Repeat.count`)
- optional plaintext **analysis inputs** (e.g., bounds/statistics for selected values)
- an explicit **analysis profile** (what to compute and what units to report)

and returns a deterministic **analysis report**. Examples include:

- usage metrics (e.g., FLOPs, comparisons, byte traffic) per iteration and total
- conservative output bounds (intervals, abs-max, norm bounds) for selected values
- accumulated rounding/quantization error bounds for selected numeric encodings
- explicit reasons when analysis is incomplete (unknown shapes, missing operator contracts, unsupported encodings)

### 4.0 Operator analysis contracts (what makes a graph analyzable)

`OpGraph` itself is just a container. Static analysis is possible only if **every referenced operator**
is accompanied by a *data-only* analysis contract.

An operator analysis contract is identified by the operator’s canonical URI (e.g. a `/class/...` path)
and must be usable without executing publisher code. Conceptually, it provides:

- **Type/shape rule:** validation and (optionally) type inference for `TypeSpec.params`
- **Metric rules:** deterministic metric functions (e.g., FLOPs, comparisons, memory traffic) as a function of shapes
- **Bounds rules:** conservative bound propagation rules for accepted analysis input types
- **Invariants:** requirements like “loop-carried values must be clipped/quantized each iteration” for fixed-point safety
- **Purity/effects:** whether the operator is pure, and if not, what effects it has (so analyzers can default-deny)

In practice, the contract should be shipped alongside the operator’s class/type metadata (e.g. under
`/class/...`) and registered at install time, so:

- hosts can analyze offline without bespoke code paths, and
- clients can cache contracts for local preflight analysis.

### 4.1 Python-first enterprise workflow: external analyzers + signed reports

Some enterprise users will implement metering/certification logic in Python and may not have access
to (or the ability to deploy) Rust code. The Op-graph infrastructure supports this by treating
analysis as **data**, not as host logic.

Recommended pattern:

1. A Python analyzer service/tool takes `{graph, analysis_inputs, profile, options}` and produces an analysis report.
2. The report is **signed** by a tenant/operator-controlled key and bound to the exact analyzed inputs
   (graph + profile + options + any analysis inputs) via a digest.
3. TinyChain hosts *may* verify the signature and digest when the report is used for enforcement
   (quota gating, metering, or pre-execution certification requirements).

This keeps `tc-ir` minimal:

- The op-graph container remains transport-neutral data.
- Exact metering or certification logic stays in Python, outside the kernel.
- The kernel, if it participates, only needs generic signature + digest verification.

#### Cross-organization / closed-source boundaries (opaque application ops)

In many real deployments, an op-graph will cross opaque boundaries:

- a node invokes an operator whose implementation is closed-source,
- the operator is operated by a different company, and/or
- the operator executes remotely via RPC as part of a dependency call.

The op-graph infrastructure cannot assume the host can inspect or reproduce that operator’s internal
cost/bounds behavior. Instead, analysis supports **delegation** and **composition**:

1. The outer analyzer treats the operator as opaque by default (`opaque_policy`).
2. The operator’s provider supplies an **attested analysis subreport** for that invocation:
   - a deterministic digest binding the specific invocation (`OpRef` + declared `TypeSpec`s + relevant analysis inputs),
   - the reported metrics/properties for that invocation, and
   - a signature by an authorized key.
3. The outer analyzer composes these subreports into a complete report.

This yields general infrastructure for user-defined analysis across opaque application boundaries:

- Each provider can define its own metric/proof logic for its operator(s).
- The host/kernel only needs generic verification rules (signature + digest binding + allowlist).

**Minimal trust model (recommended):**

- Authorization is by canonical path: a tenant or host config maintains an allowlist
  `operator URI → allowed analysis signer keys`.
- A signed subreport is accepted only if its signer is allowed for the operator URI and its digest matches
  the invocation being analyzed/enforced.

**Aggregation rule (recommended):**

- A top-level report may include `subreports` attached to node ids.
- If a node is opaque but provides a valid subreport, it is no longer opaque for the requested profile.
- If a node has neither a contract nor a valid subreport, it remains opaque and totals are partial/unknown.

### 4.2 Opaque operators (analysis policy)

In practice, graphs will sometimes invoke operators which are opaque to an analyzer (no contract,
or a contract which intentionally omits bounds/metric details). For v0, it is acceptable for an
analyzer to **error out** or **skip** such nodes, as long as the result is explicit.

Define an `opaque_policy` for analysis requests:

- `reject`: fail if any invoked operator lacks a usable contract.
- `ignore`: compute metrics/properties only over the analyzable subgraph and report opaque nodes/values as “unknown”.
- `metric_only` (optional): allow an operator to supply metric rules without full bounds/safety rules.

When `opaque_policy != reject`, the analysis report should include at least:

- `opaque_nodes`: node ids (and operator URIs) which were skipped
- `unknown_cost` / `unknown_metrics`: whether totals are lower bounds (missing opaque contributions)
- `unknown_bounds`: which outputs (or intermediate values) have unknown bounds due to opacity

### 4.3 Example analysis profile: numeric FLOPs + bounds (non-normative)

A standard numeric operator library can define an analysis profile which:

- accepts a strict interval envelope (e.g., tensor `abs_max`) and optional symbolic dim bindings,
- reports metrics like FLOPs per iteration/total, and
- propagates conservative bounds and fixed-point safety invariants.

All of the concrete math rules (matmul FLOP formulas, norm/abs-max bounds, quantization margin checks,
repeat invariants, etc.) live in the **numeric operator library’s contract/profile**, not in `OpGraph`.

---

## 5) Python client integration (API proposal)

### Module layout

Add `client/py/tinychain/compute.py` (and export in `client/py/tinychain/__init__.py`) with:

- dataclasses for `TypeSpec` conventions used by the standard numeric operator library, plus an `OpGraph` builder
- a builder that enforces:
  - deterministic ordering
  - topological constraints
  - basic shape checks (for supported numeric conventions)
- `to_json()` / `from_json()` which implement the typed-map encoding
- thin request helpers that produce `tc.OpRef` objects targeting the compute library/service routes

### Proposed Python surface (sketch)

```python
import tinychain as tc
from tinychain.compute import OpGraph, TensorType, AbsMax, Target, analyze_opref

graph = (
    OpGraph(version="0.1.0")
      .input("x", TensorType(dtype="f32", shape=("N", "D"), encoding="plain"))
      .input("w", TensorType(dtype="f32", shape=("D", "H"), encoding="plain"))
      .matmul("y", "x", "w")         # default operator URI comes from the numeric operator library
      .quantize("yq", "y", signed=True, bits=16, scale_pow2=-8)
      .output("yq")
)

envelope = {
  "x": AbsMax(3.0),
  "w": AbsMax(0.2),
  "dims": {"N": 1024, "D": 256, "H": 128},
}

target = Target(decode_margin=8, require_quantize_each_repeat=True)

analyze = analyze_opref(graph, envelope=envelope, target=target)

with tc.backend(kernel):
    report = tc.execute(analyze)
```

### Analysis report shape (Python)

Return a plain JSON object (stable fields, additive evolution):

```json
{
  "version": "0.1.0",
  "profile": "numeric.flops_bounds.v0",
  "metric_profile_version": "flops-0.1",
  "metrics": {
    "flops_per_iteration": 67108864,
    "flops_total": 67108864
  },
  "bounds": {"yq": {"abs_max": 123.0}},
  "opaque_nodes": [],
  "subreports": [],
  "signature": {"alg": "ed25519", "key_id": "tenant-metering-key-1", "sig": "<base64>"},
  "errors": [],
  "warnings": []
}
```

The Python client should provide:

- a typed `Report` dataclass wrapper (optional in v0)
- `Report.raise_on_error()` for ergonomic failure handling

---

## 6) Compatibility, versioning, and security

### Versioning strategy

- `OpGraph.version` is semantic (e.g., `"0.1.0"`).
- The **type tag** stays stable (`/state/scalar/value/op_graph`); incompatible schema changes bump `version`.
- Analysis reports include a separate `metric_profile_version` so metric rules can evolve without rewriting the IR schema.

### Backward compatibility with existing IR

- No breaking changes to `Scalar`, `OpRef`, or `TCRef` encoding.
- Operator URIs are canonical paths (typically `/class/...`) which follow the existing namespace/version rules.

### Security and DoS limits

Hosts must enforce hard limits before analysis/execution:

- max nodes, max edges, max tensor rank
- max resolved element count per tensor
- max `Repeat.count` and max nested repeats
- max media constant sizes (ciphertexts) and total referenced media bytes per request

Policy:

- Default deny unknown operators during analysis unless an explicit operator contract is available and the operator is marked analyzable for the requested profile.
- Enforce outbound egress rules normally for any operator which performs TinyChain I/O: operator calls still route through the kernel and remain subject to manifest-declared dependency allowlists.

---

## 7) Minimal implementation plan (v0 → v1)

### v0 (minimal, analyzer-first)

1. **`tc-ir`:** Add `tc_ir::op_graph` types + `destream_json` encode/decode; add serialization round-trip tests.
2. **Python client:** Add `tinychain.compute` builder + `to_json()`; add local validation helpers (shape/dtype/toposort).
3. **Host stub endpoint:** Add a `/lib/...` (or `/service/...`) handler that accepts `{graph, analysis_inputs, profile, options}` and returns a structured analysis report JSON. Execution can be a stub returning “not implemented” while analysis ships.
4. **Analyzer skeleton:** Implement:
   - graph validation (topological, typing, shapes, policy presence)
   - cost accounting for supported operator contracts (v0 numeric FLOPs)
   - v0 bound propagation for `AbsMax` + fixed-point checks (as defined by numeric operator contracts)
5. **Rejection ergonomics:** Return actionable error codes/messages:
   - unknown dims without bounds
   - missing policy ops
   - unsupported op/encoding

### v1 (richer IR + stronger bounds)

1. **Symbolic dims:** add richer constraints for `TypeSpec.params` (e.g., `N <= 8192`) and allow partial evaluation.
2. **Better numeric bounds:** accept optional `L2Norm` / row/col norm envelopes for less conservative numeric operator bounds.
3. **More operator libraries:** ship richer standard operator sets (numeric, string/text, record/ETL, etc.) as libraries/classes with contracts, not as `tc-ir` enums.
4. **Lowering + execution:** lower Op-graph into canonical kernel IR at install time for caching, and execute via the shared scheduler.
5. **Importers:** optional ONNX/MLIR importers (client-side) which compile into Op-graph IR.

---

## Open questions (answers)

1) **Where does compute-graph IR live?**

In `tc-ir` as `tc_ir::op_graph` (dependency-light boundary contract). Avoid a new crate unless and until the IR becomes large enough to merit splitting.

2) **How are tensor shapes represented?**

As type parameters under `TypeSpec.params`. A numeric tensor convention can use:

- `shape: ["N","D",...]` for symbolic dims
- `dtype: "f32" | "f64" | ...` for numeric types
- `encoding: "plain" | {...}` for explicit numeric encoding policy

Certification requires concrete values (or upper bounds) for any symbolic dims to compute costs and safety checks.

3) **How do we require numeric policy ops so analysis/certification is meaningful?**

Numeric policy is expressed as **operators** (e.g. `clip`, `rescale`, `quantize`, `cast`) defined by a numeric operator library. Certification relies on those operators’ contracts and can enforce invariants such as:

- fixed-point boundaries at graph I/O, and
- per-iteration policy ops on loop-carried values within `Repeat`.

4) **How do we integrate with existing `OpRef` semantics without opaque routes?**

Keep the compute IR self-describing and analyzable. Execution is implemented by a handler which:

- validates + analyzes the payload, then
- optionally lowers it into `TCRef`/`OpRef` graphs (canonical IR) so the kernel scheduler executes it with the same deterministic semantics as other graphs.

The handler is an implementation detail; the program IR remains transparent and portable.
