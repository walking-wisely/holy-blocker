//! Module 1 parser for hagezi's NSFW blocklist
//! (<https://github.com/hagezi/dns-blocklists>) — plain domain-per-line, one domain (optionally
//! `*.`-prefixed) per line, comments starting with `#`. Parsing itself is
//! [`super::parse_document`] — this file only supplies the source's identity and category.

use crate::types::{Category, SourceId};

use super::{LineFormat, ParseOutput, parse_document};

/// hagezi's NSFW list is a single fixed category, same as StevenBlack's `porn` extension — see
/// `stevenblack::parse`'s doc comment.
pub fn parse(bytes: &[u8]) -> ParseOutput {
    parse_document(
        bytes,
        SourceId::Hagezi,
        Category::Adult,
        LineFormat::PlainDomain,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ScopeHint;

    #[test]
    fn parses_a_realistic_fragment_including_a_wildcard() {
        let out = parse(
            b"# hagezi nsfw\n\
              example-adult-site.test\n\
              *.cdn-adult-site.test\n\
              \n",
        );

        assert_eq!(out.entries.len(), 2);
        assert_eq!(out.entries[0].domain, "example-adult-site.test");
        assert_eq!(out.entries[0].scope_hint, ScopeHint::None);
        assert_eq!(out.entries[1].domain, "cdn-adult-site.test");
        assert_eq!(out.entries[1].scope_hint, ScopeHint::Apex);
        assert!(out.entries.iter().all(|e| e.source == SourceId::Hagezi));
        assert!(out.entries.iter().all(|e| e.category == Category::Adult));
        assert_eq!(out.report.wildcard_count, 1);
    }
}
