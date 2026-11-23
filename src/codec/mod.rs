//! IR codecs own typed structure; destream codecs own framing and leaf parsing.
mod decode;
mod encode;

pub use decode::{decode_scalar, decode_scalar_entry};
