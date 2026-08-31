use std::time::Duration;

/// Adaptive poll interval for background DB loops.
///
/// Polling at a fixed short cadence keeps a scale-to-zero Postgres (Neon,
/// Aurora Serverless) permanently awake, which bills compute time around the
/// clock. Backing off while there is no work lets the compute suspend.
pub struct IdleBackoff {
    base: Duration,
    max: Duration,
    current: Duration,
}

impl IdleBackoff {
    pub fn new(base: Duration, max: Duration) -> Self {
        Self {
            base,
            max: max.max(base),
            current: base,
        }
    }

    pub fn interval(&self) -> Duration {
        self.current
    }

    pub fn saw_work(&mut self) {
        self.current = self.base;
    }

    pub fn saw_idle(&mut self) {
        self.current = (self.current * 2).min(self.max);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doubles_while_idle_then_holds_at_max() {
        let mut b = IdleBackoff::new(Duration::from_secs(5), Duration::from_secs(20));
        assert_eq!(b.interval(), Duration::from_secs(5));
        b.saw_idle();
        assert_eq!(b.interval(), Duration::from_secs(10));
        b.saw_idle();
        assert_eq!(b.interval(), Duration::from_secs(20));
        b.saw_idle();
        assert_eq!(b.interval(), Duration::from_secs(20));
    }

    #[test]
    fn work_resets_to_base() {
        let mut b = IdleBackoff::new(Duration::from_secs(5), Duration::from_secs(60));
        b.saw_idle();
        b.saw_idle();
        b.saw_work();
        assert_eq!(b.interval(), Duration::from_secs(5));
    }

    #[test]
    fn max_below_base_is_clamped_to_base() {
        let mut b = IdleBackoff::new(Duration::from_secs(30), Duration::from_secs(5));
        b.saw_idle();
        assert_eq!(b.interval(), Duration::from_secs(30));
    }
}
