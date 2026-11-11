pub mod frame;
pub mod msg;
pub mod session;

pub use frame::{FrameDecoder, Limits, Next, TooLarge};
pub use msg::{AppIdentity, ClientRole, MIN_PROTO, PROTO, ToDaemon, ToPanel};
pub use session::{CloseReason, End, Event, Inbound, Session};

#[derive(Debug, PartialEq, Eq)]
pub enum EncodeError {
    TooLarge,
    Unserializable,
}

pub fn encode_message<M: serde::Serialize>(
    message: &M,
    outbound: Limits,
) -> Result<Vec<u8>, EncodeError> {
    let body = serde_json::to_vec(message).map_err(|_| EncodeError::Unserializable)?;
    frame::encode(&body, outbound).map_err(|TooLarge| EncodeError::TooLarge)
}

#[cfg(test)]
mod tests {
    #[test]
    fn unwinding_panics_are_enabled() {
        assert!(cfg!(panic = "unwind"));
    }
}
