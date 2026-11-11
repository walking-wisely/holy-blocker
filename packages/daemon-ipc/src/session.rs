use crate::frame::{self, FrameDecoder, Limits, Next};
use crate::msg::{self, ClientRole, Hello, MIN_PROTO, PROTO, Parsed, ToDaemon, ToPanel};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    Daemon(ClientRole),
    Panel,
    Agent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    PreHello,
    ProtoMismatch(u32),
    DuplicateHello,
    Malformed,
    Oversized,
    WrongRole,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inbound {
    ToDaemon(ToDaemon),
    ToPanel(ToPanel),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Hello { proto: u32 },
    Message(Inbound),
    Unsupported,
    Close(CloseReason),
}

#[derive(Debug, PartialEq, Eq)]
enum State {
    AwaitingHello,
    Open,
    Closed,
}

pub struct Session {
    end: End,
    decoder: FrameDecoder,
    state: State,
}

impl Session {
    pub fn new(end: End, limits: Limits) -> Self {
        Self {
            end,
            decoder: FrameDecoder::new(limits),
            state: State::AwaitingHello,
        }
    }

    pub fn hello(&self, outbound: Limits) -> Vec<u8> {
        frame::encode(&msg::hello_body(), outbound).expect("a hello is far below any frame cap")
    }

    pub fn is_closed(&self) -> bool {
        self.state == State::Closed
    }

    pub fn buffered(&self) -> usize {
        self.decoder.buffered()
    }

    pub fn receive(&mut self, bytes: &[u8]) -> Vec<Event> {
        let mut events = Vec::new();
        let mut input = bytes;
        while self.state != State::Closed {
            match self.decoder.next(&mut input) {
                Next::NeedMore => break,
                Next::Oversized(_) => events.push(self.close(CloseReason::Oversized)),
                Next::Frame(body) => events.push(self.on_frame(&body)),
            }
        }
        events
    }

    fn close(&mut self, reason: CloseReason) -> Event {
        self.state = State::Closed;
        Event::Close(reason)
    }

    fn on_frame(&mut self, body: &[u8]) -> Event {
        match self.state {
            State::AwaitingHello => self.on_first_frame(body),
            State::Open => self.on_open_frame(body),
            State::Closed => unreachable!("receive stops once closed"),
        }
    }

    fn on_first_frame(&mut self, body: &[u8]) -> Event {
        match msg::tag_of(body).as_deref() {
            None => self.close(CloseReason::Malformed),
            Some("hello") => match serde_json::from_slice::<Hello>(body) {
                Ok(Hello { proto }) if (MIN_PROTO..=PROTO).contains(&proto) => {
                    self.state = State::Open;
                    Event::Hello { proto }
                }
                Ok(Hello { proto }) => self.close(CloseReason::ProtoMismatch(proto)),
                Err(_) => self.close(CloseReason::Malformed),
            },
            Some(_) => self.close(CloseReason::PreHello),
        }
    }

    fn on_open_frame(&mut self, body: &[u8]) -> Event {
        if msg::tag_of(body).as_deref() == Some("hello") {
            return self.close(CloseReason::DuplicateHello);
        }
        match self.end {
            End::Daemon(role) => match msg::parse::<ToDaemon>(body) {
                Parsed::Message(m) if m.allowed_for(role) => Event::Message(Inbound::ToDaemon(m)),
                Parsed::Message(_) => self.close(CloseReason::WrongRole),
                Parsed::Unsupported => Event::Unsupported,
                Parsed::Malformed => self.close(CloseReason::Malformed),
            },
            End::Panel => match msg::parse::<ToPanel>(body) {
                Parsed::Message(m) => Event::Message(Inbound::ToPanel(m)),
                Parsed::Unsupported => Event::Unsupported,
                Parsed::Malformed => self.close(CloseReason::Malformed),
            },
            End::Agent => match msg::tag_of(body) {
                Some(_) => Event::Unsupported,
                None => self.close(CloseReason::Malformed),
            },
        }
    }
}
