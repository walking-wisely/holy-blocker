/// Extract the SNI hostname from a raw TLS 1.x ClientHello record.
///
/// Returns `None` on partial buffers, malformed records, missing SNI extension,
/// or non-UTF-8 hostnames. No external TLS library is used — this is a targeted
/// byte-level parse of the stable TLS record/handshake header structures.
pub fn extract_sni(buf: &[u8]) -> Option<String> {
    let mut pos = 0;

    // TLS record header: content_type(1) + legacy_version(2) + length(2)
    if buf.len() < 5 {
        return None;
    }
    if buf[pos] != 0x16 {
        // not a Handshake record
        return None;
    }
    pos += 3; // skip content_type + legacy_version
    let record_len = u16::from_be_bytes([buf[pos], buf[pos + 1]]) as usize;
    pos += 2;

    if buf.len() < pos + record_len {
        return None;
    }
    let record_end = pos + record_len;

    // Handshake header: msg_type(1) + length(3)
    if pos + 4 > record_end {
        return None;
    }
    if buf[pos] != 0x01 {
        // not a ClientHello
        return None;
    }
    pos += 1;
    let hs_len = u24_to_usize(&buf[pos..pos + 3]);
    pos += 3;

    if pos + hs_len > record_end {
        return None;
    }
    let hs_end = pos + hs_len;

    // ClientHello body:
    //   legacy_version(2) + random(32) + session_id_len(1) + session_id(var)
    //   + cipher_suites_len(2) + cipher_suites(var)
    //   + compression_methods_len(1) + compression_methods(var)
    //   + extensions_len(2) + extensions(var)

    // legacy_version + random
    if pos + 34 > hs_end {
        return None;
    }
    pos += 34;

    // session_id
    if pos + 1 > hs_end {
        return None;
    }
    let sid_len = buf[pos] as usize;
    pos += 1 + sid_len;

    // cipher_suites
    if pos + 2 > hs_end {
        return None;
    }
    let cs_len = u16::from_be_bytes([buf[pos], buf[pos + 1]]) as usize;
    pos += 2 + cs_len;

    // compression_methods
    if pos + 1 > hs_end {
        return None;
    }
    let comp_len = buf[pos] as usize;
    pos += 1 + comp_len;

    // extensions
    if pos + 2 > hs_end {
        return None;
    }
    let ext_total = u16::from_be_bytes([buf[pos], buf[pos + 1]]) as usize;
    pos += 2;

    if pos + ext_total > hs_end {
        return None;
    }
    let ext_end = pos + ext_total;

    while pos + 4 <= ext_end {
        let ext_type = u16::from_be_bytes([buf[pos], buf[pos + 1]]);
        let ext_len = u16::from_be_bytes([buf[pos + 2], buf[pos + 3]]) as usize;
        pos += 4;

        if pos + ext_len > ext_end {
            return None;
        }

        if ext_type == 0x0000 {
            // server_name extension
            // server_name_list_len(2) + name_type(1) + name_len(2) + name(var)
            if ext_len < 5 {
                return None;
            }
            let list_len = u16::from_be_bytes([buf[pos], buf[pos + 1]]) as usize;
            if list_len < 3 || pos + 2 + list_len > ext_end {
                return None;
            }
            let name_type = buf[pos + 2];
            if name_type != 0x00 {
                // not host_name type
                return None;
            }
            let name_len = u16::from_be_bytes([buf[pos + 3], buf[pos + 4]]) as usize;
            let name_start = pos + 5;
            if name_start + name_len > ext_end {
                return None;
            }
            return std::str::from_utf8(&buf[name_start..name_start + name_len])
                .ok()
                .map(|s| s.to_owned());
        }

        pos += ext_len;
    }

    None
}

