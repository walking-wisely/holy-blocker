//! Keyword rule kind: a confirmed token blocks names that carry it.
//!
//! Heuristic by design (`docs/decisions/custom-app-blocking.md`, decision 12): it misses a service
//! whose domains do not carry its name and can block an unrelated site that does. Both ends of that
//! trade are bounded here by a length minimum and a stoplist, and the person confirms each token
//! before it reaches this module.

pub const MIN_TOKEN_LEN: usize = 5;

/// Sorted and lowercase; `normalize_token` binary-searches it. Changes only through the decision
/// record.
pub const STOPLIST: &[&str] = &[
    "browser",
    "calculator",
    "calendar",
    "camera",
    "chat",
    "clock",
    "cloud",
    "drive",
    "email",
    "files",
    "games",
    "line",
    "mail",
    "maps",
    "media",
    "meta",
    "mobile",
    "music",
    "news",
    "notes",
    "phone",
    "photo",
    "photos",
    "search",
    "settings",
    "shop",
    "store",
    "translate",
    "video",
    "videos",
    "weather",
    "world",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeywordMatch {
    /// The registrable domain's own label equals the token (`instagram.com`).
    Exact,
    /// Some label left of the public suffix contains the token (`cdninstagram.com`).
    Contains,
}

/// The form of `raw` a rule can use: ASCII letters and digits, lowercased. `None` when that form is
/// shorter than [`MIN_TOKEN_LEN`] or on the [`STOPLIST`].
pub fn normalize_token(raw: &str) -> Option<String> {
    let token: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    let usable = token.len() >= MIN_TOKEN_LEN && STOPLIST.binary_search(&token.as_str()).is_err();
    usable.then_some(token)
}

#[derive(Debug, Clone, Default)]
pub struct KeywordRules {
    tokens: Vec<String>,
}

impl KeywordRules {
    /// Tokens that fail [`normalize_token`] are dropped.
    pub fn new<S: AsRef<str>>(tokens: &[S]) -> Self {
        let mut usable: Vec<String> = tokens
            .iter()
            .filter_map(|t| normalize_token(t.as_ref()))
            .collect();
        usable.sort();
        usable.dedup();
        KeywordRules { tokens: usable }
    }

    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    /// `normalized` must be the output of `domain_normalize::normalize`. A name that is itself a
    /// public suffix has no label to match.
    pub fn matches(&self, normalized: &str) -> Option<KeywordMatch> {
        if self.tokens.is_empty() {
            return None;
        }
        let registrable = psl::domain_str(normalized)?;
        let (own_label, suffix) = registrable.split_once('.')?;
        let below_suffix = normalized.strip_suffix(suffix)?.strip_suffix('.')?;

        if self.tokens.iter().any(|t| t == own_label) {
            return Some(KeywordMatch::Exact);
        }
        let contained = below_suffix
            .split('.')
            .any(|label| self.tokens.iter().any(|t| label.contains(t.as_str())));
        contained.then_some(KeywordMatch::Contains)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(tokens: &[&str]) -> KeywordRules {
        KeywordRules::new(tokens)
    }

    #[test]
    fn stoplist_is_sorted_lowercase_and_unique() {
        let mut sorted = STOPLIST.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted, STOPLIST);
        assert!(
            STOPLIST
                .iter()
                .all(|w| w.chars().all(|c| c.is_ascii_lowercase()))
        );
    }

    #[test]
    fn stoplist_holds_the_words_the_decision_names() {
        for word in ["line", "meta", "mail", "maps"] {
            assert!(STOPLIST.contains(&word), "{word}");
        }
    }

    #[test]
    fn token_is_reduced_to_lowercase_letters_and_digits() {
        assert_eq!(
            normalize_token("Insta-Gram 2!").as_deref(),
            Some("instagram2")
        );
        assert_eq!(
            normalize_token("com.instagram").as_deref(),
            Some("cominstagram")
        );
    }

    #[test]
    fn token_under_the_minimum_is_refused() {
        assert_eq!(normalize_token("tiktk"), Some("tiktk".to_string()));
        assert_eq!(normalize_token("tikt"), None);
        assert_eq!(normalize_token("a b c d"), None);
    }

    #[test]
    fn stoplisted_token_is_refused_in_any_spelling() {
        assert_eq!(normalize_token("Photos"), None);
        assert_eq!(normalize_token("P-h-o-t-o-s"), None);
        assert_eq!(normalize_token("browser"), None);
    }

    #[test]
    fn non_ascii_letters_do_not_count_toward_the_minimum() {
        assert_eq!(normalize_token("телеграм"), None);
        assert_eq!(
            normalize_token("телеграмtelegram").as_deref(),
            Some("telegram")
        );
    }

    #[test]
    fn registrable_label_equal_to_a_token_is_an_exact_match() {
        let r = rules(&["instagram"]);
        for name in [
            "instagram.com",
            "www.instagram.com",
            "a.b.instagram.com",
            "instagram.co.uk",
        ] {
            assert_eq!(r.matches(name), Some(KeywordMatch::Exact), "{name}");
        }
    }

    #[test]
    fn a_label_containing_a_token_is_a_contains_match() {
        let r = rules(&["instagram"]);
        for name in [
            "cdninstagram.com",
            "scontent.cdninstagram.com",
            "instagram-help.net",
            "my.instagramfans.org",
        ] {
            assert_eq!(r.matches(name), Some(KeywordMatch::Contains), "{name}");
        }
    }

    #[test]
    fn a_subdomain_label_is_checked_at_every_depth() {
        let r = rules(&["instagram"]);
        assert_eq!(
            r.matches("instagram.example.com"),
            Some(KeywordMatch::Contains)
        );
        assert_eq!(
            r.matches("x.cdninstagram.example.com"),
            Some(KeywordMatch::Contains)
        );
    }

    #[test]
    fn exact_wins_over_contains_across_tokens() {
        let r = rules(&["instagram", "cdninstagram"]);
        assert_eq!(r.matches("cdninstagram.com"), Some(KeywordMatch::Exact));
    }

    #[test]
    fn the_public_suffix_is_never_matched() {
        let r = rules(&["blogspot"]);
        assert_eq!(r.matches("example.blogspot.com"), None);
        let r = rules(&["travel"]);
        assert_eq!(r.matches("example.travel"), None);
        assert_eq!(r.matches("example.co.uk"), None);
    }

    #[test]
    fn a_name_that_is_itself_a_suffix_does_not_match() {
        let r = rules(&["instagram"]);
        assert_eq!(r.matches("com"), None);
        assert_eq!(r.matches("co.uk"), None);
    }

    #[test]
    fn unrelated_and_partial_names_do_not_match() {
        let r = rules(&["instagram"]);
        for name in [
            "example.com",
            "insta.com",
            "gram.org",
            "instagra.com",
            "instagram",
        ] {
            assert_eq!(r.matches(name), None, "{name}");
        }
    }

    #[test]
    fn no_tokens_match_nothing() {
        let r = KeywordRules::default();
        assert!(r.is_empty());
        assert_eq!(r.matches("instagram.com"), None);
    }

    #[test]
    fn ineligible_tokens_are_dropped_and_do_not_match() {
        let r = rules(&["meta", "photos", "ab"]);
        assert!(r.is_empty());
        assert_eq!(r.matches("meta.com"), None);
        assert_eq!(r.matches("photos.example.com"), None);
    }

    #[test]
    fn duplicate_and_differently_spelled_tokens_collapse() {
        let r = rules(&["Instagram", "insta-gram", "INSTAGRAM"]);
        assert_eq!(r.tokens, vec!["instagram".to_string()]);
    }

    #[test]
    fn punycode_labels_match_only_on_their_ascii_text() {
        let r = rules(&["instagram"]);
        assert_eq!(r.matches("xn--80ak6aa92e.com"), None);
        assert_eq!(r.matches("instagram.xn--p1ai"), Some(KeywordMatch::Exact));
    }
}
