//! DNS message parsing and blocked-name synthesis.
//!
//! Only what a filter needs: read the question out of a query, and build a
//! refusal for a name the policy blocks. There is no resolver here and no
//! answer-section parsing — a permitted query is forwarded verbatim to a real
//! resolver and its reply is passed back untouched.
//!
//! Wire format: RFC 1035 §4.1 (message format), §4.1.1 (header), §4.1.2
//! (question), §4.1.4 (message compression). RCODE values: RFC 1035 §4.1.1 and
//! the IANA DNS RCODEs registry (RFC 6895 §2.3).

/// Well-known port for DNS over UDP and TCP.
/// IANA "Service Name and Transport Protocol Port Number Registry"; RFC 1035 §4.2.
pub const PORT_DNS: u16 = 53;

/// Fixed DNS header size: ID, flags, and the four section counts — RFC 1035 §4.1.1.
pub const HEADER_LEN: usize = 12;

/// A label length byte's top two bits are 11 for a compression pointer, and the
/// other two combinations are reserved — RFC 1035 §4.1.4.
const LABEL_KIND_MASK: u8 = 0b1100_0000;

/// Maximum length of a single label — RFC 1035 §2.3.4.
const MAX_LABEL_LEN: usize = 63;

/// Maximum length of an encoded domain name, including length bytes and the
/// root terminator — RFC 1035 §2.3.4.
const MAX_NAME_WIRE_LEN: usize = 255;

/// QR bit: 0 = query, 1 = response — RFC 1035 §4.1.1.
const FLAG_QR: u16 = 0x8000;
/// RD bit: recursion desired, echoed into the response — RFC 1035 §4.1.1.
const FLAG_RD: u16 = 0x0100;
/// RA bit: recursion available, set by a responder — RFC 1035 §4.1.1.
const FLAG_RA: u16 = 0x0080;
/// OPCODE occupies bits 14-11 of the flags word — RFC 1035 §4.1.1.
const OPCODE_MASK: u16 = 0x7800;
/// OPCODE 0 = standard query. Only standard queries carry a name we can filter.
const OPCODE_QUERY: u16 = 0x0000;
/// RCODE occupies the low four bits of the flags word — RFC 1035 §4.1.1.
const RCODE_MASK: u16 = 0x000f;
/// RCODE 3 = Name Error ("NXDOMAIN") — RFC 1035 §4.1.1.
const RCODE_NXDOMAIN: u16 = 3;

/// The single question carried by a standard query.
///
/// `name` is lowercased and carries no trailing root dot, because that is the
/// form [`crate::DomainFilter`] indexes. DNS names are case-insensitive for
/// matching purposes — RFC 1035 §2.3.3, restated in RFC 4343 §3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnsQuestion {
    pub id: u16,
    pub name: String,
    pub qtype: u16,
    pub qclass: u16,
}

/// Parse the question from a DNS **query**.
///
/// Returns `None` for anything this filter has no business rewriting: a
/// response, a non-standard opcode, a question count other than one, a name
/// using compression, or a truncated buffer. The caller forwards those
/// untouched rather than guessing.
pub fn parse_query(msg: &[u8]) -> Option<DnsQuestion> {
    parse_query_with_end(msg).map(|(question, _)| question)
}

