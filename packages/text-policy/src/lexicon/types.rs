#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Category {
    ExplicitAct,
    AdultPlatform,
    CommercialAdult,
    Nudity,
    AnatomyAmbiguous,
    EducationException,
    MedicalException,
    SafetyException,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Severity {
    Low,
    Medium,
    High,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MatchMode {
    ExactPhrase,
    TokenSequence,
    Compact,
    UrlTokenSequence,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DictionaryTerm {
    pub phrase: String,
    pub category: Category,
    pub severity: Severity,
    pub match_modes: Vec<MatchMode>,
    pub surfaces: Vec<String>,
}

impl DictionaryTerm {
    pub fn new(
        phrase: impl Into<String>,
        category: Category,
        severity: Severity,
        match_modes: Vec<MatchMode>,
    ) -> Self {
        Self {
            phrase: phrase.into(),
            category,
            severity,
            match_modes,
            surfaces: Vec::new(),
        }
    }

    pub fn with_surfaces(mut self, surfaces: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.surfaces = surfaces.into_iter().map(Into::into).collect();
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Dictionary {
    pub name: String,
    pub terms: Vec<DictionaryTerm>,
}

impl Dictionary {
    pub fn new(name: impl Into<String>, terms: Vec<DictionaryTerm>) -> Self {
        Self {
            name: name.into(),
            terms,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompiledTerm {
    pub dictionary: String,
    pub phrase: String,
    pub category: Category,
    pub severity: Severity,
    pub match_modes: Vec<MatchMode>,
    pub surfaces: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatchSpan {
    Bytes { start: usize, end: usize },
    Tokens { start: usize, end: usize },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_is_ordered_low_medium_high() {
        assert!(Severity::Low < Severity::Medium);
        assert!(Severity::Medium < Severity::High);
        assert!(Severity::Low < Severity::High);
    }

    #[test]
    fn dictionary_term_with_surfaces_replaces_empty_vec() {
        let term = DictionaryTerm::new(
            "primary",
            Category::Nudity,
            Severity::Medium,
            vec![MatchMode::ExactPhrase],
        )
        .with_surfaces(["alt one", "alt two"]);
        assert_eq!(term.surfaces, vec!["alt one".to_string(), "alt two".to_string()]);
    }

}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LexiconMatch {
    pub term_id: usize,
    pub dictionary: String,
    pub phrase: String,
    pub category: Category,
    pub severity: Severity,
    pub mode: MatchMode,
    pub surface: String,
    pub span: MatchSpan,
}
