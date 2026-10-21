#![forbid(unsafe_code)]

//! TinyChain's scalar and operation algebra, plus its shared transaction,
//! routing, and view contracts.

pub use hr_id::Id;

/// A fixed-size SHA-256 digest, shared by semantic hash producers and consumers.
pub type Sha256Hash = async_hash::Output<async_hash::Sha256>;

mod txn;
pub use txn::*;

mod public;
pub use public::*;

mod view;
pub use view::*;

mod map;
pub use map::Map;

mod codec;
pub use codec::{decode_op_def_entry, decode_scalar};

/// Maximum decoded structural nesting, shared with enclosing State decoders.
///
/// Typed recursive decoding uses a conservative limit on normal worker stacks;
/// 32-level normal-stack regressions provide headroom for the public limit of 16.
/// Boxing alone does not bound recursive polling. Native constructors must respect
/// this contract; codec checks do not bound program-generated structures.
pub const MAX_SCALAR_DEPTH: usize = 16;

mod scalar;
pub use scalar::*;

mod op;
pub use op::*;

mod tcref;
pub use tcref::*;

#[cfg(test)]
#[path = "../tests/unit/stack_tests.rs"]
mod stack_tests;

#[cfg(test)]
#[path = "../tests/unit/tests.rs"]
mod tests;
