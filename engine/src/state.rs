#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryState {
    Composing,
    Committed,
    Invalidated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhysicalEvent {
    Printable(char),
    Backspace,
    WordBoundary(char),
    Delete,
    Navigate,
    SelectionChanged,
    ExternalEdit,
    Paste(String),
    FocusChanged,
    ReplacementFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseShape {
    Lower,
    Upper,
    Mixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpellingValidity {
    Unknown,
    Valid,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndoPolicy {
    AppendTrigger,
    ConsumeTrigger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformKind {
    Tone(char),
    Mark(&'static str),
    StrokeDd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionKind {
    Apply,
    ReplaceActive,
    UndoActive,
    Edit,
    Invalidate,
    Commit,
    Recover,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransformIdentity {
    pub kind: TransformKind,
    pub trigger: char,
    pub generation: u64,
    pub undo_policy: UndoPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompositionState {
    pub raw_keys: String,
    pub visible: String,
    pub active_transform: Option<TransformIdentity>,
    pub case_shape: CaseShape,
    pub validity: SpellingValidity,
    pub boundary: BoundaryState,
    pub generation: u64,
    pub last_transition: Option<TransitionKind>,
}
