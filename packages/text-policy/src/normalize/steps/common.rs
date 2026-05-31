pub fn trim_text(text: &str) -> String {
    text.trim().to_string()
}

pub fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_text_removes_outer_whitespace_only() {
        assert_eq!(trim_text("  Keep\t   Spacing  "), "Keep\t   Spacing");
    }

    #[test]
    fn collapse_whitespace_replaces_whitespace_runs_with_single_spaces() {
        assert_eq!(collapse_whitespace("first\t \nsecond"), "first second");
    }
}
