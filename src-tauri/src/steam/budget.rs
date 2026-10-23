//! A request budget for Steam's Market, which refuses a connection for a
//! while once it has asked too much over the last hour or so.
//!
//! A refusal came after about 885 Market requests in an hour (one every 4
//! seconds for two hours). The budget lets a short burst through at the
//! normal pace, so clicks and small scans stay quick, and holds long reads to
//! one request every 7.5 seconds, about 480 an hour. If Steam still refuses,
//! the pace slows by half again and stays slower.
//!
//! After a refusal the app leaves that kind of request alone for a while.
//! Asking again while Steam is still refusing seemed to keep the refusal
//! going (one retry every 30 minutes was refused for hours), so each refusal
//! in a row doubles the wait, up to four hours.

use std::time::{Duration, Instant};

/// Requests that can go out at the normal pace before the budget applies.
pub const BURST: f64 = 60.0;
/// Time per request once the burst is used up.
pub const PERIOD_MS: u64 = 7_500;
/// The slowest the pace gets after refusals.
pub const MAX_PERIOD_MS: u64 = 30_000;
/// Answers in a row after which a pace slowed by refusals speeds back up a step.
pub const RECOVER_AFTER: u32 = 50;
/// The wait after a first refusal.
pub const FIRST_BLOCK_MINUTES: u64 = 30;
/// The longest wait after refusals in a row.
pub const MAX_BLOCK_MINUTES: u64 = 240;

/// How long to leave a kind of request alone after `refusals` refusals in a
/// row: 30 minutes, then 1, 2 and 4 hours.
pub fn block_minutes(refusals: u32) -> u64 {
    let doublings = refusals.saturating_sub(1).min(8);
    (FIRST_BLOCK_MINUTES << doublings).min(MAX_BLOCK_MINUTES)
}

#[derive(Debug, Clone)]
pub struct Budget {
    tokens: f64,
    period_ms: u64,
    last: Option<Instant>,
    /// Answers since the last refusal or speed-up.
    answered: u32,
}

impl Default for Budget {
    fn default() -> Self {
        Self::new(PERIOD_MS)
    }
}

impl Budget {
    pub fn new(period_ms: u64) -> Self {
        Self {
            tokens: BURST,
            period_ms: period_ms.clamp(PERIOD_MS, MAX_PERIOD_MS),
            last: None,
            answered: 0,
        }
    }

    pub fn period_ms(&self) -> u64 {
        self.period_ms
    }

    pub fn set_period_ms(&mut self, ms: u64) {
        self.period_ms = ms.clamp(PERIOD_MS, MAX_PERIOD_MS);
    }

    /// Takes one request from the budget at `now`, and says how long to wait
    /// before sending it.
    pub fn take(&mut self, now: Instant) -> Duration {
        let period = self.period_ms as f64 / 1000.0;
        if let Some(last) = self.last {
            let earned = now.saturating_duration_since(last).as_secs_f64() / period;
            self.tokens = (self.tokens + earned).min(BURST);
        }
        self.last = Some(now);
        self.tokens -= 1.0;
        if self.tokens >= 0.0 {
            Duration::ZERO
        } else {
            // Wait until the request is earned. The clock moves on to then,
            // so the next request is earned from that point.
            let wait = Duration::from_secs_f64(-self.tokens * period);
            self.tokens = 0.0;
            self.last = Some(now + wait);
            wait
        }
    }

    /// After a refusal: half again as slow, up to the slowest pace, and no
    /// burst until the budget has refilled.
    pub fn slow_down(&mut self) {
        self.period_ms = (self.period_ms * 3 / 2).clamp(PERIOD_MS, MAX_PERIOD_MS);
        self.tokens = 0.0;
        self.answered = 0;
    }

    /// After an answer: a pace slowed by refusals speeds back up a step every
    /// `RECOVER_AFTER` answers in a row, until it's back to normal.
    pub fn answered(&mut self) {
        if self.period_ms <= PERIOD_MS {
            self.answered = 0;
            return;
        }
        self.answered += 1;
        if self.answered >= RECOVER_AFTER {
            self.answered = 0;
            self.period_ms = (self.period_ms * 2 / 3).clamp(PERIOD_MS, MAX_PERIOD_MS);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sends `n` requests as fast as the budget and a 4-second gap allow,
    /// returning the time the last one went out.
    fn run(b: &mut Budget, start: Instant, n: usize) -> Instant {
        let mut t = start;
        for i in 0..n {
            if i > 0 {
                t += Duration::from_secs(4);
            }
            t += b.take(t);
        }
        t
    }

    #[test]
    fn bursts_then_holds_long_reads_to_the_period() {
        let start = Instant::now();
        let mut b = Budget::default();
        // The first 60 go out at the normal 4-second pace.
        let t = run(&mut b, start, 60);
        assert_eq!(t - start, Duration::from_secs(4 * 59));
        // An hour of requests after that stays near 480.
        let mut b = Budget::default();
        let mut t = start;
        let mut sent = 0;
        while t - start < Duration::from_secs(3600) {
            if sent > 0 {
                t += Duration::from_secs(4);
            }
            t += b.take(t);
            sent += 1;
        }
        assert!((520..=560).contains(&sent), "{sent} in the first hour");
        let mut later = 0;
        let hour_two = t + Duration::from_secs(3600);
        while t < hour_two {
            t += Duration::from_secs(4);
            t += b.take(t);
            later += 1;
        }
        assert!((470..=490).contains(&later), "{later} in the second hour");
    }

    #[test]
    fn refills_after_a_rest_and_slows_after_a_refusal() {
        let start = Instant::now();
        let mut b = Budget::default();
        run(&mut b, start, 100);
        // Ten quiet minutes earn back 60 / 7.5 s = 80, capped at the burst.
        let t = start + Duration::from_secs(4 * 99 + 600);
        assert_eq!(b.take(t), Duration::ZERO);
        b.slow_down();
        assert_eq!(b.period_ms(), 11_250);
        let wait = b.take(t + Duration::from_secs(4));
        assert!(wait > Duration::from_secs(7), "{wait:?}");
        for _ in 0..10 {
            b.slow_down();
        }
        assert_eq!(b.period_ms(), MAX_PERIOD_MS);
        b.set_period_ms(1);
        assert_eq!(b.period_ms(), PERIOD_MS);
    }

    #[test]
    fn speeds_back_up_after_answers_in_a_row() {
        let mut b = Budget::new(MAX_PERIOD_MS);
        let mut steps = Vec::new();
        for _ in 0..4 {
            for _ in 0..RECOVER_AFTER {
                b.answered();
            }
            steps.push(b.period_ms());
        }
        assert_eq!(steps, [20_000, 13_333, 8_888, PERIOD_MS]);
        // A refusal starts the count again.
        let mut b = Budget::new(MAX_PERIOD_MS);
        for _ in 0..RECOVER_AFTER - 1 {
            b.answered();
        }
        b.slow_down();
        b.answered();
        assert_eq!(b.period_ms(), MAX_PERIOD_MS);
    }

    #[test]
    fn waits_double_after_refusals_in_a_row() {
        let waits: Vec<u64> = (0..7).map(block_minutes).collect();
        assert_eq!(waits, [30, 30, 60, 120, 240, 240, 240]);
        assert_eq!(block_minutes(u32::MAX), MAX_BLOCK_MINUTES);
    }
}
