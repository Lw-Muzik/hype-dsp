//! Reconnect policy: what each close code means (§3.10) and how long to wait
//! before the next attempt (§3.1 step 4). Pure functions, so the rules are
//! tested directly rather than through socket timing.

use std::time::Duration;

use serde::Serialize;

/// Close codes the hub uses (§3.10).
pub mod close_code {
    pub const PROTOCOL_ERROR: u16 = 4000;
    pub const TOKEN_INVALID: u16 = 4001;
    pub const NO_HELLO: u16 = 4002;
    pub const UNLINKED: u16 = 4003;
    pub const REPLACED: u16 = 4004;
    pub const ACCOUNT_MERGED: u16 = 4005;
}

/// Why the client stopped for good (until it is started again).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// 4003: this device was revoked or belongs to another user.
    Unlinked,
    /// 4004: a newer socket for this device id replaced this one.
    Replaced,
    /// The app asked the client to stop.
    Shutdown,
    /// A `wss://` hub, but no TLS configuration could be built.
    TlsUnavailable,
}

/// What to do after the socket closed with `code` (`None` = no close frame:
/// a dropped connection, a failed connect, a timeout).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAction {
    /// Reconnect after the backoff delay.
    Reconnect,
    /// Get a fresh access token first, then reconnect (4001, 4005).
    RefreshThenReconnect,
    /// Do not reconnect.
    Stop(StopReason),
}

pub fn classify_close(code: Option<u16>) -> CloseAction {
    match code {
        Some(close_code::TOKEN_INVALID) | Some(close_code::ACCOUNT_MERGED) => {
            CloseAction::RefreshThenReconnect
        }
        Some(close_code::UNLINKED) => CloseAction::Stop(StopReason::Unlinked),
        Some(close_code::REPLACED) => CloseAction::Stop(StopReason::Replaced),
        // 4000 (protocol error), 4002 (no hello), normal closes, server
        // restarts and dropped connections all reconnect with backoff.
        _ => CloseAction::Reconnect,
    }
}

/// Exponential backoff: `base · 2^attempt`, capped, then scaled by a random
/// factor in `[1 − jitter, 1 + jitter)`.
///
/// The contract's "1 s, 2 s, 4 s … capped at 30 s, ±20 % jitter" is read as
/// jitter applied to the capped step, so a client at the cap waits 24–36 s.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Backoff {
    pub base: Duration,
    pub cap: Duration,
    /// Fraction, e.g. `0.2` for ±20 %.
    pub jitter: f64,
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            base: Duration::from_secs(1),
            cap: Duration::from_secs(30),
            jitter: 0.2,
        }
    }
}

impl Backoff {
    /// The delay before reconnect attempt number `attempt` (0 = the first
    /// retry after a session ended). `unit` is a uniform sample in `[0, 1)`.
    pub fn delay(&self, attempt: u32, unit: f64) -> Duration {
        let factor = 2f64.powi(attempt.min(30) as i32);
        let step = self.base.as_secs_f64() * factor;
        let capped = step.min(self.cap.as_secs_f64());
        let unit = unit.clamp(0.0, 1.0);
        let scale = 1.0 - self.jitter + 2.0 * self.jitter * unit;
        Duration::from_secs_f64((capped * scale).max(0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_codes_map_to_the_contract_reactions() {
        use close_code::*;
        assert_eq!(
            classify_close(Some(TOKEN_INVALID)),
            CloseAction::RefreshThenReconnect
        );
        assert_eq!(
            classify_close(Some(ACCOUNT_MERGED)),
            CloseAction::RefreshThenReconnect
        );
        assert_eq!(
            classify_close(Some(UNLINKED)),
            CloseAction::Stop(StopReason::Unlinked)
        );
        assert_eq!(
            classify_close(Some(REPLACED)),
            CloseAction::Stop(StopReason::Replaced)
        );
        for code in [PROTOCOL_ERROR, NO_HELLO, 1000, 1001, 1006, 1011] {
            assert_eq!(
                classify_close(Some(code)),
                CloseAction::Reconnect,
                "code {code}"
            );
        }
        assert_eq!(classify_close(None), CloseAction::Reconnect);
    }

    #[test]
    fn backoff_doubles_from_one_second_and_caps_at_thirty() {
        let b = Backoff::default();
        let mid = |n| b.delay(n, 0.5).as_secs_f64();
        assert_eq!(mid(0), 1.0);
        assert_eq!(mid(1), 2.0);
        assert_eq!(mid(2), 4.0);
        assert_eq!(mid(4), 16.0);
        assert_eq!(mid(5), 30.0);
        assert_eq!(mid(40), 30.0, "huge attempt counts must not overflow");
    }

    #[test]
    fn jitter_spans_plus_minus_twenty_percent() {
        let b = Backoff::default();
        assert!((b.delay(0, 0.0).as_secs_f64() - 0.8).abs() < 1e-9);
        assert!((b.delay(0, 0.999_999).as_secs_f64() - 1.2).abs() < 1e-5);
        assert!((b.delay(10, 0.0).as_secs_f64() - 24.0).abs() < 1e-9);
    }
}
