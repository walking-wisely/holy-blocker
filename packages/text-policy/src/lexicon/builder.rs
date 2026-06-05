use std::error::Error;
use std::fmt;

use crate::normalize::{Language, NormalizationPipeline, separator_tokens};

use super::automaton::{ByteAhoCorasick, BytePattern, TokenAhoCorasick, TokenPattern};
use super::matcher::LexiconMatcher;
use super::types::{CompiledTerm, Dictionary, DictionaryTerm, MatchMode};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LexiconBuildError {
    EmptyDictionaryName,
    EmptyPhrase { dictionary: String },
    EmptyMatchModes { dictionary: String, phrase: String },
    EmptySurface { dictionary: String, phrase: String },
}

impl fmt::Display for LexiconBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyDictionaryName => write!(f, "dictionary name cannot be empty"),
            Self::EmptyPhrase { dictionary } => {
                write!(f, "dictionary '{dictionary}' contains an empty phrase")
            }
            Self::EmptyMatchModes { dictionary, phrase } => write!(
                f,
                "dictionary '{dictionary}' term '{phrase}' has no match modes"
            ),
            Self::EmptySurface { dictionary, phrase } => write!(
                f,
                "dictionary '{dictionary}' term '{phrase}' contains an empty surface"
            ),
        }
    }
}

impl Error for LexiconBuildError {}

#[derive(Clone, Debug)]
pub struct LexiconBuilder {
    language: Language,
    dictionaries: Vec<Dictionary>,
}

impl LexiconBuilder {
    pub fn new(language: Language) -> Self {
        Self {
            language,
            dictionaries: Vec::new(),
        }
    }

    pub fn add_dictionary(mut self, dictionary: Dictionary) -> Self {
        self.dictionaries.push(dictionary);
        self
    }

    pub fn build(self) -> Result<LexiconMatcher, LexiconBuildError> {
        let pipeline = NormalizationPipeline::for_language(self.language);
        let mut terms = Vec::new();
        let mut exact_patterns = Vec::new();
        let mut compact_patterns = Vec::new();
        let mut token_patterns = Vec::new();
        let mut url_token_patterns = Vec::new();

        for dictionary in self.dictionaries {
            if dictionary.name.trim().is_empty() {
                return Err(LexiconBuildError::EmptyDictionaryName);
            }

            for term in dictionary.terms {
                if term.phrase.trim().is_empty() {
                    return Err(LexiconBuildError::EmptyPhrase {
                        dictionary: dictionary.name,
                    });
                }

                if term.match_modes.is_empty() {
                    return Err(LexiconBuildError::EmptyMatchModes {
                        dictionary: dictionary.name,
                        phrase: term.phrase,
                    });
                }

                let term_id = terms.len();
                let raw_surfaces = surfaces_for_term(&term, &dictionary.name)?;
                let mut compiled_surfaces = Vec::new();

                for raw_surface in raw_surfaces {
                    let surface_views = pipeline.normalize_views(&raw_surface);
                    let normalized_surface = surface_views.normalized.clone();
                    compiled_surfaces.push(normalized_surface.clone());

                    for mode in dedup_modes(&term.match_modes) {
                        match mode {
                            MatchMode::ExactPhrase => {
                                if !normalized_surface.is_empty() {
                                    exact_patterns.push(BytePattern {
                                        term_id,
                                        mode,
                                        surface: normalized_surface.clone(),
                                        bytes: normalized_surface.as_bytes().to_vec(),
                                        requires_token_boundary: true,
                                    });
                                }
                            }
                            MatchMode::Compact => {
                                if !surface_views.leet_compact.is_empty() {
                                    compact_patterns.push(BytePattern {
                                        term_id,
                                        mode,
                                        surface: surface_views.leet_compact.clone(),
                                        bytes: surface_views.leet_compact.as_bytes().to_vec(),
                                        requires_token_boundary: false,
                                    });
                                }
                            }
                            MatchMode::TokenSequence => {
                                let tokens = surface_views.separator_tokens.clone();
                                if !tokens.is_empty() {
                                    token_patterns.push(TokenPattern {
                                        term_id,
                                        mode,
                                        surface: normalized_surface.clone(),
                                        tokens,
                                    });
                                }
                            }
                            MatchMode::UrlTokenSequence => {
                                let tokens = separator_tokens(&normalized_surface);
                                if !tokens.is_empty() {
                                    url_token_patterns.push(TokenPattern {
                                        term_id,
                                        mode,
                                        surface: normalized_surface.clone(),
                                        tokens,
                                    });
                                }
                            }
                        }
                    }
                }

                terms.push(CompiledTerm {
                    dictionary: dictionary.name.clone(),
                    phrase: term.phrase,
                    category: term.category,
                    severity: term.severity,
                    match_modes: dedup_modes(&term.match_modes),
                    surfaces: dedup_strings(compiled_surfaces),
                });
            }
        }

        Ok(LexiconMatcher::new(
            self.language,
            terms,
            ByteAhoCorasick::new(exact_patterns),
            ByteAhoCorasick::new(compact_patterns),
            TokenAhoCorasick::new(token_patterns),
            TokenAhoCorasick::new(url_token_patterns),
        ))
    }
}

