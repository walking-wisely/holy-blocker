pub(super) fn url_token_streams(text: &str) -> Vec<Vec<String>> {
    text.split_whitespace()
        .filter_map(|chunk| {
            let chunk = chunk.trim_matches(|ch: char| {
                !ch.is_alphanumeric() && ch != '.' && ch != '/' && ch != ':' && ch != '-'
            });

            if !looks_url_like(chunk) {
                return None;
            }

            let without_scheme = chunk
                .split_once("://")
                .map(|(_, rest)| rest)
                .unwrap_or(chunk);

            let tokens: Vec<String> = without_scheme
                .split(|ch: char| !ch.is_alphanumeric())
                .filter(|token| !token.is_empty())
                .map(|token| token.to_lowercase())
                .collect();

            (!tokens.is_empty()).then_some(tokens)
        })
        .collect()
}

fn looks_url_like(text: &str) -> bool {
    text.contains("://")
        || text.contains('/')
        || text
            .split('.')
            .filter(|part| part.chars().any(char::is_alphanumeric))
            .count()
            >= 2
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(text: &str) -> Vec<String> {
        url_token_streams(text).into_iter().flatten().collect()
    }

    // --- looks_url_like ---

    #[test]
    fn looks_url_like_true_for_scheme() {
        assert!(looks_url_like("http://localhost"));
        assert!(looks_url_like("ftp://files.example.com"));
    }

    #[test]
    fn looks_url_like_true_for_slash() {
        assert!(looks_url_like("/some/path"));
        assert!(looks_url_like("example.com/page"));
    }

    #[test]
    fn looks_url_like_true_for_two_dot_parts() {
        assert!(looks_url_like("example.com"));
        assert!(looks_url_like("sub.example.org"));
    }

    #[test]
    fn looks_url_like_false_for_plain_word() {
        assert!(!looks_url_like("notaurl"));
        assert!(!looks_url_like("word"));
    }

    #[test]
    fn looks_url_like_false_for_single_dot_part() {
        assert!(!looks_url_like("single."));
        assert!(!looks_url_like(".hidden"));
    }

    // --- url_token_streams ---

    #[test]
    fn plain_domain_splits_into_labels() {
        let streams = url_token_streams("example.com");
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0], vec!["example", "com"]);
    }

    #[test]
    fn https_url_strips_scheme_and_splits_path() {
        let tokens = flat("https://example.com/adult-platform");
        assert!(tokens.contains(&"example".to_string()));
        assert!(tokens.contains(&"com".to_string()));
        assert!(tokens.contains(&"adult".to_string()));
        assert!(tokens.contains(&"platform".to_string()));
    }

    #[test]
    fn plain_word_produces_no_stream() {
        assert!(url_token_streams("notaurl").is_empty());
    }

    #[test]
    fn empty_string_produces_no_streams() {
        assert!(url_token_streams("").is_empty());
    }

    #[test]
    fn path_with_leading_slash_is_url_like() {
        let tokens = flat("/some/path");
        assert!(tokens.contains(&"some".to_string()));
        assert!(tokens.contains(&"path".to_string()));
    }

    #[test]
    fn localhost_with_scheme_produces_one_token() {
        let streams = url_token_streams("http://localhost");
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0], vec!["localhost"]);
    }

    #[test]
    fn ip_address_is_url_like() {
        assert!(!url_token_streams("192.168.1.1").is_empty());
    }

    #[test]
    fn trailing_punctuation_stripped_before_tokenising() {
        let tokens = flat("https://example.com/path.");
        assert!(tokens.contains(&"path".to_string()));
        assert!(!tokens.contains(&"path.".to_string()));
    }

    #[test]
    fn multiple_urls_produce_multiple_streams() {
        let streams = url_token_streams("visit example.com or other.org today");
        assert_eq!(streams.len(), 2);
    }

    #[test]
    fn plain_words_between_urls_are_not_included() {
        let streams = url_token_streams("hello world example.com notaurl another.site");
        assert_eq!(streams.len(), 2);
    }

    #[test]
    fn tokens_are_lowercased() {
        let tokens = flat("EXAMPLE.COM");
        assert!(tokens.contains(&"example".to_string()));
        assert!(tokens.contains(&"com".to_string()));
        assert!(!tokens.contains(&"EXAMPLE".to_string()));
    }

    #[test]
    fn empty_segments_from_double_slashes_filtered_out() {
        let tokens = flat("example.com//double//slash");
        assert!(tokens.iter().all(|t| !t.is_empty()));
    }

    #[test]
    fn query_string_separators_produce_tokens() {
        let tokens = flat("example.com/search?q=foo&bar=baz");
        assert!(tokens.contains(&"search".to_string()));
        assert!(tokens.contains(&"foo".to_string()));
        assert!(tokens.contains(&"bar".to_string()));
        assert!(tokens.contains(&"baz".to_string()));
    }

    #[test]
    fn url_with_fragment_splits_cleanly() {
        let tokens = flat("example.com/page#section");
        assert!(tokens.contains(&"page".to_string()));
        assert!(tokens.contains(&"section".to_string()));
    }

    #[test]
    fn stream_for_url_contains_no_scheme_tokens() {
        let tokens = flat("https://example.com");
        assert!(!tokens.contains(&"https".to_string()));
    }
}
