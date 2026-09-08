# Contributing to `tc-ir`

`tc-ir` defines the shared intermediate representation TinyChain adapters,
hosts, and tooling consume. Keep it dependency-light and transport-neutral.

## How this crate fits into TinyChain

- Owns the scalar/reference/operation algebra, intrinsic syntactic queries,
  structural hashing, and shared native invocation/lifecycle contracts.
- Defines contracts consumed by State and host runtimes without depending on
  either runtime or on adapters.
- Documents stable semantics in `IR_INTERFACE_GUIDELINES.md`; proposed shapes
  belong in a roadmap until implemented.

## Contribution workflow

1. Read the workspace and crate `AGENTS.md` files.
2. Keep formatting and linting clean: run `cargo fmt` and
   `cargo clippy --all-targets --all-features -D warnings` before sending
   patches.
3. Update `IR_INTERFACE_GUIDELINES.md` when a stable public semantic contract
   changes; do not duplicate private implementation details there.
4. Run `cargo test --all-targets --all-features` and the affected adapter fixture
   tests.
5. Treat a canonical representation change as an explicit breaking change. Do
   not hide it behind optional fields or a second decoder unless a separate
   compatibility contract has been approved.

## Rights and licensing

By contributing to this crate you represent that (a) the work is authored by
you (or you have the necessary rights to contribute it), (b) the contribution is
unencumbered by third-party intellectual property claims, and (c) you transfer
and assign all right, title, and interest in the contribution to The TinyChain
Contributors for distribution under the Apache 2.0 license (see `LICENSE`). No
other restrictions or encumbrances may attach to your contribution.
