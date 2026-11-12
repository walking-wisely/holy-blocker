use crate::frame::{self, FrameDecoder, Limits, Next};
use crate::limiter::Bucket;
use crate::msg::{
    self, AppIdentity, ClientRole, Hello, MIN_PROTO, PROTO, Parsed, ToDaemon, ToPanel,
};

const MAX_PENDING_REMOVALS: usize = 16;

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
    PreHelloTimeout,
    FrameTimeout,
    Idle,
    HeartbeatLost,
    UnsupportedFlood,
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
    Throttled,
    Duplicate,
    Close(CloseReason),
}

/// Counts and the window they cover; nothing about the peer, a path or a message.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Counters {
    pub throttled: u64,
    pub duplicates: u64,
    pub window_ms: u64,
}

#[derive(Debug, PartialEq, Eq)]
enum State {
    AwaitingHello,
    Open,
    Closed,
}

struct Buckets {
    messages: Bucket,
    state_changes: Bucket,
}

impl Buckets {
    fn admit(&mut self, state_changing: bool, now_ms: u64) -> bool {
        let general = self.messages.has_token(now_ms);
        let state = !state_changing || self.state_changes.has_token(now_ms);
        if !(general && state) {
            return false;
        }
        self.messages.spend();
        if state_changing {
            self.state_changes.spend();
        }
        true
    }
}

pub struct Session {
    end: End,
    limits: Limits,
    decoder: FrameDecoder,
    state: State,
    created_ms: u64,
    last_activity_ms: u64,
    last_heartbeat_ms: u64,
    frame_started_ms: Option<u64>,
    unsupported_run: u32,
    buckets: Buckets,
    disarm_pending: bool,
    removals_pending: Vec<AppIdentity>,
    throttled: u64,
    duplicates: u64,
    window_started_ms: u64,
}

impl Session {
    pub fn new(end: End, limits: Limits, now_ms: u64) -> Self {
        Self {
            end,
            limits,
            decoder: FrameDecoder::new(limits),
            state: State::AwaitingHello,
            created_ms: now_ms,
            last_activity_ms: now_ms,
            last_heartbeat_ms: now_ms,
            frame_started_ms: None,
            unsupported_run: 0,
            buckets: Buckets {
                messages: Bucket::new(limits.messages_per_sec, now_ms),
                state_changes: Bucket::new(limits.state_changes_per_sec, now_ms),
            },
            disarm_pending: false,
            removals_pending: Vec::new(),
            throttled: 0,
            duplicates: 0,
            window_started_ms: now_ms,
        }
    }

    pub fn hello(&self, outbound: Limits) -> Result<Vec<u8>, frame::TooLarge> {
        frame::encode(&msg::hello_body(), outbound)
    }

    pub fn is_closed(&self) -> bool {
        self.state == State::Closed
    }

    pub fn buffered(&self) -> usize {
        self.decoder.buffered()
    }

    /// The server calls this once a disarm request is resolved some way the session cannot see
    /// (a refusal, an expiry); confirm and cancel resolve it on their own.
    pub fn settle_disarm(&mut self) {
        self.disarm_pending = false;
    }

    pub fn settle_removal(&mut self, identity: &AppIdentity) {
        self.removals_pending.retain(|pending| pending != identity);
    }

    pub fn drain_counters(&mut self, now_ms: u64) -> Counters {
        let counters = Counters {
            throttled: self.throttled,
            duplicates: self.duplicates,
            window_ms: now_ms.saturating_sub(self.window_started_ms),
        };
        self.throttled = 0;
        self.duplicates = 0;
        self.window_started_ms = self.window_started_ms.max(now_ms);
        counters
    }

    pub fn tick(&mut self, now_ms: u64) -> Option<CloseReason> {
        let reason = self.expired(now_ms)?;
        self.state = State::Closed;
        Some(reason)
    }

    pub fn receive(&mut self, bytes: &[u8], now_ms: u64) -> Vec<Event> {
        let mut events = Vec::new();
        if self.state == State::Closed {
            return events;
        }
        let late = self.expired(now_ms);
        if let Some(reason) = late.filter(|_| self.state != State::Open) {
            events.push(self.close(reason));
            return events;
        }
        if !bytes.is_empty() {
            self.last_activity_ms = self.last_activity_ms.max(now_ms);
        }
        let mut input = bytes;
        while self.state != State::Closed {
            match self.decoder.next(&mut input) {
                Next::NeedMore => break,
                Next::Oversized(_) => events.push(self.close(CloseReason::Oversized)),
                Next::Frame(body) => {
                    self.frame_started_ms = None;
                    events.extend(self.on_frame(&body, now_ms, late.is_some()));
                }
            }
        }
        if let Some(reason) = late.filter(|_| self.state != State::Closed) {
            events.push(self.close(reason));
        }
        if self.decoder.buffered() > 0 && self.frame_started_ms.is_none() {
            self.frame_started_ms = Some(now_ms);
        }
        events
    }

    fn expired(&self, now_ms: u64) -> Option<CloseReason> {
        if self.state == State::Closed {
            return None;
        }
        let since = |from: u64| now_ms.saturating_sub(from);
        let limits = &self.limits;
        if self.state == State::AwaitingHello
            && since(self.created_ms) >= limits.pre_hello_deadline_ms
        {
            return Some(CloseReason::PreHelloTimeout);
        }
        if self
            .frame_started_ms
            .is_some_and(|started| since(started) >= limits.frame_deadline_ms)
        {
            return Some(CloseReason::FrameTimeout);
        }
        if since(self.last_activity_ms) >= limits.idle_ms {
            return Some(CloseReason::Idle);
        }
        let watches_heartbeat =
            self.state == State::Open && self.end == End::Daemon(ClientRole::Agent);
        if watches_heartbeat && since(self.last_heartbeat_ms) >= limits.heartbeat_window_ms {
            return Some(CloseReason::HeartbeatLost);
        }
        None
    }

