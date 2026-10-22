#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Tone {
    #[default]
    None,
    Acute,
    Grave,
    Hook,
    Tilde,
    Dot,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum VowelMark {
    #[default]
    None,
    Breve,
    Circumflex,
    Horn,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RawSyllable {
    keys: String,
    tone: Tone,
    mark: VowelMark,
}

impl RawSyllable {
    pub fn new(keys: impl Into<String>) -> Self {
        Self {
            keys: keys.into(),
            tone: Tone::None,
            mark: VowelMark::None,
        }
    }

    pub fn keys(&self) -> &str {
        &self.keys
    }

    pub fn tone(&self) -> Tone {
        self.tone
    }

    pub fn mark(&self) -> VowelMark {
        self.mark
    }

    pub fn with_tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }

    pub fn with_mark(mut self, mark: VowelMark) -> Self {
        self.mark = mark;
        self
    }
}
