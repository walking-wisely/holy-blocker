use std::collections::{HashMap, VecDeque};

use super::types::MatchMode;

#[derive(Clone, Debug)]
pub(super) struct BytePattern {
    pub(super) term_id: usize,
    pub(super) mode: MatchMode,
    pub(super) surface: String,
    pub(super) bytes: Vec<u8>,
    pub(super) requires_token_boundary: bool,
}

#[derive(Clone, Debug)]
struct ByteNode {
    next: HashMap<u8, usize>,
    fail: usize,
    outputs: Vec<usize>,
}

#[derive(Clone, Debug)]
pub(super) struct ByteAhoCorasick {
    patterns: Vec<BytePattern>,
    nodes: Vec<ByteNode>,
}

impl ByteAhoCorasick {
    pub(super) fn new(patterns: Vec<BytePattern>) -> Self {
        let mut nodes = vec![ByteNode {
            next: HashMap::new(),
            fail: 0,
            outputs: Vec::new(),
        }];

        for (pattern_index, pattern) in patterns.iter().enumerate() {
            let mut node_index = 0;
            for byte in &pattern.bytes {
                let next_index = match nodes[node_index].next.get(byte) {
                    Some(next_index) => *next_index,
                    None => {
                        nodes.push(ByteNode {
                            next: HashMap::new(),
                            fail: 0,
                            outputs: Vec::new(),
                        });
                        let next_index = nodes.len() - 1;
                        nodes[node_index].next.insert(*byte, next_index);
                        next_index
                    }
                };
                node_index = next_index;
            }
            nodes[node_index].outputs.push(pattern_index);
        }

        let mut queue = VecDeque::new();
        let root_children: Vec<usize> = nodes[0].next.values().copied().collect();
        for child in root_children {
            queue.push_back(child);
        }

        while let Some(node_index) = queue.pop_front() {
            let transitions: Vec<(u8, usize)> = nodes[node_index]
                .next
                .iter()
                .map(|(byte, next_index)| (*byte, *next_index))
                .collect();

            for (byte, child_index) in transitions {
                queue.push_back(child_index);

                let mut fallback = nodes[node_index].fail;
                while fallback != 0 && !nodes[fallback].next.contains_key(&byte) {
                    fallback = nodes[fallback].fail;
                }

                let fail = nodes[fallback].next.get(&byte).copied().unwrap_or(0);
                nodes[child_index].fail = fail;

                let inherited_outputs = nodes[fail].outputs.clone();
                nodes[child_index].outputs.extend(inherited_outputs);
            }
        }

        Self { patterns, nodes }
    }

    pub(super) fn find_iter(&self, text: &str) -> Vec<ByteMatchCandidate> {
        let mut matches = Vec::new();
        let mut node_index = 0;
        let bytes = text.as_bytes();

        for (position, byte) in bytes.iter().enumerate() {
            while node_index != 0 && !self.nodes[node_index].next.contains_key(byte) {
                node_index = self.nodes[node_index].fail;
            }

            node_index = self.nodes[node_index].next.get(byte).copied().unwrap_or(0);

            for pattern_index in &self.nodes[node_index].outputs {
                let pattern = &self.patterns[*pattern_index];
                let end = position + 1;
                let start = end - pattern.bytes.len();

                if !pattern.requires_token_boundary || has_token_boundaries(text, start, end) {
                    matches.push(ByteMatchCandidate {
                        pattern: pattern.clone(),
                        start,
                        end,
                    });
                }
            }
        }

        matches
    }
}

#[derive(Clone, Debug)]
pub(super) struct ByteMatchCandidate {
    pub(super) pattern: BytePattern,
    pub(super) start: usize,
    pub(super) end: usize,
}

#[derive(Clone, Debug)]
pub(super) struct TokenPattern {
    pub(super) term_id: usize,
    pub(super) mode: MatchMode,
    pub(super) surface: String,
    pub(super) tokens: Vec<String>,
}

#[derive(Clone, Debug)]
struct TokenNode {
    next: HashMap<String, usize>,
    fail: usize,
    outputs: Vec<usize>,
}

#[derive(Clone, Debug)]
pub(super) struct TokenAhoCorasick {
    patterns: Vec<TokenPattern>,
    nodes: Vec<TokenNode>,
}

