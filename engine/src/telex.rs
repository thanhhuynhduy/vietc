// SPDX-License-Identifier: MIT
mod mark;
mod model;
mod parser;
mod raw;
mod render;
mod tone;

pub use model::{Boundary, Glide, ParseStatus, ParsedSyllable};
pub use parser::parse;
pub use raw::{RawSyllable, Tone, VowelMark};
pub use render::{replacement_diff, resolve_and_render, resolve_with_undo, ReplacementDiff};

#[cfg(test)]
mod property_tests;
#[cfg(test)]
mod tests;
