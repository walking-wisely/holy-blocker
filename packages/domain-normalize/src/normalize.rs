//! The domain comparison key.
//!
//! `packages/domain-blocklist` normalizes every source entry with this function at build time;
//! `packages/net-shield` normalizes every query with the same function at lookup time. Neither
//! side may reimplement it — see the "one implementation, no drift" rule in
//! `docs/components/domain-blocklist/plan.md`, Module 0.
//!
//! `normalize()` is a comparison key only. It never changes what a rule covers — in particular
//! it does **not** strip a leading `www.` label, on purpose (see the plan doc for why that would
//! silently conflate a rule naming `www.example.com` with one naming `example.com`).

use idna::uts46::{AsciiDenyList, DnsLength, Hyphens, Uts46};

/// RFC 1035 §2.3.4 — a label is restricted to 63 octets or less.
const MAX_LABEL_LEN: usize = 63;

/// RFC 1035 §2.3.4 — a domain name is restricted to 255 octets in wire form (a 1-byte length
/// prefix per label plus a trailing 0-length root label); the conventional printable-form limit,
/// with dots instead of length-prefix bytes and no root label, is 253. Length is validated here
/// *after* A-label (punycode) expansion, per RFC 5891 §4.4 — punycode can only grow a label, so a
/// pre-conversion check would pass strings that are too long once encoded.
const MAX_NAME_LEN: usize = 253;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum NormalizeError {
    #[error("empty domain")]
    Empty,
    /// UTS #46 processing (mapping, case folding, or RFC 3492 punycode encoding, per RFC 5892
    /// code-point eligibility) rejected the input.
    #[error("domain failed UTS #46 / IDNA processing")]
    InvalidIdna,
    #[error("label exceeds 63 bytes after A-label conversion")]
    LabelTooLong,
    #[error("domain exceeds 253 bytes after A-label conversion")]
    NameTooLong,
}

