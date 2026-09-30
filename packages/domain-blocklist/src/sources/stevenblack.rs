//! Module 1 parser for the StevenBlack unified `hosts` list
//! (<https://github.com/StevenBlack/hosts>) — the `porn` extension specifically, which is what
//! this project's `SourceConfig` for [`SourceId::StevenBlack`] pins.
//!
//! Format: hosts-file syntax, `<address> <domain>` per line (`0.0.0.0 example.com` or
//! `127.0.0.1 example.com`); comments start with `#`. Parsing itself is
//! [`super::parse_document`] — this file only supplies the source's identity and category.

use crate::types::{Category, SourceId};

use super::{LineFormat, ParseOutput, parse_document};

/// StevenBlack's `porn` extension is a single fixed category — the source doesn't distinguish
/// further within it, per the plan's module 1 ("hagezi's NSFW list and StevenBlack's `porn`
/// extension are each a single fixed category").
pub fn parse(bytes: &[u8]) -> ParseOutput {
    parse_document(
        bytes,
        SourceId::StevenBlack,
        Category::Adult,
        LineFormat::HostsFile,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ScopeHint;

    #[test]
    fn parses_a_realistic_hosts_fragment() {
        let out = parse(
            b"# StevenBlack hosts porn extension\n\
              0.0.0.0 example-adult-site.test\n\
              127.0.0.1 another-adult-site.test\n\
              \n\
              0.0.0.0 0.0.0.0\n",
        );

        assert_eq!(out.entries.len(), 2);
        assert_eq!(out.entries[0].domain, "example-adult-site.test");
        assert_eq!(out.entries[0].source, SourceId::StevenBlack);
        assert_eq!(out.entries[0].category, Category::Adult);
        assert_eq!(out.entries[0].scope_hint, ScopeHint::None);
        assert_eq!(out.report.dropped_ip_literal, 1);
        assert_eq!(out.report.skipped_comment_or_blank, 2);
    }
}
