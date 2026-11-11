pub mod frame;
pub mod msg;
pub mod session;

pub use frame::{FrameDecoder, Limits, Next, TooLarge};
pub use msg::{AppIdentity, ClientRole, MIN_PROTO, PROTO, ToDaemon, ToPanel};
pub use session::{CloseReason, End, Event, Inbound, Session};

pub fn encode_message<M: serde::Serialize>(
    message: &M,
    outbound: Limits,
) -> Result<Vec<u8>, TooLarge> {
    let body = serde_json::to_vec(message).map_err(|_| TooLarge)?;
    frame::encode(&body, outbound)
}

#[cfg(test)]
mod tests {
    #[test]
    fn unwinding_panics_are_enabled() {
        assert!(cfg!(panic = "unwind"));
    }
}
