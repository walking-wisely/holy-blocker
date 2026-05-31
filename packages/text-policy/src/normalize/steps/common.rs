use unicode_normalization::UnicodeNormalization;

pub fn normalize_nfkc(text: &str) -> String {
    text.nfkc().collect()
}

pub fn trim_text(text: &str) -> String {
    text.trim().to_string()
}

pub fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn separator_tokens(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .collect()
}

pub fn compact(text: &str) -> String {
    text.chars().filter(|ch| ch.is_alphanumeric()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_nfkc_converts_compatibility_characters() {
        assert_eq!(normalize_nfkc("Ｆｕｌｌｗｉｄｔｈ"), "Fullwidth");
        assert_eq!(normalize_nfkc("① ² ﬃ"), "1 2 ffi");
    }

    #[test]
    fn normalize_nfkc_composes_canonical_sequences() {
        assert_eq!(normalize_nfkc("e\u{301}"), "é");
    }

    #[test]
    fn trim_text_removes_outer_whitespace_only() {
        assert_eq!(trim_text("  Keep\t   Spacing  "), "Keep\t   Spacing");
    }

    #[test]
    fn collapse_whitespace_replaces_whitespace_runs_with_single_spaces() {
        assert_eq!(collapse_whitespace("first\t \nsecond"), "first second");
    }

    #[test]
    fn separator_tokens_splits_on_non_alphanumeric_boundaries() {
        assert_eq!(
            separator_tokens("adult-content_site/path.name"),
            vec!["adult", "content", "site", "path", "name"]
        );
    }

    #[test]
    fn separator_tokens_keeps_letters_and_digits_inside_tokens() {
        assert_eq!(
            separator_tokens("COVID-19 B2B Привіт-світе"),
            vec!["COVID", "19", "B2B", "Привіт", "світе"]
        );
    }

    #[test]
    fn compact_removes_separators_without_dropping_letters_or_digits() {
        assert_eq!(compact("p-o_r.n/h u b"), "pornhub");
        assert_eq!(compact("Привіт-світе 123"), "Привітсвіте123");
    }
}
