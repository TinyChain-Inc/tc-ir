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

1. Align proposed changes with `AGENTS.md` in this repository (graph-first
   modeling, shared primitives, no bespoke transports).
2. Keep formatting and linting clean: run `cargo fmt` and
   `cargo clippy --all-targets --all-features -D warnings` before sending
   patches.
3. Add or update documentation in `IR_INTERFACE_GUIDELINES.md` whenever you
   introduce, rename, or deprecate IR structures or macros.
4. Run `cargo test -p tc-ir` to validate serialization/round-trip behavior, plus
   any adapter tests that exercise the new surface area.
5. Highlight downstream migration steps in your PR so adapters can adopt the
   revised IR without guesswork.

## Rights and licensing

By contributing to this crate you represent that (a) the work is authored by
you (or you have the necessary rights to contribute it), (b) the contribution is
unencumbered by third-party intellectual property claims, and (c) you transfer
and assign all right, title, and interest in the contribution to The TinyChain
Contributors for distribution under the Apache 2.0 license (see `LICENSE`). No
other restrictions or encumbrances may attach to your contribution.
