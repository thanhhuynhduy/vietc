use super::raw::{RawSyllable, Tone, VowelMark};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Boundary {
    EndOfInput,
    Whitespace,
    Punctuation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Glide {
    Qu,
    Gi,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParseStatus {
    Complete,
    Incomplete,
    Invalid,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedSyllable {
    raw: RawSyllable,
    onset: String,
    nucleus: String,
    coda: String,
    glide: Option<Glide>,
    boundary: Boundary,
    status: ParseStatus,
}

impl ParsedSyllable {
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        raw: RawSyllable,
        onset: impl Into<String>,
        nucleus: impl Into<String>,
        coda: impl Into<String>,
        glide: Option<Glide>,
        boundary: Boundary,
        status: ParseStatus,
    ) -> Result<Self, &'static str> {
        let onset = onset.into();
        let nucleus = nucleus.into();
        let coda = coda.into();
        if format!("{onset}{nucleus}{coda}") != raw.keys() {
            return Err("parts must be contiguous and reconstruct the raw keys");
        }
        Ok(Self {
            raw,
            onset,
            nucleus,
            coda,
            glide,
            boundary,
            status,
        })
    }

    pub fn raw(&self) -> &RawSyllable {
        &self.raw
    }

    pub fn onset(&self) -> &str {
        &self.onset
    }

    pub fn nucleus(&self) -> &str {
        &self.nucleus
    }

    pub fn coda(&self) -> &str {
        &self.coda
    }

    pub fn glide(&self) -> Option<Glide> {
        self.glide
    }

    pub fn boundary(&self) -> Boundary {
        self.boundary
    }

    pub fn status(&self) -> ParseStatus {
        self.status
    }

    pub fn tone(&self) -> Tone {
        self.raw.tone()
    }

    pub fn mark(&self) -> VowelMark {
        self.raw.mark()
    }
}
