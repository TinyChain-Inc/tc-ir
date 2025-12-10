# Contributing to `tc-ir`

`tc-ir` defines the shared intermediate representation (IR) TinyChain adapters,
hosts, and tooling consume. Keep it dependency-light and semantically stable so
every part of the stack can agree on manifests, scalar ops, and graph topology
without mirroring host internals.

## How this crate fits into TinyChain

- Acts as the canonical schema for `Scalar`, `TCRef`, manifests, and operation
  graphs that adapters compile before handing work to `tc-server`.
- Serves as the contract between higher-level runtimes (Python, WASM, future
  bindings) and the kernel, so behavior changes here ripple across the fleet.
- Documents IR expectations in `IR_INTERFACE_GUIDELINES.md`, keeping new fields
  versioned and backward-compatible.

## Contribution workflow

1. Align proposed changes with the repo-wide design guidance in `/AGENTS.md`
   (graph-first modeling, shared primitives, no bespoke transports).
2. Follow the shared style rules in `/CODE_STYLE.md` (grouped imports, `cargo fmt`,
   and `cargo clippy --all-targets --all-features -D warnings`). Crate-specific notes
   should reference that doc rather than redefining formatting.
3. Add or update documentation in `IR_INTERFACE_GUIDELINES.md` whenever you
   introduce, rename, or deprecate IR structures or macros.
4. Run `cargo test -p tc-ir` to validate serialization/round-trip behavior, plus
   any adapter tests that exercise the new surface area.
5. Highlight downstream migration steps in your PR so adapters can adopt the
   revised IR without guesswork.

## Rights and licensing

By contributing to this crate you represent that (a) the work is authored by
you (or you have the necessary rights to contribute it) and (b) you transfer and
assign all right, title, and interest in the contribution to the TinyChain
Open-Source Project for distribution under the TinyChain open-source license
(Apache 2.0, see the root `LICENSE`). No other restrictions or encumbrances may
attach to your contribution.