impl TokenAhoCorasick {
    pub(super) fn new(patterns: Vec<TokenPattern>) -> Self {
        let mut nodes = vec![TokenNode {
            next: HashMap::new(),
            fail: 0,
            outputs: Vec::new(),
        }];

        for (pattern_index, pattern) in patterns.iter().enumerate() {
            let mut node_index = 0;
            for token in &pattern.tokens {
                let next_index = match nodes[node_index].next.get(token) {
                    Some(next_index) => *next_index,
                    None => {
                        nodes.push(TokenNode {
                            next: HashMap::new(),
                            fail: 0,
                            outputs: Vec::new(),
                        });
                        let next_index = nodes.len() - 1;
                        nodes[node_index].next.insert(token.clone(), next_index);
                        next_index
                    }
                };
                node_index = next_index;
            }
            nodes[node_index].outputs.push(pattern_index);
        }

        let mut queue = VecDeque::new();
        let root_children: Vec<usize> = nodes[0].next.values().copied().collect();
        for child in root_children {
            queue.push_back(child);
        }

        while let Some(node_index) = queue.pop_front() {
            let transitions: Vec<(String, usize)> = nodes[node_index]
                .next
                .iter()
                .map(|(token, next_index)| (token.clone(), *next_index))
                .collect();

            for (token, child_index) in transitions {
                queue.push_back(child_index);

                let mut fallback = nodes[node_index].fail;
                while fallback != 0 && !nodes[fallback].next.contains_key(&token) {
                    fallback = nodes[fallback].fail;
                }

                let fail = nodes[fallback].next.get(&token).copied().unwrap_or(0);
                nodes[child_index].fail = fail;

                let inherited_outputs = nodes[fail].outputs.clone();
                nodes[child_index].outputs.extend(inherited_outputs);
            }
        }

        Self { patterns, nodes }
    }

    pub(super) fn find_iter(&self, tokens: &[String]) -> Vec<TokenMatchCandidate> {
        let mut matches = Vec::new();
        let mut node_index = 0;

        for (position, token) in tokens.iter().enumerate() {
            while node_index != 0 && !self.nodes[node_index].next.contains_key(token) {
                node_index = self.nodes[node_index].fail;
            }

            node_index = self.nodes[node_index].next.get(token).copied().unwrap_or(0);

            for pattern_index in &self.nodes[node_index].outputs {
                let pattern = &self.patterns[*pattern_index];
                let end = position + 1;
                let start = end - pattern.tokens.len();
                matches.push(TokenMatchCandidate {
                    pattern: pattern.clone(),
                    start,
                    end,
                });
            }
        }

        matches
    }
}

#[derive(Clone, Debug)]
pub(super) struct TokenMatchCandidate {
    pub(super) pattern: TokenPattern,
    pub(super) start: usize,
    pub(super) end: usize,
}

