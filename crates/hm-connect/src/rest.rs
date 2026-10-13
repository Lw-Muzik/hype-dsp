//! REST paths for linking and device management (§2), relative to the API
//! base the app already uses (e.g. `http://host:9400/api`). The HTTP itself
//! lives in the app, next to its token store; this module only builds paths
//! and refuses ids that could escape their path segment.

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PathError {
    #[error("a link code is 6 digits")]
    BadCode,
    #[error("not a valid id")]
    BadId,
    #[error("the API base must start with http:// or https://")]
    BadBase,
}

pub const LINK_DEVICE: &str = "/connect/links/device";
pub const DEVICES: &str = "/connect/devices";

/// `ws(s)://<api-host>/api/connect/ws` from the REST base `http(s)://<api-host>/api`.
pub fn ws_url(api_base: &str) -> Result<String, PathError> {
    let base = api_base.trim().trim_end_matches('/');
    let rest = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        return Err(PathError::BadBase);
    };
    Ok(format!("{rest}/connect/ws"))
}

/// Normalise what the user typed (spaces, dashes) into a 6-digit code.
pub fn normalize_code(input: &str) -> Result<String, PathError> {
    let digits: String = input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect();
    if digits.len() == 6 && digits.bytes().all(|b| b.is_ascii_digit()) {
        Ok(digits)
    } else {
        Err(PathError::BadCode)
    }
}

fn checked_id(id: &str) -> Result<&str, PathError> {
    let ok = !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    ok.then_some(id).ok_or(PathError::BadId)
}

/// `GET /connect/links/:code`
pub fn link_preview(code: &str) -> Result<String, PathError> {
    Ok(format!("/connect/links/{}", normalize_code(code)?))
}

/// `POST /connect/links/:code/approve`
pub fn link_approve(code: &str) -> Result<String, PathError> {
    Ok(format!("/connect/links/{}/approve", normalize_code(code)?))
}

/// `POST /connect/links/:code/deny`
pub fn link_deny(code: &str) -> Result<String, PathError> {
    Ok(format!("/connect/links/{}/deny", normalize_code(code)?))
}

/// `POST /connect/links/:linkId/claim`
pub fn link_claim(link_id: &str) -> Result<String, PathError> {
    Ok(format!("/connect/links/{}/claim", checked_id(link_id)?))
}

/// `DELETE /connect/devices/:id`
pub fn device(id: &str) -> Result<String, PathError> {
    Ok(format!("/connect/devices/{}", checked_id(id)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_url_follows_the_api_base() {
        assert_eq!(
            ws_url("http://37.60.225.220:9400/api").unwrap(),
            "ws://37.60.225.220:9400/api/connect/ws"
        );
        assert_eq!(
            ws_url("https://api.hypemuzik.app/api/").unwrap(),
            "wss://api.hypemuzik.app/api/connect/ws"
        );
        assert_eq!(ws_url("ftp://x").unwrap_err(), PathError::BadBase);
    }

    #[test]
    fn codes_are_six_digits_after_trimming_separators() {
        assert_eq!(normalize_code(" 483-920 ").unwrap(), "483920");
        assert_eq!(normalize_code("48392").unwrap_err(), PathError::BadCode);
        assert_eq!(normalize_code("48392a").unwrap_err(), PathError::BadCode);
        assert_eq!(normalize_code("../../x").unwrap_err(), PathError::BadCode);
        assert_eq!(
            link_approve("483920").unwrap(),
            "/connect/links/483920/approve"
        );
    }

    #[test]
    fn ids_cannot_escape_their_segment() {
        assert_eq!(
            device("6f1d2c3b-4a59-4e6f-8a7b-9c0d1e2f3a4b").unwrap(),
            "/connect/devices/6f1d2c3b-4a59-4e6f-8a7b-9c0d1e2f3a4b"
        );
        assert_eq!(
            link_claim("cm1link0000000001abcdefg").unwrap(),
            "/connect/links/cm1link0000000001abcdefg/claim"
        );
        for bad in ["", "../me", "a/b", "a?b=c", "a b"] {
            assert_eq!(device(bad).unwrap_err(), PathError::BadId, "{bad:?}");
        }
    }
}
