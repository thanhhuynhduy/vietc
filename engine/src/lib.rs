// SPDX-License-Identifier: MIT
mod autocorrect;
mod bamboo;
mod bigram;
mod engine;
mod engine_helpers;
mod english;
pub mod event;
mod filename_restore;
mod input_method;
mod lm;
pub mod ngram_lm;
mod ring_buffer;
pub mod spelling;
pub mod stat_lm;
mod state;
mod symspell;
pub mod telex;
mod user_dict;

#[cfg(test)]
mod tests;

pub use engine::Engine;
pub use engine::EngineEvent;
pub use engine_helpers::is_flush_char;
pub use event::{Command, EventStore, InputEvent};
pub use input_method::InputMethod;
pub use state::{
    BoundaryState, CaseShape, CompositionState, PhysicalEvent, Replacement, ReplacementReason,
    SpellingValidity, TransformIdentity, TransformKind, TransitionKind, UndoPolicy,
};
pub use symspell::warm_up as warm_up_symspell;
pub use user_dict::UserDict;
