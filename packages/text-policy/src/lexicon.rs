mod automaton;
mod builder;
mod matcher;
mod types;
mod url;

pub use builder::{LexiconBuildError, LexiconBuilder};
pub use matcher::LexiconMatcher;
pub use types::{
    Category, CompiledTerm, Dictionary, DictionaryTerm, LexiconMatch, MatchMode, MatchSpan,
    Severity,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::normalize::Language;

    fn matcher() -> LexiconMatcher {
        LexiconBuilder::new(Language::English)
            .add_dictionary(Dictionary::new(
                "core",
                vec![
                    DictionaryTerm::new(
                        "adult platform",
                        Category::AdultPlatform,
                        Severity::High,
                        vec![MatchMode::ExactPhrase, MatchMode::TokenSequence],
                    ),
                    DictionaryTerm::new(
                        "pornhub",
                        Category::AdultPlatform,
                        Severity::High,
                        vec![MatchMode::Compact],
                    ),
                    DictionaryTerm::new(
                        "medical anatomy",
                        Category::MedicalException,
                        Severity::Low,
                        vec![MatchMode::ExactPhrase],
                    ),
                ],
            ))
            .add_dictionary(Dictionary::new(
                "domains",
                vec![DictionaryTerm::new(
                    "adult platform",
                    Category::CommercialAdult,
                    Severity::High,
                    vec![MatchMode::UrlTokenSequence],
                )],
            ))
            .build()
            .expect("valid test dictionary")
    }

    #[test]
    fn loads_multiple_dictionaries_and_preserves_term_metadata() {
        let matcher = matcher();

        assert_eq!(matcher.terms().len(), 4);
        assert_eq!(matcher.terms()[0].dictionary, "core");
        assert_eq!(matcher.terms()[3].dictionary, "domains");
        assert_eq!(matcher.terms()[3].category, Category::CommercialAdult);
    }

    #[test]
    fn exact_phrase_matches_only_on_token_boundaries() {
        let matcher = matcher();

        let matches = matcher.matches("This mentions an adult platform today.");
        assert!(
            matches
                .iter()
                .any(|candidate| candidate.mode == MatchMode::ExactPhrase)
        );

        let embedded = matcher.matches("This mentions an xadult platforming string.");
        assert!(
            embedded
                .iter()
                .all(|candidate| candidate.mode != MatchMode::ExactPhrase)
        );
    }

    #[test]
    fn token_sequence_matches_across_separators() {
        let matcher = matcher();

        let matches = matcher.matches("This says adult-platform in a title.");

        assert!(
            matches
                .iter()
                .any(|candidate| candidate.mode == MatchMode::TokenSequence
                    && candidate.span == MatchSpan::Tokens { start: 2, end: 4 })
        );
    }

    #[test]
    fn compact_mode_matches_obvious_spacing_and_leet_obfuscation() {
        let matcher = matcher();

        let matches = matcher.matches("Open p-0.r_n h u b");

        assert!(
            matches
                .iter()
                .any(|candidate| candidate.mode == MatchMode::Compact
                    && candidate.phrase == "pornhub")
        );
    }

    #[test]
    fn url_token_sequence_matches_domain_and_slug_tokens() {
        let matcher = matcher();

        let matches = matcher.matches("Visit https://example.com/adult-platform/watch.");

        assert!(
            matches
                .iter()
                .any(|candidate| candidate.dictionary == "domains"
                    && candidate.mode == MatchMode::UrlTokenSequence
                    && candidate.span == MatchSpan::Tokens { start: 2, end: 4 })
        );
    }

    #[test]
    fn surfaces_compile_to_the_same_term_metadata() {
        let matcher = LexiconBuilder::new(Language::English)
            .add_dictionary(Dictionary::new(
                "aliases",
                vec![
                    DictionaryTerm::new(
                        "primary phrase",
                        Category::EducationException,
                        Severity::Low,
                        vec![MatchMode::ExactPhrase],
                    )
                    .with_surfaces(["alternate phrase"]),
                ],
            ))
            .build()
            .expect("valid dictionary");

        let matches = matcher.matches("This has an alternate phrase.");

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].phrase, "primary phrase");
        assert_eq!(matches[0].surface, "alternate phrase");
        assert_eq!(matches[0].category, Category::EducationException);
    }

    #[test]
    fn phrase_occurring_twice_produces_two_matches() {
        let matcher = matcher();
        let matches = matcher.matches("adult platform here, adult platform there.");
        let exact_count = matches
            .iter()
            .filter(|m| m.mode == MatchMode::ExactPhrase && m.phrase == "adult platform")
            .count();
        assert_eq!(exact_count, 2);
    }

    #[test]
    fn matches_sorted_by_term_id_then_mode_then_position() {
        let matcher = matcher();
        let matches = matcher.matches("adult platform");
        for window in matches.windows(2) {
            let a = &window[0];
            let b = &window[1];
            let a_key = (a.term_id, a.mode as u8, span_start(a.span), span_end(a.span));
            let b_key = (b.term_id, b.mode as u8, span_start(b.span), span_end(b.span));
            assert!(
                a_key <= b_key,
                "matches out of sort order: {:?} before {:?}",
                a_key,
                b_key
            );
        }
    }

    #[test]
    fn identical_spans_from_two_surfaces_deduplicated() {
        // Two surfaces that normalise to the same byte string and produce the
        // same span would create a duplicate entry; the dedup pass removes it.
        let matcher = LexiconBuilder::new(Language::English)
            .add_dictionary(Dictionary::new(
                "dedup",
                vec![DictionaryTerm::new(
                    "alpha",
                    Category::AnatomyAmbiguous,
                    Severity::Low,
                    vec![MatchMode::ExactPhrase],
                )
                // "ALPHA" normalises to "alpha" → same compiled surface
                .with_surfaces(["ALPHA"])],
            ))
            .build()
            .expect("valid");

        let matches = matcher.matches("the alpha word");
        let exact: Vec<_> = matches
            .iter()
            .filter(|m| m.mode == MatchMode::ExactPhrase)
            .collect();
        assert_eq!(exact.len(), 1, "duplicate surface match should be deduped");
    }

    #[test]
    fn lexicon_match_fields_all_populated() {
        let matcher = matcher();
        let matches = matcher.matches("adult platform");
        let exact = matches
            .iter()
            .find(|m| m.mode == MatchMode::ExactPhrase)
            .expect("should have exact match");

        assert_eq!(exact.dictionary, "core");
        assert_eq!(exact.phrase, "adult platform");
        assert_eq!(exact.category, Category::AdultPlatform);
        assert_eq!(exact.severity, Severity::High);
        assert!(matches!(exact.span, MatchSpan::Bytes { .. }));
    }

    #[test]
    fn token_match_fields_carry_token_span() {
        let matcher = matcher();
        let matches = matcher.matches("adult-platform");
        let token_match = matches
            .iter()
            .find(|m| m.mode == MatchMode::TokenSequence)
            .expect("should have token match");

        assert!(matches!(token_match.span, MatchSpan::Tokens { .. }));
    }

    #[test]
    fn empty_string_produces_no_matches() {
        assert!(matcher().matches("").is_empty());
    }

    #[test]
    fn unrelated_text_produces_no_matches() {
        assert!(matcher().matches("the quick brown fox").is_empty());
    }

    fn span_start(span: MatchSpan) -> usize {
        match span {
            MatchSpan::Bytes { start, .. } | MatchSpan::Tokens { start, .. } => start,
        }
    }

    fn span_end(span: MatchSpan) -> usize {
        match span {
            MatchSpan::Bytes { end, .. } | MatchSpan::Tokens { end, .. } => end,
        }
    }
}
