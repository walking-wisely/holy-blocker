use std::time::Duration;

pub const INITIAL: Duration = Duration::from_millis(500);
pub const CAP: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct Backoff {
    initial: Duration,
    cap: Duration,
    next: Duration,
}

impl Backoff {
    pub fn new(initial: Duration, cap: Duration) -> Self {
        Self {
            initial,
            cap,
            next: initial,
        }
    }

    pub fn standard() -> Self {
        Self::new(INITIAL, CAP)
    }

    pub fn next_delay(&mut self) -> Duration {
        let delay = self.next;
        self.next = (self.next * 2).min(self.cap);
        delay
    }

    pub fn reset(&mut self) {
        self.next = self.initial;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_at_500_ms_and_doubles() {
        let mut b = Backoff::standard();
        let ms: Vec<u128> = (0..4).map(|_| b.next_delay().as_millis()).collect();
        assert_eq!(ms, [500, 1000, 2000, 4000]);
    }

    #[test]
    fn never_exceeds_30_seconds() {
        let mut b = Backoff::standard();
        let delays: Vec<Duration> = (0..20).map(|_| b.next_delay()).collect();
        assert_eq!(*delays.last().unwrap(), Duration::from_secs(30));
        assert!(delays.iter().all(|d| *d <= Duration::from_secs(30)));
    }

    #[test]
    fn reset_returns_to_the_initial_delay() {
        let mut b = Backoff::standard();
        for _ in 0..6 {
            b.next_delay();
        }
        b.reset();
        assert_eq!(b.next_delay(), Duration::from_millis(500));
    }
}
