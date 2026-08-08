//! Module 1 parser for Université Toulouse 1 Capitole's category blocklists
//! (<https://dsi.ut-capitole.fr/blacklists/>) — a directory per category, each holding a
//! `domains` file (one domain per line, `*.`-prefixed wildcards included) and a sibling `urls`
//! file. Only `domains` is consumed; `urls` is out of scope for a domain-keyed artifact — see
//! [`count_urls_entries`].
//!
//! Unlike StevenBlack and hagezi, UT1 hands out category **per directory** rather than the whole
//! source carrying one fixed category, so [`parse_domains`] takes `category` as a parameter —
//! the pipeline binary (module 7, unbuilt) is expected to fetch each category directory this
//! project consumes (e.g. `adult/domains`, `gambling/domains`, `dating/domains`) under its own
//! [`super::SourceConfig`] and call this once per directory.

use crate::types::{Category, SourceId};

use super::{LineFormat, ParseOutput, parse_document};

/// Parses one UT1 category directory's `domains` file. `category` is supplied by the caller
/// (which directory this file came from), not inferred from the bytes — UT1 gives no in-band
/// signal of which category a `domains` file belongs to.
pub fn parse_domains(bytes: &[u8], category: Category) -> ParseOutput {
    parse_document(bytes, SourceId::Ut1, category, LineFormat::PlainDomain)
}

/// Counts how many entries a UT1 category's sibling `urls` file would have contributed, without
/// extracting or emitting any of them. `urls` entries are a documented coverage limitation, not a
/// silent drop, per the plan ("the count of skipped `urls` entries is reported in the build's
/// metrics so the size of the gap is visible") — the pipeline binary reports this number
/// alongside the build; nothing downstream of module 1 ever sees a `urls` entry itself.
pub fn count_urls_entries(bytes: &[u8]) -> u64 {
    let text = String::from_utf8_lossy(bytes);
    text.lines()
        .filter_map(|line| {
            line.split_once('#')
                .map_or(Some(line), |(before, _)| Some(before))
        })
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .count() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ScopeHint;

    #[test]
    fn parses_a_realistic_gambling_domains_fragment() {
        let out = parse_domains(
            b"# UT1 gambling\n\
              example-betting-site.test\n\
              *.affiliate-betting.test\n",
            Category::Gambling,
        );

        assert_eq!(out.entries.len(), 2);
        assert!(out.entries.iter().all(|e| e.source == SourceId::Ut1));
        assert!(out.entries.iter().all(|e| e.category == Category::Gambling));
        assert_eq!(out.entries[1].domain, "affiliate-betting.test");
        assert_eq!(out.entries[1].scope_hint, ScopeHint::Apex);
    }

    #[test]
    fn parse_domains_tags_the_category_the_caller_names_regardless_of_directory_content() {
        let out = parse_domains(b"example-dating-site.test\n", Category::Dating);
        assert_eq!(out.entries[0].category, Category::Dating);
    }

    #[test]
    fn count_urls_entries_counts_non_comment_non_blank_lines() {
        let n = count_urls_entries(
            b"# UT1 gambling urls\n\
              https://example-betting-site.test/path\n\
              \n\
              https://example-betting-site.test/other # note\n",
        );
        assert_eq!(n, 2);
    }

    #[test]
    fn count_urls_entries_is_zero_for_an_empty_file() {
        assert_eq!(count_urls_entries(b""), 0);
        assert_eq!(count_urls_entries(b"# only a comment\n"), 0);
    }
}