/// As [`parse_query`], but also returns the offset one past the question — the
/// point a response appends its answer section at.
fn parse_query_with_end(msg: &[u8]) -> Option<(DnsQuestion, usize)> {
    if msg.len() < HEADER_LEN {
        return None;
    }

    let id = u16::from_be_bytes([msg[0], msg[1]]);
    let flags = u16::from_be_bytes([msg[2], msg[3]]);
    if flags & FLAG_QR != 0 {
        // A response, not a query. Never synthesised over.
        return None;
    }
    if flags & OPCODE_MASK != OPCODE_QUERY {
        return None;
    }

    let qdcount = u16::from_be_bytes([msg[4], msg[5]]);
    if qdcount != 1 {
        return None;
    }

    let (name, after_name) = read_name(msg, HEADER_LEN)?;

    // QTYPE(2) + QCLASS(2) follow the name — RFC 1035 §4.1.2.
    let end = after_name.checked_add(4)?;
    if msg.len() < end {
        return None;
    }
    let qtype = u16::from_be_bytes([msg[after_name], msg[after_name + 1]]);
    let qclass = u16::from_be_bytes([msg[after_name + 2], msg[after_name + 3]]);

    Some((
        DnsQuestion {
            id,
            name,
            qtype,
            qclass,
        },
        end,
    ))
}

/// Read a length-prefixed domain name starting at `pos`.
///
/// Compression pointers are rejected rather than followed: a query's QNAME is
/// the first name in the message, so there is nothing earlier for it to point
/// at, and accepting one here would be the only place a malformed input could
/// drive a loop.
fn read_name(msg: &[u8], mut pos: usize) -> Option<(String, usize)> {
    let mut labels: Vec<&str> = Vec::new();
    let mut wire_len = 0usize;

    loop {
        let len_byte = *msg.get(pos)?;
        if len_byte & LABEL_KIND_MASK != 0 {
            // Compression pointer or a reserved encoding — RFC 1035 §4.1.4.
            return None;
        }
        pos += 1;
        wire_len += 1;

        if len_byte == 0 {
            // Root label terminates the name.
            break;
        }

        let len = len_byte as usize;
        if len > MAX_LABEL_LEN {
            return None;
        }
        wire_len += len;
        if wire_len > MAX_NAME_WIRE_LEN {
            return None;
        }

        let end = pos.checked_add(len)?;
        let label = msg.get(pos..end)?;
        labels.push(std::str::from_utf8(label).ok()?);
        pos = end;
    }

    Some((labels.join(".").to_ascii_lowercase(), pos))
}