fn u24_to_usize(b: &[u8]) -> usize {
    ((b[0] as usize) << 16) | ((b[1] as usize) << 8) | (b[2] as usize)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a minimal TLS ClientHello record buffer with a single SNI entry.
    fn build_client_hello(sni: Option<&str>) -> Vec<u8> {
        // extensions payload
        let ext_payload = match sni {
            Some(name) => {
                let name_bytes = name.as_bytes();
                let name_len = name_bytes.len() as u16;
                let list_len = (3 + name_bytes.len()) as u16;
                let mut e = Vec::new();
                e.extend_from_slice(&0x0000u16.to_be_bytes()); // ext type: server_name
                let ext_data_len = (2 + 1 + 2 + name_bytes.len()) as u16;
                e.extend_from_slice(&ext_data_len.to_be_bytes());
                e.extend_from_slice(&list_len.to_be_bytes());
                e.push(0x00); // name_type: host_name
                e.extend_from_slice(&name_len.to_be_bytes());
                e.extend_from_slice(name_bytes);
                e
            }
            None => vec![],
        };

        // ClientHello body (before extensions)
        let mut hello_body: Vec<u8> = Vec::new();
        hello_body.extend_from_slice(&[0x03, 0x03]); // legacy_version TLS 1.2
        hello_body.extend_from_slice(&[0u8; 32]); // random
        hello_body.push(0x00); // session_id_len = 0
        hello_body.extend_from_slice(&2u16.to_be_bytes()); // cipher_suites_len
        hello_body.extend_from_slice(&[0x00, 0x2f]); // TLS_RSA_WITH_AES_128_CBC_SHA
        hello_body.push(0x01); // compression_methods_len
        hello_body.push(0x00); // null compression
        hello_body.extend_from_slice(&(ext_payload.len() as u16).to_be_bytes());
        hello_body.extend_from_slice(&ext_payload);

        // Handshake header
        let hs_len = hello_body.len();
        let mut handshake: Vec<u8> = Vec::new();
        handshake.push(0x01); // ClientHello
        handshake.push(((hs_len >> 16) & 0xff) as u8);
        handshake.push(((hs_len >> 8) & 0xff) as u8);
        handshake.push((hs_len & 0xff) as u8);
        handshake.extend_from_slice(&hello_body);

        // TLS record header
        let mut record: Vec<u8> = Vec::new();
        record.push(0x16); // Handshake
        record.extend_from_slice(&[0x03, 0x01]); // TLS 1.0 legacy version
        record.extend_from_slice(&(handshake.len() as u16).to_be_bytes());
        record.extend_from_slice(&handshake);
        record
    }

    #[test]
    fn extracts_sni_from_well_formed_client_hello() {
        let buf = build_client_hello(Some("example.com"));
        assert_eq!(extract_sni(&buf), Some("example.com".to_owned()));
    }

    #[test]
    fn returns_none_when_no_sni_extension() {
        let buf = build_client_hello(None);
        assert_eq!(extract_sni(&buf), None);
    }

    #[test]
    fn returns_none_on_empty_buffer() {
        assert_eq!(extract_sni(&[]), None);
    }

    #[test]
    fn returns_none_on_truncated_record_header() {
        let buf = build_client_hello(Some("example.com"));
        // only 3 bytes — not enough for the 5-byte record header
        assert_eq!(extract_sni(&buf[..3]), None);
    }

    #[test]
    fn returns_none_when_record_length_exceeds_buffer() {
        let mut buf = build_client_hello(Some("example.com"));
        // inflate the record length field so it claims more data than present
        let claimed: u16 = buf.len() as u16 + 100;
        buf[3] = (claimed >> 8) as u8;
        buf[4] = (claimed & 0xff) as u8;
        assert_eq!(extract_sni(&buf), None);
    }

    #[test]
    fn returns_none_when_content_type_is_not_handshake() {
        let mut buf = build_client_hello(Some("example.com"));
        buf[0] = 0x17; // Application Data, not Handshake
        assert_eq!(extract_sni(&buf), None);
    }

    #[test]
    fn returns_none_when_handshake_type_is_not_client_hello() {
        let mut buf = build_client_hello(Some("example.com"));
        buf[5] = 0x02; // ServerHello, not ClientHello
        assert_eq!(extract_sni(&buf), None);
    }

    #[test]
    fn returns_none_on_truncated_at_session_id() {
        let buf = build_client_hello(Some("example.com"));
        // cut off just after the random bytes (2 + 32 bytes into hs body = 39 bytes)
        // record(5) + hs_header(4) + legacy_version(2) + random(32) = 43
        assert_eq!(extract_sni(&buf[..43]), None);
    }

    #[test]
    fn returns_none_on_truncated_at_extensions() {
        let buf = build_client_hello(Some("example.com"));
        // drop the last 10 bytes to cut into the extension list
        let len = buf.len();
        assert_eq!(extract_sni(&buf[..len - 10]), None);
    }

    #[test]
    fn handles_subdomain_sni() {
        let buf = build_client_hello(Some("deep.sub.example.com"));
        assert_eq!(extract_sni(&buf), Some("deep.sub.example.com".to_owned()));
    }
}
