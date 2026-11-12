const TOKEN: u64 = 1000;

pub(crate) struct Bucket {
    capacity: u64,
    per_ms: u64,
    tokens: u64,
    last_ms: u64,
}

impl Bucket {
    pub(crate) fn new(per_sec: u32, now_ms: u64) -> Self {
        let capacity = u64::from(per_sec) * TOKEN;
        Self {
            capacity,
            per_ms: u64::from(per_sec),
            tokens: capacity,
            last_ms: now_ms,
        }
    }

    pub(crate) fn has_token(&mut self, now_ms: u64) -> bool {
        let elapsed = now_ms.saturating_sub(self.last_ms);
        self.tokens = self
            .tokens
            .saturating_add(elapsed.saturating_mul(self.per_ms))
            .min(self.capacity);
        self.last_ms = self.last_ms.max(now_ms);
        self.tokens >= TOKEN
    }

    pub(crate) fn spend(&mut self) {
        self.tokens = self.tokens.saturating_sub(TOKEN);
    }
}
