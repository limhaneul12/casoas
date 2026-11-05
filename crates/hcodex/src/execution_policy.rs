//! Optional caller-owned job deadlines, separate from bounded transport/startup I/O.
use std::time::{Duration, Instant};

/// Zero seconds represents an unbounded job lifetime. Positive budgets retain
/// their historical meaning; cancellation remains independently supervised.
#[derive(Clone, Copy, Debug, Default)]
pub struct ExecutionDeadline {
    at: Option<Instant>,
}
impl ExecutionDeadline {
    pub fn from_limit(limit: Duration) -> Self {
        let now = Instant::now();
        Self {
            at: if limit.is_zero() {
                None
            } else {
                Some(now.checked_add(limit).unwrap_or(now))
            },
        }
    }
    pub fn from_seconds(seconds: f64) -> crate::Result<Self> {
        if !seconds.is_finite() || !(0.0..=86_400.0).contains(&seconds) {
            return Err(
                "execution timeout must be zero (unbounded) or at most 86400 seconds".into(),
            );
        }
        let limit = Duration::from_secs_f64(seconds);
        if seconds > 0.0 && limit.is_zero() {
            return Ok(Self::from(Instant::now()));
        }
        Ok(Self::from_limit(limit))
    }
    pub fn remaining(self) -> Option<Duration> {
        self.at
            .map(|at| at.saturating_duration_since(Instant::now()))
    }
    /// Remaining seconds on the wire: `Some(0.0)` means unbounded; `None`
    /// means an explicit budget expired and must never be redispatched.
    pub fn remaining_seconds(self) -> Option<f64> {
        match self.remaining() {
            None => Some(0.0),
            Some(remaining) if remaining.is_zero() => None,
            Some(remaining) => Some(remaining.as_secs_f64()),
        }
    }
    pub fn is_elapsed(self) -> bool {
        self.at.is_some_and(|at| Instant::now() >= at)
    }
    pub fn bounded_io_deadline(self, maximum: Duration) -> Instant {
        let now = Instant::now();
        let maximum = now.checked_add(maximum).unwrap_or(now);
        self.at.map_or(maximum, |at| at.min(maximum))
    }
    pub async fn wait(self) {
        match self.at {
            Some(at) => tokio::time::sleep_until(tokio::time::Instant::from_std(at)).await,
            None => std::future::pending::<()>().await,
        }
    }
}
impl From<Instant> for ExecutionDeadline {
    fn from(at: Instant) -> Self {
        Self { at: Some(at) }
    }
}

/// Existing duration-taking process APIs use the same zero-means-unbounded rule.
pub async fn wait_for_limit(limit: Duration) {
    ExecutionDeadline::from_limit(limit).wait().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_deadline_is_distinct_from_an_elapsed_explicit_deadline() -> crate::Result<()> {
        let unbounded = ExecutionDeadline::from_seconds(0.0)?;
        assert_eq!(unbounded.remaining(), None);
        assert_eq!(unbounded.remaining_seconds(), Some(0.0));
        assert!(!unbounded.is_elapsed());
        let elapsed = ExecutionDeadline::from(Instant::now());
        assert!(elapsed.is_elapsed());
        assert_eq!(elapsed.remaining(), Some(Duration::ZERO));
        assert_eq!(elapsed.remaining_seconds(), None);
        assert!(ExecutionDeadline::from_seconds(0.000_000_000_01)?.is_elapsed());
        assert!(ExecutionDeadline::from_seconds(f64::NAN).is_err());
        assert!(ExecutionDeadline::from_seconds(-1.0).is_err());
        Ok(())
    }
}
