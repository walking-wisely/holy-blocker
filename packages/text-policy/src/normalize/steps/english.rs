pub fn lowercase(text: &str) -> String {
    text.to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowercase_case_folds_english_text() {
        assert_eq!(lowercase("Hello WORLD"), "hello world");
    }
}