/// Produces the comparison key for `domain`.
///
/// Order (not optional — see the plan doc):
/// 1. Strip a single trailing dot.
/// 2. UTS #46 mapping and case folding.
/// 3. RFC 3492 punycode encoding of any U-labels (steps 2 and 3 happen together, in that order,
///    inside one UTS #46 pass — running case folding after punycode would encode two spellings of
///    the same name differently).
/// 4. Validate label (63-byte) and name (253-byte) length limits, measured after A-label
///    expansion since punycode only ever grows a label.
pub fn normalize(domain: &str) -> Result<String, NormalizeError> {
    let trimmed = domain.strip_suffix('.').unwrap_or(domain);
    if trimmed.is_empty() {
        return Err(NormalizeError::Empty);
    }

    // AsciiDenyList::URL, matching UseSTD3ASCIIRules=false plus the WHATWG URL Standard's own
    // forbidden-domain-code-point check (idna's own doc comment names this as exactly what the
    // URL Standard's domain-to-ASCII step applies). A real browser resolves `beStrict = false`
    // when parsing a URL's host — the URL Standard says so explicitly, "due to web compatibility"
    // — so a strict LDH-only deny list (AsciiDenyList::STD3) rejects domains a browser will
    // actually navigate to. The load-bearing case: an underscore label is not DNS-preferred syntax
    // (RFC 952/1123) but is legal on the wire (RFC 1035 §2.3.1 labels are arbitrary octets) and
    // resolves in practice (`_dmarc.google.com` is a real, ubiquitous example) — rejecting it here
    // does not narrow this crate's guaranteed-safe scope, it silently drops the domain from the
    // merged set entirely, so it can never become a rule at all. AsciiDenyList::URL still denies
    // glyphless/control characters and URL-structural punctuation (`%#/:<>?@[\]^|`, which is also
    // how an IPv6 literal's `:`/`[`/`]` stay rejected here, same as before). Hyphens::Allow:
    // positional hyphen restrictions (leading/trailing/3rd-4th-position) reject real-world names
    // idna's own `domain_to_ascii_strict` docs warn about (YouTube CDN nodes, some GitHub Pages
    // hosts); length is validated separately below, so DnsLength::Ignore here.
    let cow = Uts46::new()
        .to_ascii(
            trimmed.as_bytes(),
            AsciiDenyList::URL,
            Hyphens::Allow,
            DnsLength::Ignore,
        )
        .map_err(|_| NormalizeError::InvalidIdna)?;
    let ascii = cow.into_owned();

    if ascii.is_empty() {
        return Err(NormalizeError::Empty);
    }

    for label in ascii.split('.') {
        if label.is_empty() {
            return Err(NormalizeError::InvalidIdna);
        }
        if label.len() > MAX_LABEL_LEN {
            return Err(NormalizeError::LabelTooLong);
        }
    }
    if ascii.len() > MAX_NAME_LEN {
        return Err(NormalizeError::NameTooLong);
    }

    Ok(ascii)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowercases_plain_ascii() {
        assert_eq!(normalize("Example.COM").unwrap(), "example.com");
    }

    #[test]
    fn strips_single_trailing_dot() {
        assert_eq!(normalize("example.com.").unwrap(), "example.com");
    }

    #[test]
    fn rejects_empty_domain() {
        assert_eq!(normalize(""), Err(NormalizeError::Empty));
        assert_eq!(normalize("."), Err(NormalizeError::Empty));
    }

    #[test]
    fn rejects_empty_label() {
        assert_eq!(normalize("foo..com"), Err(NormalizeError::InvalidIdna));
        assert_eq!(normalize(".example.com"), Err(NormalizeError::InvalidIdna));
    }

    #[test]
    fn converts_u_label_to_a_label() {
        // müller.de -> xn--mller-kva.de (RFC 3492 worked example domain, ASCII 'u'-umlaut form).
        assert_eq!(normalize("müller.de").unwrap(), "xn--mller-kva.de");
    }

    #[test]
    fn case_folds_before_punycode_so_two_spellings_collide() {
        // Two case-variant spellings of the same Unicode label must normalize identically —
        // mapping/case-folding has to happen before punycode encoding, or they would not.
        let lower = normalize("müller.de").unwrap();
        let upper = normalize("MÜLLER.DE").unwrap();
        assert_eq!(lower, upper);
    }

    #[test]
    fn does_not_strip_www() {
        assert_eq!(normalize("www.example.com").unwrap(), "www.example.com");
        assert_ne!(
            normalize("www.example.com").unwrap(),
            normalize("example.com").unwrap()
        );
    }

    #[test]
    fn rejects_label_over_63_bytes_after_conversion() {
        let label = "a".repeat(64);
        let domain = format!("{label}.com");
        assert_eq!(normalize(&domain), Err(NormalizeError::LabelTooLong));
    }

    #[test]
    fn accepts_label_at_exactly_63_bytes() {
        let label = "a".repeat(63);
        let domain = format!("{label}.com");
        assert!(normalize(&domain).is_ok());
    }

    #[test]
    fn rejects_name_over_253_bytes_after_conversion() {
        // 4 labels of 63 'a's plus 3 dots = 255 bytes, over the 253 limit.
        let label = "a".repeat(63);
        let domain = format!("{label}.{label}.{label}.{label}");
        assert_eq!(normalize(&domain), Err(NormalizeError::NameTooLong));
    }

    #[test]
    fn rejects_glyphless_and_url_structural_characters() {
        // Space is glyphless (deny_glyphless=true in AsciiDenyList::URL) — still rejected.
        assert_eq!(normalize("exa mple.com"), Err(NormalizeError::InvalidIdna));
        // '/' is one of the URL-structural characters AsciiDenyList::URL denies explicitly.
        assert_eq!(normalize("exa/mple.com"), Err(NormalizeError::InvalidIdna));
    }

    #[test]
    fn accepts_underscore_a_real_browser_resolves() {
        // Underscore is DNS-wire-legal (RFC 1035 §2.3.1) and a real browser's URL parser accepts
        // it (WHATWG URL Standard, beStrict=false) — AsciiDenyList::STD3 previously rejected this
        // and silently dropped a browser-reachable domain from the merged set entirely.
        assert_eq!(normalize("under_score.com").unwrap(), "under_score.com");
        assert_eq!(normalize("_dmarc.google.com").unwrap(), "_dmarc.google.com");
    }

    #[test]
    fn still_rejects_ipv6_literal_colons_and_brackets() {
        // AsciiDenyList::URL's own doc comment: "this deny list rejects IPv6 addresses" — ':'/
        // '['/']' are all in its explicit URL-structural deny list, same as under STD3 before.
        assert_eq!(normalize("[::1]"), Err(NormalizeError::InvalidIdna));
    }

    #[test]
    fn idempotent_on_an_already_normalized_domain() {
        let once = normalize("example.com").unwrap();
        let twice = normalize(&once).unwrap();
        assert_eq!(once, twice);
    }
}
