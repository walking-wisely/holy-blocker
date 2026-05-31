use super::common;

pub fn lowercase(text: &str) -> String {
    text.to_lowercase()
}

pub fn leet_compact(text: &str) -> String {
    common::compact(text).chars().map(normalize_leet_char).collect()
}

fn normalize_leet_char(ch: char) -> char {
    match ch {
        '0' => 'o',
        '1' => 'i',
        '3' => 'e',
        '4' => 'a',
        '5' => 's',
        '7' => 't',
        _ => ch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowercase_case_folds_english_text() {
        assert_eq!(lowercase("Hello WORLD"), "hello world");
    }

    #[test]
    fn leet_compact_removes_separators_and_substitutes_common_ascii_leet() {
        assert_eq!(leet_compact("p-0-r-n-h-u-b"), "pornhub");
        assert_eq!(leet_compact("4dult c0nt3nt"), "adultcontent");
    }

    #[test]
    fn leet_compact_uses_a_conservative_single_character_map() {
        assert_eq!(leet_compact("1337 5p34k"), "ieetspeak");
    }
}
