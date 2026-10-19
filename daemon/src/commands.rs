use vietc_engine::Replacement;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputCommand {
    Type(String),
    Backspace(usize),
    Replace(Replacement),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionOutcome {
    Applied {
        visible_revision: u64,
    },
    Stale {
        expected_visible_revision: u64,
        actual_visible_revision: u64,
    },
    Failed,
}
