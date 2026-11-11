use serde::{Deserialize, Serialize};

use crate::frame;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Outbound {
    RequestDisarm,
    ConfirmDisarm,
    CancelDisarm,
    RaiseProtection,
    ListApps,
}

impl Outbound {
    pub fn to_frame(self) -> Vec<u8> {
        let body = serde_json::to_vec(&self).expect("a unit variant always serialises");
        frame::encode(&body).expect("a request is far below the frame cap")
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Inbound {
    pub kind: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Malformed;

pub fn parse_inbound(body: &[u8]) -> Result<Inbound, Malformed> {
    #[derive(Deserialize)]
    struct Tag {
        r#type: String,
    }
    serde_json::from_slice::<Tag>(body)
        .map(|t| Inbound { kind: t.r#type })
        .map_err(|_| Malformed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_request_is_a_flat_object_with_a_snake_case_type() {
        let cases = [
            (Outbound::RequestDisarm, "request_disarm"),
            (Outbound::ConfirmDisarm, "confirm_disarm"),
            (Outbound::CancelDisarm, "cancel_disarm"),
            (Outbound::RaiseProtection, "raise_protection"),
            (Outbound::ListApps, "list_apps"),
        ];
        for (request, tag) in cases {
            let json = serde_json::to_value(request).unwrap();
            assert_eq!(json, serde_json::json!({ "type": tag }));
        }
    }

    #[test]
    fn a_request_frame_decodes_back_to_its_json() {
        let frame = Outbound::ListApps.to_frame();
        let frame::Decoded::Frame { body, .. } = frame::decode(&frame) else {
            panic!("expected a frame");
        };
        assert_eq!(body, br#"{"type":"list_apps"}"#);
    }

    #[test]
    fn inbound_keeps_only_the_type_tag() {
        let got = parse_inbound(br#"{"type":"status","anything":[1,2]}"#).unwrap();
        assert_eq!(got.kind, "status");
    }

    #[test]
    fn inbound_without_a_string_type_is_malformed() {
        for body in [&b"not json"[..], b"[]", b"{}", br#"{"type":3}"#] {
            assert_eq!(parse_inbound(body), Err(Malformed), "{body:?}");
        }
    }
}
