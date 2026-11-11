pub const LENGTH_PREFIX: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_frame: u32,
}

impl Limits {
    pub const fn new(max_frame: u32) -> Self {
        Self { max_frame }
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
