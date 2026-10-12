//! Where the client gets its access token. The app implements
//! [`TokenProvider`] over its account session; tests use a fake.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;

/// Result of asking the provider for a fresh token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Renewal {
    /// A new access token.
    Renewed(String),
    /// The session is gone for good (the refresh token was rejected). The
    /// client waits for a new session to appear.
    SessionGone,
    /// The refresh couldn't be completed right now (offline, 5xx). The client
    /// backs off and tries again.
    Unavailable,
}

/// Supplies access tokens. Both methods may block (the desktop's account
/// store uses blocking HTTP); the client calls them off its async threads.
pub trait TokenProvider: Send + Sync + 'static {
    /// The current access token, or `None` when there is no session yet.
    fn current(&self) -> Option<String>;

    /// The hub rejected `rejected` (4001), the account was merged (4005), or
    /// the token is about to expire. Return a fresh one. Implementations must
    /// be single-flight: if `rejected` is no longer the current token,
    /// return the current one instead of spending the refresh token again.
    fn renew(&self, rejected: &str) -> Renewal;
}

/// Seconds since the epoch at which a JWT expires, read from its `exp`
/// claim without verifying it (the hub verifies; this only decides whether
/// to refresh before connecting). `None` for anything that isn't a JWT.
pub fn jwt_expiry(token: &str) -> Option<u64> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    claims.get("exp")?.as_u64()
}

/// Whether `token` expires within `margin` of `now`. A token whose expiry
/// can't be read is treated as fresh: the hub's 4001 is the backstop.
pub fn expires_within(token: &str, margin: Duration, now: SystemTime) -> bool {
    let Some(exp) = jwt_expiry(token) else {
        return false;
    };
    let now = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    exp <= now.saturating_add(margin.as_secs())
}

#[cfg(test)]
pub(crate) mod test_jwt {
    use base64::Engine as _;

    /// An unsigned JWT-shaped token with the given `exp`.
    pub fn with_exp(exp: u64) -> String {
        let enc = |v: serde_json::Value| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v.to_string())
        };
        format!(
            "{}.{}.sig",
            enc(serde_json::json!({"alg": "HS256"})),
            enc(serde_json::json!({"sub": "u", "exp": exp}))
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_exp_claim() {
        assert_eq!(
            jwt_expiry(&test_jwt::with_exp(1_760_000_000)),
            Some(1_760_000_000)
        );
        assert_eq!(jwt_expiry("not-a-jwt"), None);
        assert_eq!(
            jwt_expiry("eyJhbGciOiJIUzI1NiJ9.e30.sig"),
            None,
            "no exp claim"
        );
    }

    #[test]
    fn near_expiry_uses_the_margin() {
        let now = UNIX_EPOCH + Duration::from_secs(1_000);
        let margin = Duration::from_secs(60);
        assert!(expires_within(&test_jwt::with_exp(1_030), margin, now));
        assert!(
            expires_within(&test_jwt::with_exp(900), margin, now),
            "already expired"
        );
        assert!(!expires_within(&test_jwt::with_exp(1_200), margin, now));
        assert!(
            !expires_within("opaque", margin, now),
            "unreadable = let the hub decide"
        );
    }
}
