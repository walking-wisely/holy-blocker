pub const LENGTH_PREFIX: usize = 4;

pub const TO_DAEMON_MAX_FRAME: u32 = 4 * 1024;
pub const TO_PANEL_MAX_FRAME: u32 = 64 * 1024;
pub const FRAME_DEADLINE_MS: u64 = 5_000;
pub const PRE_HELLO_DEADLINE_MS: u64 = 5_000;
pub const IDLE_MS: u64 = 60_000;
pub const HEARTBEAT_WINDOW_MS: u64 = 15_000;
pub const UNSUPPORTED_BUDGET: u32 = 8;
pub const MESSAGES_PER_SEC: u32 = 10;
pub const STATE_CHANGES_PER_SEC: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_frame: u32,
    pub frame_deadline_ms: u64,
    pub pre_hello_deadline_ms: u64,
    pub idle_ms: u64,
    pub heartbeat_window_ms: u64,
    pub unsupported_budget: u32,
    pub messages_per_sec: u32,
    pub state_changes_per_sec: u32,
}

impl Limits {
    pub const fn new(max_frame: u32) -> Self {
        Self {
            max_frame,
            frame_deadline_ms: FRAME_DEADLINE_MS,
            pre_hello_deadline_ms: PRE_HELLO_DEADLINE_MS,
            idle_ms: IDLE_MS,
            heartbeat_window_ms: HEARTBEAT_WINDOW_MS,
            unsupported_budget: UNSUPPORTED_BUDGET,
            messages_per_sec: MESSAGES_PER_SEC,
            state_changes_per_sec: STATE_CHANGES_PER_SEC,
        }
    }

    pub const fn to_daemon() -> Self {
        Self::new(TO_DAEMON_MAX_FRAME)
    }

    pub const fn to_panel() -> Self {
        Self::new(TO_PANEL_MAX_FRAME)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct TooLarge;

pub fn encode(body: &[u8], limits: Limits) -> Result<Vec<u8>, TooLarge> {
    if body.len() > limits.max_frame as usize {
        return Err(TooLarge);
    }
    let mut frame = Vec::with_capacity(LENGTH_PREFIX + body.len());
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    frame.extend_from_slice(body);
    Ok(frame)
}

#[derive(PartialEq, Eq)]
pub enum Next {
    NeedMore,
    Frame(Vec<u8>),
    Oversized(u32),
}

impl std::fmt::Debug for Next {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NeedMore => f.write_str("NeedMore"),
            Self::Frame(body) => write!(f, "Frame(<{} bytes>)", body.len()),
            Self::Oversized(declared) => write!(f, "Oversized({declared})"),
        }
    }
}

pub struct FrameDecoder {
    limits: Limits,
    buf: Vec<u8>,
    oversized: Option<u32>,
}

impl FrameDecoder {
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            buf: Vec::new(),
            oversized: None,
        }
    }

    pub fn buffered(&self) -> usize {
        self.buf.len()
    }

    /// Consumes from `input` only what the current frame needs, so the buffer never exceeds
    /// `LENGTH_PREFIX + max_frame` however large a single read is.
    pub fn next(&mut self, input: &mut &[u8]) -> Next {
        if let Some(declared) = self.oversized {
            return Next::Oversized(declared);
        }
        if self.buf.len() < LENGTH_PREFIX {
            self.take(input, LENGTH_PREFIX - self.buf.len());
            if self.buf.len() < LENGTH_PREFIX {
                return Next::NeedMore;
            }
        }
        let declared = u32::from_be_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]);
        if declared > self.limits.max_frame {
            self.buf.clear();
            self.oversized = Some(declared);
            return Next::Oversized(declared);
        }
        let total = LENGTH_PREFIX.saturating_add(declared as usize);
        self.take(input, total - self.buf.len());
        if self.buf.len() < total {
            return Next::NeedMore;
        }
        let body = self.buf.split_off(LENGTH_PREFIX);
        self.buf.clear();
        Next::Frame(body)
    }

    fn take(&mut self, input: &mut &[u8], want: usize) {
        let n = want.min(input.len());
        self.buf.extend_from_slice(&input[..n]);
        *input = &input[n..];
    }
}