fn surfaces_for_term(
    term: &DictionaryTerm,
    dictionary: &str,
) -> Result<Vec<String>, LexiconBuildError> {
    let mut surfaces = vec![term.phrase.clone()];
    surfaces.extend(term.surfaces.clone());

    for surface in &surfaces {
        if surface.trim().is_empty() {
            return Err(LexiconBuildError::EmptySurface {
                dictionary: dictionary.to_string(),
                phrase: term.phrase.clone(),
            });
        }
    }

    Ok(dedup_strings(surfaces))
}

fn dedup_modes(modes: &[MatchMode]) -> Vec<MatchMode> {
    let mut deduped = Vec::new();
    for mode in modes {
        if !deduped.contains(mode) {
            deduped.push(*mode);
        }
    }
    deduped
}

fn dedup_strings(strings: Vec<String>) -> Vec<String> {
    let mut deduped = Vec::new();
    for string in strings {
        if !deduped.contains(&string) {
            deduped.push(string);
        }
    }
    deduped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexicon::types::{Category, MatchMode, Severity};
    use crate::normalize::Language;

    fn simple_term(phrase: &str) -> DictionaryTerm {
        DictionaryTerm::new(
            phrase,
            Category::ExplicitAct,
            Severity::High,
            vec![MatchMode::ExactPhrase],
        )
    }

    fn simple_dict(name: &str, phrase: &str) -> Dictionary {
        Dictionary::new(name, vec![simple_term(phrase)])
    }

    fn builder() -> LexiconBuilder {
        LexiconBuilder::new(Language::English)
    }

    // ── dedup_modes ────────────────────────────────────────────────────────

    #[test]
    fn dedup_modes_removes_consecutive_duplicate() {
        let result = dedup_modes(&[MatchMode::ExactPhrase, MatchMode::ExactPhrase]);
        assert_eq!(result, vec![MatchMode::ExactPhrase]);
    }

    #[test]
    fn dedup_modes_preserves_distinct_modes() {
        let result = dedup_modes(&[MatchMode::ExactPhrase, MatchMode::TokenSequence]);
        assert_eq!(result, vec![MatchMode::ExactPhrase, MatchMode::TokenSequence]);
    }

    #[test]
    fn dedup_modes_empty_input() {
        assert!(dedup_modes(&[]).is_empty());
    }

    #[test]
    fn dedup_modes_preserves_order_of_first_occurrence() {
        let result = dedup_modes(&[
            MatchMode::TokenSequence,
            MatchMode::ExactPhrase,
            MatchMode::TokenSequence,
        ]);
        assert_eq!(result, vec![MatchMode::TokenSequence, MatchMode::ExactPhrase]);
    }

    // ── dedup_strings ─────────────────────────────────────────────────────

    #[test]
    fn dedup_strings_removes_duplicate() {
        let result = dedup_strings(vec!["a".into(), "b".into(), "a".into()]);
        assert_eq!(result, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn dedup_strings_empty_input() {
        assert!(dedup_strings(vec![]).is_empty());
    }

    #[test]
    fn dedup_strings_all_distinct_unchanged() {
        let result = dedup_strings(vec!["x".into(), "y".into(), "z".into()]);
        assert_eq!(result.len(), 3);
    }

    // ── LexiconBuildError variants ─────────────────────────────────────────

    #[test]
    fn error_empty_dictionary_name() {
        let err = builder().add_dictionary(simple_dict("", "phrase")).build();
        assert!(matches!(err, Err(LexiconBuildError::EmptyDictionaryName)));
    }

    #[test]
    fn error_whitespace_only_dictionary_name() {
        let err = builder().add_dictionary(simple_dict("   ", "phrase")).build();
        assert!(matches!(err, Err(LexiconBuildError::EmptyDictionaryName)));
    }

    #[test]
    fn error_empty_phrase() {
        let err = builder()
            .add_dictionary(Dictionary::new("d", vec![simple_term("")]))
            .build();
        assert!(matches!(err, Err(LexiconBuildError::EmptyPhrase { dictionary }) if dictionary == "d"));
    }

    #[test]
    fn error_whitespace_only_phrase() {
        let err = builder()
            .add_dictionary(Dictionary::new("d", vec![simple_term("   ")]))
            .build();
        assert!(matches!(err, Err(LexiconBuildError::EmptyPhrase { dictionary }) if dictionary == "d"));
    }

    #[test]
    fn error_empty_match_modes() {
        let term = DictionaryTerm::new("phrase", Category::ExplicitAct, Severity::High, vec![]);
        let err = builder()
            .add_dictionary(Dictionary::new("d", vec![term]))
            .build();
        assert!(matches!(
            err,
            Err(LexiconBuildError::EmptyMatchModes { dictionary, phrase })
                if dictionary == "d" && phrase == "phrase"
        ));
    }

    #[test]
    fn error_empty_surface_string() {
        let term = simple_term("phrase").with_surfaces([""]);
        let err = builder()
            .add_dictionary(Dictionary::new("d", vec![term]))
            .build();
        assert!(matches!(
            err,
            Err(LexiconBuildError::EmptySurface { dictionary, phrase })
                if dictionary == "d" && phrase == "phrase"
        ));
    }

    #[test]
    fn error_whitespace_only_surface() {
        let term = simple_term("phrase").with_surfaces(["   "]);
        let err = builder()
            .add_dictionary(Dictionary::new("d", vec![term]))
            .build();
        assert!(matches!(
            err,
            Err(LexiconBuildError::EmptySurface { dictionary, phrase })
                if dictionary == "d" && phrase == "phrase"
        ));
    }

    #[test]
    fn error_in_second_dictionary_still_reported() {
        let err = builder()
            .add_dictionary(simple_dict("valid", "good phrase"))
            .add_dictionary(simple_dict("", "phrase"))
            .build();
        assert!(matches!(err, Err(LexiconBuildError::EmptyDictionaryName)));
    }

    #[test]
    fn error_display_includes_dictionary_name() {
        let err = LexiconBuildError::EmptyPhrase { dictionary: "mydict".into() };
        assert!(err.to_string().contains("mydict"));
    }

    #[test]
    fn error_display_includes_phrase() {
        let err = LexiconBuildError::EmptyMatchModes {
            dictionary: "d".into(),
            phrase: "myphrase".into(),
        };
        let s = err.to_string();
        assert!(s.contains("d"));
        assert!(s.contains("myphrase"));
    }

    #[test]
    fn error_display_empty_surface_includes_phrase() {
        let err = LexiconBuildError::EmptySurface {
            dictionary: "d".into(),
            phrase: "phrase".into(),
        };
        assert!(err.to_string().contains("phrase"));
    }

    // ── build success ──────────────────────────────────────────────────────

    #[test]
    fn build_empty_builder_succeeds_with_no_terms() {
        let matcher = builder().build().unwrap();
        assert!(matcher.terms().is_empty());
    }

    #[test]
    fn build_single_term_records_correct_metadata() {
        let matcher = builder()
            .add_dictionary(Dictionary::new(
                "dict",
                vec![DictionaryTerm::new(
                    "alpha beta",
                    Category::Nudity,
                    Severity::Medium,
                    vec![MatchMode::ExactPhrase],
                )],
            ))
            .build()
            .unwrap();
        assert_eq!(matcher.terms().len(), 1);
        assert_eq!(matcher.terms()[0].dictionary, "dict");
        assert_eq!(matcher.terms()[0].phrase, "alpha beta");
        assert_eq!(matcher.terms()[0].category, Category::Nudity);
        assert_eq!(matcher.terms()[0].severity, Severity::Medium);
    }

    #[test]
    fn build_duplicate_match_modes_deduplicated_in_compiled_term() {
        let term = DictionaryTerm::new(
            "phrase",
            Category::ExplicitAct,
            Severity::High,
            vec![MatchMode::ExactPhrase, MatchMode::ExactPhrase, MatchMode::TokenSequence],
        );
        let matcher = builder()
            .add_dictionary(Dictionary::new("d", vec![term]))
            .build()
            .unwrap();
        assert_eq!(matcher.terms()[0].match_modes.len(), 2);
    }

    #[test]
    fn build_duplicate_surface_deduplicated() {
        // phrase == surface after normalisation → only one surface compiled
        let term = simple_term("phrase").with_surfaces(["phrase"]);
        let matcher = builder()
            .add_dictionary(Dictionary::new("d", vec![term]))
            .build()
            .unwrap();
        assert_eq!(matcher.terms()[0].surfaces.len(), 1);
    }

    #[test]
    fn build_multiple_dictionaries_assigns_sequential_term_ids() {
        let matcher = builder()
            .add_dictionary(Dictionary::new(
                "first",
                vec![simple_term("one"), simple_term("two")],
            ))
            .add_dictionary(Dictionary::new("second", vec![simple_term("three")]))
            .build()
            .unwrap();
        assert_eq!(matcher.terms().len(), 3);
        assert_eq!(matcher.terms()[0].dictionary, "first");
        assert_eq!(matcher.terms()[1].dictionary, "first");
        assert_eq!(matcher.terms()[2].dictionary, "second");
    }

    #[test]
    fn build_surface_alias_compiles_same_category_and_severity() {
        let term = DictionaryTerm::new(
            "primary phrase",
            Category::MedicalException,
            Severity::Low,
            vec![MatchMode::ExactPhrase],
        )
        .with_surfaces(["alternate phrase"]);
        let matcher = builder()
            .add_dictionary(Dictionary::new("d", vec![term]))
            .build()
            .unwrap();
        let compiled = &matcher.terms()[0];
        assert_eq!(compiled.category, Category::MedicalException);
        assert_eq!(compiled.severity, Severity::Low);
        assert_eq!(compiled.surfaces.len(), 2);
    }
}
