pub const LENGTH_PREFIX: usize = 4;
pub const MAX_BODY: usize = 16 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum Decoded {
    NeedMore,
    Frame { body: Vec<u8>, consumed: usize },
    Oversized(u32),
}

#[derive(Debug, PartialEq, Eq)]
pub struct TooLarge;

pub fn encode(body: &[u8]) -> Result<Vec<u8>, TooLarge> {
    if body.len() > MAX_BODY {
        return Err(TooLarge);
    }
    let mut frame = Vec::with_capacity(LENGTH_PREFIX + body.len());
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    frame.extend_from_slice(body);
    Ok(frame)
}

pub fn decode(buf: &[u8]) -> Decoded {
    let Some(prefix) = buf.first_chunk::<LENGTH_PREFIX>() else {
        return Decoded::NeedMore;
    };
    let declared = u32::from_be_bytes(*prefix);
    if declared as usize > MAX_BODY {
        return Decoded::Oversized(declared);
    }
    let end = LENGTH_PREFIX + declared as usize;
    if buf.len() < end {
        return Decoded::NeedMore;
    }
    Decoded::Frame {
        body: buf[LENGTH_PREFIX..end].to_vec(),
        consumed: end,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn vectors() -> Value {
        serde_json::from_str(include_str!(
            "../../../../packages/daemon-ipc/tests/vectors/daemon-ipc-framing.json"
        ))
        .unwrap()
    }

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn vector_cap_matches_the_constant() {
        assert_eq!(vectors()["max_body"], MAX_BODY as u64);
    }

    #[test]
    fn valid_vectors_decode_and_encode_identically() {
        for case in vectors()["valid"].as_array().unwrap() {
            let frame = unhex(case["frame_hex"].as_str().unwrap());
            let body = case["body_utf8"].as_str().unwrap().as_bytes();
            assert_eq!(
                decode(&frame),
                Decoded::Frame {
                    body: body.to_vec(),
                    consumed: case["consumed"].as_u64().unwrap() as usize
                },
                "{}",
                case["name"]
            );
            assert_eq!(encode(body).unwrap(), frame, "{}", case["name"]);
        }
    }

    #[test]
    fn truncated_vectors_need_more() {
        for case in vectors()["needs_more"].as_array().unwrap() {
            assert_eq!(
                decode(&unhex(case["bytes_hex"].as_str().unwrap())),
                Decoded::NeedMore,
                "{}",
                case["name"]
            );
        }
    }

    #[test]
    fn oversized_vectors_are_refused_before_any_body_arrives() {
        for case in vectors()["oversized"].as_array().unwrap() {
            assert_eq!(
                decode(&unhex(case["bytes_hex"].as_str().unwrap())),
                Decoded::Oversized(case["declared"].as_u64().unwrap() as u32)
            );
        }
    }

    #[test]
    fn a_declared_length_at_the_cap_waits_for_the_body() {
        let prefix = unhex(
            vectors()["at_cap_is_not_oversized"]["bytes_hex"]
                .as_str()
                .unwrap(),
        );
        assert_eq!(decode(&prefix), Decoded::NeedMore);
    }

    #[test]
    fn a_second_frame_is_left_in_the_buffer() {
        let mut buf = encode(b"{}").unwrap();
        buf.extend(encode(b"{\"a\":1}").unwrap());
        let Decoded::Frame { consumed, .. } = decode(&buf) else {
            panic!("expected a frame");
        };
        assert_eq!(consumed, 6);
        assert!(matches!(decode(&buf[consumed..]), Decoded::Frame { .. }));
    }

    #[test]
    fn encoding_a_body_over_the_cap_is_refused() {
        assert_eq!(encode(&vec![b' '; MAX_BODY + 1]), Err(TooLarge));
    }
}
