//! IR codecs own semantic continuations; destream codecs own framing and leaf parsing.
mod decode;
mod encode;

pub use decode::{decode_op_def_entry, decode_scalar};