/// Build an NXDOMAIN response to `query`.
///
/// Returns `None` if `query` is not a query this module parses.
///
/// NXDOMAIN rather than an address record pointing at a sink: it needs no
/// answer section, so it is correct for every QTYPE at once — A, AAAA, and the
/// SVCB/HTTPS records modern clients also ask for — whereas an A record of
/// 0.0.0.0 answers only one of them and leaves the rest to resolve normally.
pub fn nxdomain_response(query: &[u8]) -> Option<Vec<u8>> {
    let (_, question_end) = parse_query_with_end(query)?;

    let mut out = query[..question_end].to_vec();

    let flags = u16::from_be_bytes([out[2], out[3]]);
    // Echo OPCODE and RD as required of a responder (RFC 1035 §4.1.1), set QR
    // and RA, clear AA/TC/Z, and answer Name Error.
    let response_flags =
        FLAG_QR | (flags & OPCODE_MASK) | (flags & FLAG_RD) | FLAG_RA | (RCODE_NXDOMAIN & RCODE_MASK);
    out[2..4].copy_from_slice(&response_flags.to_be_bytes());

    // QDCOUNT stays 1; every other section is empty.
    out[6..8].copy_from_slice(&0u16.to_be_bytes()); // ANCOUNT
    out[8..10].copy_from_slice(&0u16.to_be_bytes()); // NSCOUNT
    out[10..12].copy_from_slice(&0u16.to_be_bytes()); // ARCOUNT

    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode `name` in the length-prefixed label form of RFC 1035 §3.1.
    fn encode_name(name: &str) -> Vec<u8> {
        let mut out = Vec::new();
        if !name.is_empty() {
            for label in name.split('.') {
                out.push(label.len() as u8);
                out.extend_from_slice(label.as_bytes());
            }
        }
        out.push(0); // root
        out
    }

    /// Build a standard query for `name` — RFC 1035 §4.1.1 header + §4.1.2 question.
    fn query_for(name: &str, qtype: u16) -> Vec<u8> {
        let mut msg = Vec::new();
        msg.extend_from_slice(&0x1234u16.to_be_bytes()); // ID
        msg.extend_from_slice(&FLAG_RD.to_be_bytes()); // QR=0, OPCODE=0, RD=1
        msg.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT
        msg.extend_from_slice(&0u16.to_be_bytes()); // ANCOUNT
        msg.extend_from_slice(&0u16.to_be_bytes()); // NSCOUNT
        msg.extend_from_slice(&0u16.to_be_bytes()); // ARCOUNT
        msg.extend_from_slice(&encode_name(name));
        msg.extend_from_slice(&qtype.to_be_bytes());
        msg.extend_from_slice(&1u16.to_be_bytes()); // QCLASS = IN
        msg
    }

    /// QTYPE 1 = A — RFC 1035 §3.2.2.
    const QTYPE_A: u16 = 1;
    /// QTYPE 65 = HTTPS — RFC 9460 §14.1. Included because modern clients ask
    /// for it alongside A/AAAA and a sink-address answer would miss it.
    const QTYPE_HTTPS: u16 = 65;

    #[test]
    fn parses_the_question_from_a_standard_query() {
        let q = parse_query(&query_for("ads.example.com", QTYPE_A)).expect("parses");
        assert_eq!(q.id, 0x1234);
        assert_eq!(q.name, "ads.example.com");
        assert_eq!(q.qtype, QTYPE_A);
        assert_eq!(q.qclass, 1);
    }

    #[test]
    fn name_is_lowercased_for_filter_lookup() {
        // RFC 4343 §3 — case is not significant when matching names, and
        // DomainFilter indexes the lowercase form.
        let q = parse_query(&query_for("ADS.Example.COM", QTYPE_A)).expect("parses");
        assert_eq!(q.name, "ads.example.com");
    }

    #[test]
    fn root_query_parses_as_the_empty_name() {
        // A query for "." is well formed; it simply matches no filter rule.
        let q = parse_query(&query_for("", QTYPE_A)).expect("parses");
        assert_eq!(q.name, "");
    }

    #[test]
    fn a_response_is_not_treated_as_a_query() {
        let mut msg = query_for("example.com", QTYPE_A);
        let flags = u16::from_be_bytes([msg[2], msg[3]]) | FLAG_QR;
        msg[2..4].copy_from_slice(&flags.to_be_bytes());
        assert!(parse_query(&msg).is_none());
    }

    #[test]
    fn non_standard_opcode_is_rejected() {
        let mut msg = query_for("example.com", QTYPE_A);
        // OPCODE 5 = UPDATE (RFC 2136 §2) — not a name lookup.
        let flags = u16::from_be_bytes([msg[2], msg[3]]) | (5 << 11);
        msg[2..4].copy_from_slice(&flags.to_be_bytes());
        assert!(parse_query(&msg).is_none());
    }

    #[test]
    fn question_count_other_than_one_is_rejected() {
        for qdcount in [0u16, 2] {
            let mut msg = query_for("example.com", QTYPE_A);
            msg[4..6].copy_from_slice(&qdcount.to_be_bytes());
            assert!(parse_query(&msg).is_none(), "QDCOUNT={qdcount}");
        }
    }

    #[test]
    fn compression_pointer_in_the_question_is_rejected() {
        // 0xC0 0x0C points back at the header — RFC 1035 §4.1.4. Legal in a
        // response, never in a query's QNAME, and following it is the one way
        // a malformed input could loop.
        let mut msg = query_for("example.com", QTYPE_A);
        msg.truncate(HEADER_LEN);
        msg.extend_from_slice(&[0xc0, 0x0c]);
        msg.extend_from_slice(&QTYPE_A.to_be_bytes());
        msg.extend_from_slice(&1u16.to_be_bytes());
        assert!(parse_query(&msg).is_none());
    }

    #[test]
    fn truncated_buffers_are_rejected_without_panicking() {
        let full = query_for("ads.example.com", QTYPE_A);
        for cut in 0..full.len() {
            assert!(parse_query(&full[..cut]).is_none(), "cut at {cut}");
        }
        assert!(parse_query(&full).is_some());
    }

    #[test]
    fn name_longer_than_the_wire_limit_is_rejected() {
        // 255 bytes total, including length octets and the root terminator —
        // RFC 1035 §2.3.4.
        let long = std::iter::repeat_n("abcdefghij", 30)
            .collect::<Vec<_>>()
            .join(".");
        assert!(long.len() > MAX_NAME_WIRE_LEN);
        assert!(parse_query(&query_for(&long, QTYPE_A)).is_none());
    }

    #[test]
    fn nxdomain_response_mirrors_the_question_and_answers_name_error() {
        let query = query_for("ads.example.com", QTYPE_A);
        let reply = nxdomain_response(&query).expect("builds");

        assert_eq!(&reply[0..2], &query[0..2], "ID must be echoed");
        let flags = u16::from_be_bytes([reply[2], reply[3]]);
        assert_ne!(flags & FLAG_QR, 0, "QR must mark this a response");
        assert_ne!(flags & FLAG_RA, 0, "RA must be set by a responder");
        assert_ne!(flags & FLAG_RD, 0, "RD must be echoed from the query");
        assert_eq!(flags & RCODE_MASK, RCODE_NXDOMAIN);

        assert_eq!(u16::from_be_bytes([reply[4], reply[5]]), 1, "QDCOUNT");
        for (name, off) in [("ANCOUNT", 6), ("NSCOUNT", 8), ("ARCOUNT", 10)] {
            assert_eq!(
                u16::from_be_bytes([reply[off], reply[off + 1]]),
                0,
                "{name} must be empty"
            );
        }

        assert_eq!(
            &reply[HEADER_LEN..],
            &query[HEADER_LEN..],
            "the question must be echoed verbatim"
        );
    }

    #[test]
    fn nxdomain_answers_every_qtype_the_same_way() {
        // The reason NXDOMAIN was chosen over a sink address: one refusal
        // covers A, AAAA, and HTTPS/SVCB without an answer section per type.
        for qtype in [QTYPE_A, 28 /* AAAA, RFC 3596 §2.1 */, QTYPE_HTTPS] {
            let reply = nxdomain_response(&query_for("ads.example.com", qtype)).expect("builds");
            let flags = u16::from_be_bytes([reply[2], reply[3]]);
            assert_eq!(flags & RCODE_MASK, RCODE_NXDOMAIN, "qtype={qtype}");
            assert_eq!(u16::from_be_bytes([reply[6], reply[7]]), 0, "qtype={qtype}");
        }
    }

    #[test]
    fn nxdomain_response_drops_trailing_records() {
        // A query may carry an OPT record in the additional section (EDNS0,
        // RFC 6891 §6.1). The refusal must not echo it back, because ARCOUNT
        // is zeroed and the bytes would then be unaccounted for.
        let mut query = query_for("ads.example.com", QTYPE_A);
        let question_len = query.len();
        query[10..12].copy_from_slice(&1u16.to_be_bytes()); // ARCOUNT = 1
        query.extend_from_slice(&[0, 0, 41, 0x10, 0, 0, 0, 0, 0, 0, 0]); // bare OPT

        let reply = nxdomain_response(&query).expect("builds");
        assert_eq!(reply.len(), question_len);
        assert_eq!(u16::from_be_bytes([reply[10], reply[11]]), 0, "ARCOUNT");
    }

    #[test]
    fn nxdomain_response_declines_what_parse_declines() {
        assert!(nxdomain_response(&[]).is_none());
        assert!(nxdomain_response(&[0u8; HEADER_LEN]).is_none()); // QDCOUNT = 0
    }
}
