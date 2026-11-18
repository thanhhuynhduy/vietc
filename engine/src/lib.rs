// SPDX-License-Identifier: MIT
mod bamboo;
mod bigram;
mod engine;
mod engine_helpers;
mod english;
pub mod event;
mod input_method;
mod ring_buffer;
pub mod spelling;
mod state;
pub mod telex;

#[cfg(test)]
mod tests;

pub use engine::Engine;
pub use engine::EngineEvent;
pub use event::{Command, EventStore, InputEvent};
pub use input_method::InputMethod;
pub use state::{
    BoundaryState, CaseShape, CompositionState, PhysicalEvent, Replacement, ReplacementReason,
    SpellingValidity, TransformIdentity, TransformKind, TransitionKind, UndoPolicy,
};
