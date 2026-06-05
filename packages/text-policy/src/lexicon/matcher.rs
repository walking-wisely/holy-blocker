use crate::normalize::{Language, NormalizationPipeline, NormalizedText};

use super::automaton::{
    ByteAhoCorasick, ByteMatchCandidate, TokenAhoCorasick, TokenMatchCandidate,
};
use super::types::{CompiledTerm, LexiconMatch, MatchSpan};
use super::url::url_token_streams;

#[derive(Clone, Debug)]
pub struct LexiconMatcher {
    language: Language,
    terms: Vec<CompiledTerm>,
    exact_matcher: ByteAhoCorasick,
    compact_matcher: ByteAhoCorasick,
    token_matcher: TokenAhoCorasick,
    url_token_matcher: TokenAhoCorasick,
}

impl LexiconMatcher {
    pub(super) fn new(
        language: Language,
        terms: Vec<CompiledTerm>,
        exact_matcher: ByteAhoCorasick,
        compact_matcher: ByteAhoCorasick,
        token_matcher: TokenAhoCorasick,
        url_token_matcher: TokenAhoCorasick,
    ) -> Self {
        Self {
            language,
            terms,
            exact_matcher,
            compact_matcher,
            token_matcher,
            url_token_matcher,
        }
    }

    pub fn language(&self) -> Language {
        self.language
    }

    pub fn terms(&self) -> &[CompiledTerm] {
        &self.terms
    }

    pub fn matches(&self, text: &str) -> Vec<LexiconMatch> {
        let views = NormalizationPipeline::for_language(self.language).normalize_views(text);
        self.matches_normalized(&views)
    }

    pub fn matches_normalized(&self, views: &NormalizedText) -> Vec<LexiconMatch> {
        let mut matches = Vec::new();

        for candidate in self.exact_matcher.find_iter(&views.normalized) {
            matches.push(self.match_from_byte_candidate(candidate));
        }

        for candidate in self.compact_matcher.find_iter(&views.leet_compact) {
            matches.push(self.match_from_byte_candidate(candidate));
        }

        for candidate in self.token_matcher.find_iter(&views.separator_tokens) {
            matches.push(self.match_from_token_candidate(candidate));
        }

        for tokens in url_token_streams(&views.raw) {
            for candidate in self.url_token_matcher.find_iter(&tokens) {
                matches.push(self.match_from_token_candidate(candidate));
            }
        }

        matches.sort_by_key(|candidate| {
            (
                candidate.term_id,
                candidate.mode as u8,
                span_start(candidate.span),
                span_end(candidate.span),
            )
        });
        matches.dedup_by(|left, right| {
            left.term_id == right.term_id
                && left.mode == right.mode
                && left.surface == right.surface
                && left.span == right.span
        });
        matches
    }

    fn match_from_byte_candidate(&self, candidate: ByteMatchCandidate) -> LexiconMatch {
        let pattern = &candidate.pattern;
        let term = &self.terms[pattern.term_id];

        LexiconMatch {
            term_id: pattern.term_id,
            dictionary: term.dictionary.clone(),
            phrase: term.phrase.clone(),
            category: term.category,
            severity: term.severity,
            mode: pattern.mode,
            surface: pattern.surface.clone(),
            span: MatchSpan::Bytes {
                start: candidate.start,
                end: candidate.end,
            },
        }
    }

    fn match_from_token_candidate(&self, candidate: TokenMatchCandidate) -> LexiconMatch {
        let pattern = &candidate.pattern;
        let term = &self.terms[pattern.term_id];

        LexiconMatch {
            term_id: pattern.term_id,
            dictionary: term.dictionary.clone(),
            phrase: term.phrase.clone(),
            category: term.category,
            severity: term.severity,
            mode: pattern.mode,
            surface: pattern.surface.clone(),
            span: MatchSpan::Tokens {
                start: candidate.start,
                end: candidate.end,
            },
        }
    }
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