    fn close(&mut self, reason: CloseReason) -> Event {
        self.state = State::Closed;
        Event::Close(reason)
    }

    fn on_frame(&mut self, body: &[u8], now_ms: u64, late: bool) -> Option<Event> {
        match self.state {
            State::AwaitingHello => Some(self.on_first_frame(body, now_ms)),
            State::Open if late => self.on_late_frame(body),
            State::Open => Some(self.on_open_frame(body, now_ms)),
            State::Closed => unreachable!("receive stops once closed"),
        }
    }

    fn on_first_frame(&mut self, body: &[u8], now_ms: u64) -> Event {
        match msg::tag_of(body).as_deref() {
            None => self.close(CloseReason::Malformed),
            Some("hello") => match serde_json::from_slice::<Hello>(body) {
                Ok(Hello { proto }) if (MIN_PROTO..=PROTO).contains(&proto) => {
                    self.state = State::Open;
                    self.last_heartbeat_ms = now_ms;
                    Event::Hello { proto }
                }
                Ok(Hello { proto }) => self.close(CloseReason::ProtoMismatch(proto)),
                Err(_) => self.close(CloseReason::Malformed),
            },
            Some(_) => self.close(CloseReason::PreHello),
        }
    }

    /// Bytes that arrive after a deadline passed deliver only what dropping would fail open on.
    fn on_late_frame(&mut self, body: &[u8]) -> Option<Event> {
        let End::Daemon(role) = self.end else {
            return None;
        };
        match msg::parse::<ToDaemon>(body) {
            Parsed::Message(m) if m.allowed_for(role) && m.is_exempt_from_limits() => {
                self.track_pending(&m);
                Some(Event::Message(Inbound::ToDaemon(m)))
            }
            _ => None,
        }
    }

    fn on_open_frame(&mut self, body: &[u8], now_ms: u64) -> Event {
        if msg::tag_of(body).as_deref() == Some("hello") {
            return self.close(CloseReason::DuplicateHello);
        }
        match self.end {
            End::Daemon(role) => match msg::parse::<ToDaemon>(body) {
                Parsed::Message(m) if m.allowed_for(role) => self.admit(m, now_ms),
                Parsed::Message(_) => self.close(CloseReason::WrongRole),
                Parsed::Unsupported => self.on_unsupported(true, now_ms),
                Parsed::Malformed => self.close(CloseReason::Malformed),
            },
            End::Panel => match msg::parse::<ToPanel>(body) {
                Parsed::Message(m) => {
                    self.unsupported_run = 0;
                    Event::Message(Inbound::ToPanel(m))
                }
                Parsed::Unsupported => self.on_unsupported(false, now_ms),
                Parsed::Malformed => self.close(CloseReason::Malformed),
            },
            End::Agent => match msg::tag_of(body) {
                Some(_) => self.on_unsupported(false, now_ms),
                None => self.close(CloseReason::Malformed),
            },
        }
    }

    fn on_unsupported(&mut self, rate_limited: bool, now_ms: u64) -> Event {
        self.unsupported_run += 1;
        if self.unsupported_run > self.limits.unsupported_budget {
            return self.close(CloseReason::UnsupportedFlood);
        }
        if rate_limited && !self.buckets.admit(false, now_ms) {
            return self.throttle();
        }
        Event::Unsupported
    }

    fn throttle(&mut self) -> Event {
        self.throttled += 1;
        Event::Throttled
    }

    fn admit(&mut self, message: ToDaemon, now_ms: u64) -> Event {
        self.unsupported_run = 0;
        if message == ToDaemon::Heartbeat {
            self.last_heartbeat_ms = self.last_heartbeat_ms.max(now_ms);
        }
        if !message.is_exempt_from_limits()
            && !self.buckets.admit(message.is_state_changing(), now_ms)
        {
            return self.throttle();
        }
        match self.track_pending(&message) {
            Pending::New => Event::Message(Inbound::ToDaemon(message)),
            Pending::Repeated => {
                self.duplicates += 1;
                Event::Duplicate
            }
            Pending::Full => self.throttle(),
        }
    }

    fn track_pending(&mut self, message: &ToDaemon) -> Pending {
        match message {
            ToDaemon::RequestDisarm => {
                if std::mem::replace(&mut self.disarm_pending, true) {
                    return Pending::Repeated;
                }
            }
            ToDaemon::ConfirmDisarm | ToDaemon::CancelDisarm => self.disarm_pending = false,
            ToDaemon::RequestRemoveApp { identity } => {
                if self.removals_pending.contains(identity) {
                    return Pending::Repeated;
                }
                if self.removals_pending.len() >= MAX_PENDING_REMOVALS {
                    return Pending::Full;
                }
                self.removals_pending.push(identity.clone());
            }
            ToDaemon::ConfirmRemoveApp { identity } | ToDaemon::CancelRemoveApp { identity } => {
                self.removals_pending.retain(|pending| pending != identity);
            }
            _ => {}
        }
        Pending::New
    }
}

enum Pending {
    New,
    Repeated,
    Full,
}