fn has_token_boundaries(text: &str, start: usize, end: usize) -> bool {
    let left_boundary = text[..start]
        .chars()
        .next_back()
        .is_none_or(|ch| !ch.is_alphanumeric());
    let right_boundary = text[end..]
        .chars()
        .next()
        .is_none_or(|ch| !ch.is_alphanumeric());

    left_boundary && right_boundary
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexicon::types::MatchMode;

    fn byte_pat(id: usize, text: &str, boundary: bool) -> BytePattern {
        BytePattern {
            term_id: id,
            mode: MatchMode::ExactPhrase,
            surface: text.to_string(),
            bytes: text.as_bytes().to_vec(),
            requires_token_boundary: boundary,
        }
    }

    fn token_pat(id: usize, tokens: &[&str]) -> TokenPattern {
        TokenPattern {
            term_id: id,
            mode: MatchMode::TokenSequence,
            surface: tokens.join(" "),
            tokens: tokens.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn term_ids(matches: &[ByteMatchCandidate]) -> Vec<usize> {
        matches.iter().map(|m| m.pattern.term_id).collect()
    }

    fn token_term_ids(matches: &[TokenMatchCandidate]) -> Vec<usize> {
        matches.iter().map(|m| m.pattern.term_id).collect()
    }

    // ── has_token_boundaries ───────────────────────────────────────────────

    #[test]
    fn boundary_both_ends_none_is_boundary() {
        assert!(has_token_boundaries("cat", 0, 3));
    }

    #[test]
    fn boundary_left_space_is_boundary() {
        assert!(has_token_boundaries(" cat", 1, 4));
    }

    #[test]
    fn boundary_right_space_is_boundary() {
        assert!(has_token_boundaries("cat ", 0, 3));
    }

    #[test]
    fn boundary_both_space_is_boundary() {
        assert!(has_token_boundaries(" cat ", 1, 4));
    }

    #[test]
    fn boundary_punctuation_on_both_sides_is_boundary() {
        assert!(has_token_boundaries(".cat.", 1, 4));
    }

    #[test]
    fn boundary_left_alphanumeric_is_not_boundary() {
        // "acat" — 'a' is alphanumeric, so left side fails
        assert!(!has_token_boundaries("acat", 1, 4));
    }

    #[test]
    fn boundary_right_alphanumeric_is_not_boundary() {
        // "cata" — 'a' after is alphanumeric
        assert!(!has_token_boundaries("cata", 0, 3));
    }

    #[test]
    fn boundary_unicode_alphanumeric_left_blocks() {
        // 'é' (U+00E9) is two bytes [0xC3, 0xA9]; "cat" starts at byte 2
        let s = "écat";
        let start = "é".len(); // 2
        assert!(!has_token_boundaries(s, start, s.len()));
    }

    #[test]
    fn boundary_unicode_alphanumeric_right_blocks() {
        let s = "caté";
        let end = "cat".len(); // 3; 'é' follows
        assert!(!has_token_boundaries(s, 0, end));
    }

    // ── ByteAhoCorasick ────────────────────────────────────────────────────

    #[test]
    fn byte_empty_patterns_finds_nothing() {
        let ac = ByteAhoCorasick::new(vec![]);
        assert!(ac.find_iter("hello world").is_empty());
    }

    #[test]
    fn byte_empty_input_finds_nothing() {
        let ac = ByteAhoCorasick::new(vec![byte_pat(0, "hello", false)]);
        assert!(ac.find_iter("").is_empty());
    }

    #[test]
    fn byte_no_match_returns_empty() {
        let ac = ByteAhoCorasick::new(vec![byte_pat(0, "xyz", false)]);
        assert!(ac.find_iter("hello world").is_empty());
    }

    #[test]
    fn byte_single_pattern_found_in_middle() {
        let ac = ByteAhoCorasick::new(vec![byte_pat(0, "hello", false)]);
        let m = ac.find_iter("say hello there");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].start, 4);
        assert_eq!(m[0].end, 9);
    }

    #[test]
    fn byte_pattern_at_start_of_string() {
        let ac = ByteAhoCorasick::new(vec![byte_pat(0, "hi", false)]);
        let m = ac.find_iter("hi there");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].start, 0);
    }

    #[test]
    fn byte_pattern_at_end_of_string() {
        let ac = ByteAhoCorasick::new(vec![byte_pat(0, "end", false)]);
        let m = ac.find_iter("at the end");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].end, 10);
    }

    #[test]
    fn byte_multiple_occurrences_all_reported() {
        let ac = ByteAhoCorasick::new(vec![byte_pat(0, "ab", false)]);
        let m = ac.find_iter("ababab");
        assert_eq!(m.len(), 3);
        assert_eq!(m[0].start, 0);
        assert_eq!(m[1].start, 2);
        assert_eq!(m[2].start, 4);
    }

    #[test]
    fn byte_two_distinct_patterns_both_found() {
        let ac = ByteAhoCorasick::new(vec![
            byte_pat(0, "alpha", false),
            byte_pat(1, "beta", false),
        ]);
        let m = ac.find_iter("alpha and beta");
        let ids = term_ids(&m);
        assert!(ids.contains(&0));
        assert!(ids.contains(&1));
    }

    #[test]
    fn byte_fail_links_fire_suffix_pattern() {
        // Classic AC: "he" fires while matching "she" because "he" is a suffix
        // of "she". Correct fail links cause both patterns to output on 'e'.
        let ac = ByteAhoCorasick::new(vec![
            byte_pat(0, "he", false),
            byte_pat(1, "she", false),
        ]);
        let m = ac.find_iter("she");
        let ids = term_ids(&m);
        assert!(ids.contains(&0), "he should fire via fail link inside she");
        assert!(ids.contains(&1), "she should fire directly");
    }

    #[test]
    fn byte_fail_links_classic_aho_corasick_set() {
        // "his", "he", "hers", "she" — text "ahishers" should produce all four
        let ac = ByteAhoCorasick::new(vec![
            byte_pat(0, "he", false),
            byte_pat(1, "she", false),
            byte_pat(2, "his", false),
            byte_pat(3, "hers", false),
        ]);
        let m = ac.find_iter("ahishers");
        let ids = term_ids(&m);
        assert!(ids.contains(&2), "his");
        assert!(ids.contains(&0), "he (suffix of hers via fail)");
        assert!(ids.contains(&3), "hers");
    }

    #[test]
    fn byte_shared_prefix_both_patterns_fire() {
        let ac = ByteAhoCorasick::new(vec![
            byte_pat(0, "abc", false),
            byte_pat(1, "abcd", false),
        ]);
        let ids = term_ids(&ac.find_iter("abcd"));
        assert!(ids.contains(&0));
        assert!(ids.contains(&1));
    }

    #[test]
    fn byte_boundary_true_blocks_embedded_match() {
        let ac = ByteAhoCorasick::new(vec![byte_pat(0, "cat", true)]);
        assert!(ac.find_iter("concatenate").is_empty());
        assert!(ac.find_iter("acat").is_empty());
        assert!(ac.find_iter("cata").is_empty());
    }

    #[test]
    fn byte_boundary_true_passes_when_spaced() {
        let ac = ByteAhoCorasick::new(vec![byte_pat(0, "cat", true)]);
        assert_eq!(ac.find_iter("the cat sat").len(), 1);
        assert_eq!(ac.find_iter("cat sat").len(), 1);  // start of string
        assert_eq!(ac.find_iter("the cat").len(), 1);  // end of string
    }

    #[test]
    fn byte_boundary_false_matches_embedded() {
        let ac = ByteAhoCorasick::new(vec![byte_pat(0, "cat", false)]);
        assert!(!ac.find_iter("concatenate").is_empty());
    }

    #[test]
    fn byte_match_candidate_carries_term_id_and_surface() {
        let ac = ByteAhoCorasick::new(vec![byte_pat(42, "hello", false)]);
        let m = ac.find_iter("hello");
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].pattern.term_id, 42);
        assert_eq!(m[0].pattern.surface, "hello");
    }

    // ── TokenAhoCorasick ───────────────────────────────────────────────────

    fn toks(s: &str) -> Vec<String> {
        s.split_whitespace().map(|t| t.to_string()).collect()
    }

    #[test]
    fn token_empty_patterns_finds_nothing() {
        let ac = TokenAhoCorasick::new(vec![]);
        assert!(ac.find_iter(&toks("hello world")).is_empty());
    }

    #[test]
    fn token_empty_input_finds_nothing() {
        let ac = TokenAhoCorasick::new(vec![token_pat(0, &["hello"])]);
        assert!(ac.find_iter(&[]).is_empty());
    }

    #[test]
    fn token_no_match_returns_empty() {
        let ac = TokenAhoCorasick::new(vec![token_pat(0, &["xyz"])]);
        assert!(ac.find_iter(&toks("hello world")).is_empty());
    }

    #[test]
    fn token_single_token_found_with_correct_span() {
        let ac = TokenAhoCorasick::new(vec![token_pat(0, &["hello"])]);
        let m = ac.find_iter(&toks("say hello there"));
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].start, 1);
        assert_eq!(m[0].end, 2);
    }

    #[test]
    fn token_multi_token_pattern_found() {
        let ac = TokenAhoCorasick::new(vec![token_pat(0, &["alpha", "beta"])]);
        let m = ac.find_iter(&toks("the alpha beta word"));
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].start, 1);
        assert_eq!(m[0].end, 3);
    }

    #[test]
    fn token_pattern_at_start_of_stream() {
        let ac = TokenAhoCorasick::new(vec![token_pat(0, &["alpha", "beta"])]);
        let m = ac.find_iter(&toks("alpha beta gamma"));
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].start, 0);
    }

    #[test]
    fn token_pattern_at_end_of_stream() {
        let ac = TokenAhoCorasick::new(vec![token_pat(0, &["alpha", "beta"])]);
        let m = ac.find_iter(&toks("gamma alpha beta"));
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].end, 3);
    }

    #[test]
    fn token_partial_match_does_not_fire() {
        let ac = TokenAhoCorasick::new(vec![token_pat(0, &["alpha", "beta"])]);
        assert!(ac.find_iter(&toks("alpha gamma beta")).is_empty());
    }

    #[test]
    fn token_two_patterns_both_fire() {
        let ac = TokenAhoCorasick::new(vec![
            token_pat(0, &["alpha", "beta"]),
            token_pat(1, &["beta", "gamma"]),
        ]);
        let ids = token_term_ids(&ac.find_iter(&toks("alpha beta gamma")));
        assert!(ids.contains(&0));
        assert!(ids.contains(&1));
    }

    #[test]
    fn token_fail_links_fire_suffix_pattern() {
        // "b c" is a suffix of "a b c"; reading [a, b, c] should fire both
        let ac = TokenAhoCorasick::new(vec![
            token_pat(0, &["b", "c"]),
            token_pat(1, &["a", "b", "c"]),
        ]);
        let ids = token_term_ids(&ac.find_iter(&toks("a b c")));
        assert!(ids.contains(&0), "b c should fire via fail link");
        assert!(ids.contains(&1), "a b c should fire directly");
    }

    #[test]
    fn token_multiple_occurrences_all_reported() {
        let ac = TokenAhoCorasick::new(vec![token_pat(0, &["x"])]);
        let tokens: Vec<String> = ["x", "y", "x"].iter().map(|s| s.to_string()).collect();
        let m = ac.find_iter(&tokens);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].start, 0);
        assert_eq!(m[1].start, 2);
    }

    #[test]
    fn token_match_candidate_carries_term_id_and_surface() {
        let ac = TokenAhoCorasick::new(vec![token_pat(7, &["foo", "bar"])]);
        let m = ac.find_iter(&toks("foo bar"));
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].pattern.term_id, 7);
        assert_eq!(m[0].pattern.surface, "foo bar");
    }
}
