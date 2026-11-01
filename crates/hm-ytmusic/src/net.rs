//! The HTTP client under every YT Music metadata call.
//!
//! ytmapi-rs builds its own client as bare `reqwest::Client::new()`, and in
//! Nov 2026 that stopped working: every session build failed with "YouTube
//! Music rejected the session: Web error <error sending request for url
//! (https://music.youtube.com/)> received." — and since every call shares the
//! session, Explore, search, playlists and radio all went with it.
//!
//! The cause was the response, not the session: the homepage's headers grew
//! past hyper's 16 KB HTTP/2 limit (see [`MAX_HEADER_LIST`]), and h2 resets
//! such a stream before reqwest ever sees a status.
//!
//! The same bare client also has no connect timeout, read timeout or retry,
//! which a lossy link punishes on its own. Measured against
//! music.youtube.com from one, 40 fetches: five stalled for 19–76 s, pinned
//! to one address while macOS waited out its SYN backoff, and one died when
//! the far end dropped mid-handshake (`tls handshake eof`) — the same
//! "error sending request" text. With a connect timeout set: 25 of 25
//! through, slowest 6 s.

use std::time::Duration;

/// The host every ytmapi-rs query goes to.
const YTM_HOST: &str = "music.youtube.com";

/// Budget for one connect — TCP *and* TLS. hyper also splits it across the
/// addresses the name resolves to (eight per family for music.youtube.com,
/// so ~1.9 s each), which is the part that matters: a lost SYN costs a hop
/// to the next address instead of the OS's ~75 s connect backoff.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// No bytes for this long on an open request means the connection is dead —
/// typically one that went stale across sleep or a network change, which
/// would otherwise hang until the kernel noticed, minutes later.
const READ_TIMEOUT: Duration = Duration::from_secs(20);

/// Extra attempts after a failed connect.
const CONNECT_RETRIES: u32 = 2;

/// Ceiling on a response's decoded HTTP/2 header list. hyper's default is
/// 16 KB, and the music.youtube.com homepage — the page every session is
/// built from — outgrew it: ~18 KB once YouTube added a 7 KB
/// `connection-allowlist-report-only` beside a 6 KB CSP report-only, both
/// enumerating every Google ccTLD. Over the limit, h2 resets the stream
/// itself (`PROTOCOL_ERROR`), surfacing as a bare "error sending request".
/// curl allows 300 KB; this leaves YouTube the same kind of room to keep
/// growing.
const MAX_HEADER_LIST: u32 = 256 * 1024;

/// The client ytmapi-rs should use.
pub(crate) fn client() -> Result<ytmapi_rs::Client, String> {
    builder(YTM_HOST, CONNECT_TIMEOUT)
        .build()
        .map(ytmapi_rs::Client::new_from_reqwest_client)
        .map_err(|e| format!("Could not set up the YouTube Music connection: {e}"))
}

/// Retries only failed *connects*. Nothing of the request has left by then,
/// so it's safe for any query, POSTs included; once a request is out, a
/// replay could repeat whatever it did, and a status is YouTube's answer
/// rather than a fault on the line. reqwest retries at once, on a fresh
/// connection, inside a budget (20% extra load) that stops a dead network
/// turning into a retry storm.
///
/// The classifier checks the host itself, on top of `for_host`: reqwest 0.13
/// only consults the scope from the *second* retry on, so an out-of-scope
/// request still got one replay (caught by `other_hosts_are_not_retried`).
fn builder(host: &'static str, connect_timeout: Duration) -> reqwest::ClientBuilder {
    let retry = reqwest::retry::for_host(host)
        .max_retries_per_request(CONNECT_RETRIES)
        .classify_fn(move |attempt| {
            let in_scope = attempt.uri().host() == Some(host);
            let failed_to_connect = attempt
                .error()
                .and_then(|e| e.downcast_ref::<reqwest::Error>())
                .is_some_and(reqwest::Error::is_connect);
            if in_scope && failed_to_connect {
                attempt.retryable()
            } else {
                attempt.success()
            }
        });
    reqwest::Client::builder()
        .http2_max_header_list_size(MAX_HEADER_LIST)
        .connect_timeout(connect_timeout)
        .read_timeout(READ_TIMEOUT)
        .retry(retry)
}

/// What a failed session build means to the user.
///
/// ytmapi-rs only ever makes its `Web` kind from a reqwest error (it never
/// looks at the status), so `Web` is exactly "the request didn't make it" —
/// the network, not YouTube turning the cookies down. Saying "rejected the
/// session" there sends people to sign in again, which can't help.
pub(crate) fn session_error(e: ytmapi_rs::Error) -> String {
    match e.into_kind() {
        ytmapi_rs::error::ErrorKind::Web { message } => unreachable_message(&message),
        other => format!("YouTube Music rejected the session: {other}"),
    }
}

/// The user's side of a request that never got a YouTube Music answer.
///
/// One case gets its own words: Google's "unusual traffic" check, which
/// redirects to `google.com/sorry/…` and then loops (reqwest reports "too
/// many redirects"). It's about the network's address, not this machine's
/// connection — common behind carrier-grade NAT, where many users share one
/// IP — so "check your internet" would send people the wrong way.
fn unreachable_message(transport_error: &str) -> String {
    if transport_error.contains("google.com/sorry") {
        "YouTube has paused requests from this network for a while (it suspects automated \
         traffic). Wait a few minutes, then try again."
            .to_string()
    } else {
        "Couldn't reach YouTube Music. Check your internet connection, then try again.".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// A server that accepts every connection and counts it, then hands the
    /// socket to `serve`.
    async fn counting_server<F, Fut>(serve: F) -> (u16, Arc<AtomicUsize>)
    where
        F: Fn(tokio::net::TcpStream) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let hits = Arc::new(AtomicUsize::new(0));
        let counted = hits.clone();
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                counted.fetch_add(1, Ordering::SeqCst);
                serve(socket).await;
            }
        });
        (port, hits)
    }

    /// The failure the app hit on a lossy link: the far end drops the
    /// connection mid-TLS-handshake (`tls handshake eof`). Nothing of the
    /// request was sent, so it's retried — on a fresh connection each time.
    #[tokio::test]
    async fn a_connection_dropped_during_the_handshake_is_retried() {
        let (port, hits) = counting_server(|socket| async move { drop(socket) }).await;
        let client = builder("127.0.0.1", CONNECT_TIMEOUT).build().unwrap();

        let err = client
            .get(format!("https://127.0.0.1:{port}/"))
            .send()
            .await
            .expect_err("every handshake is cut off");

        assert!(err.is_connect(), "expected a connect failure, got {err:?}");
        assert_eq!(hits.load(Ordering::SeqCst), 1 + CONNECT_RETRIES as usize);
    }

    /// A request that reached the server is never replayed: it may not be
    /// safe to send twice, and a status is YouTube's answer, not a fault on
    /// the line.
    #[tokio::test]
    async fn a_response_is_not_retried() {
        let (port, hits) = counting_server(|mut socket| async move {
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;
            let _ = socket
                .write_all(
                    b"HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                )
                .await;
        })
        .await;
        let client = builder("127.0.0.1", CONNECT_TIMEOUT).build().unwrap();

        let resp = client
            .get(format!("http://127.0.0.1:{port}/"))
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), 503);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    /// Retries are scoped to YouTube Music; nothing else this client might
    /// one day carry gets replayed by accident.
    #[tokio::test]
    async fn other_hosts_are_not_retried() {
        let (port, hits) = counting_server(|socket| async move { drop(socket) }).await;
        let client = builder(YTM_HOST, CONNECT_TIMEOUT).build().unwrap();

        let _ = client
            .get(format!("https://127.0.0.1:{port}/"))
            .send()
            .await
            .expect_err("every handshake is cut off");

        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    /// The connect budget covers the TLS handshake too, not just TCP: a
    /// server that accepts and then never speaks is given up on, rather
    /// than waited on for as long as the socket lives.
    #[tokio::test]
    async fn a_hung_handshake_times_out() {
        // Production's 15 s × 3 attempts would make this a 45 s test.
        const HUNG_BUDGET: Duration = Duration::from_millis(300);
        let (port, _hits) = counting_server(|socket| async move {
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(600)).await;
                drop(socket);
            });
        })
        .await;
        let client = builder("127.0.0.1", HUNG_BUDGET).build().unwrap();

        let started = std::time::Instant::now();
        let err = client
            .get(format!("https://127.0.0.1:{port}/"))
            .send()
            .await
            .expect_err("the handshake never finishes");

        assert!(err.is_connect() && err.is_timeout(), "got {err:?}");
        // Every attempt spends the whole budget, so the bound is all of them.
        let ceiling = HUNG_BUDGET * (1 + CONNECT_RETRIES) + Duration::from_secs(2);
        assert!(started.elapsed() < ceiling, "took {:?}", started.elapsed());
    }

    /// The failure behind "YouTube Music rejected the session: Web error
    /// <error sending request for url (https://music.youtube.com/)>": the
    /// homepage now answers with ~18 KB of headers (a 7 KB
    /// `connection-allowlist-report-only` and a 6 KB CSP report-only, each
    /// listing every Google ccTLD), past hyper's 16 KB HTTP/2 default — so
    /// h2 reset the stream itself (`PROTOCOL_ERROR`) and every session build
    /// failed. Served here over h2c with the same shape and size.
    #[tokio::test]
    async fn a_response_with_youtubes_header_bloat_is_accepted() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut conn = h2::server::handshake(socket).await.unwrap();
            while let Some(Ok((_request, mut respond))) = conn.accept().await {
                let response = http::Response::builder()
                    .status(200)
                    .header("connection-allowlist-report-only", "a".repeat(7_139))
                    .header("content-security-policy-report-only", "b".repeat(6_155))
                    .header("content-security-policy", "c".repeat(4_000))
                    .body(())
                    .unwrap();
                respond.send_response(response, true).unwrap();
            }
        });
        let client = builder("127.0.0.1", CONNECT_TIMEOUT)
            .http2_prior_knowledge()
            .build()
            .unwrap();

        let resp = client
            .get(format!("http://127.0.0.1:{port}/"))
            .send()
            .await
            .expect("YouTube's headers must fit");

        assert_eq!(resp.status(), 200);
    }

    /// A transport failure is the network, not YouTube turning the cookies
    /// down — and must not read "not signed in", which the UI treats as
    /// signed out.
    #[test]
    fn a_network_failure_is_not_reported_as_a_rejected_session() {
        let message = session_error(a_transport_error());
        assert!(!message.contains("rejected"), "{message}");
        assert!(
            !message.to_lowercase().contains("not signed in"),
            "{message}"
        );
        assert!(
            message.contains("Couldn't reach YouTube Music"),
            "{message}"
        );
    }

    /// Google's bot check is the network's address, not the connection: the
    /// message must not send people to check their internet. The text is
    /// reqwest's, as seen live from a flagged address.
    #[test]
    fn googles_unusual_traffic_check_gets_its_own_message() {
        let message = unreachable_message(
            "error following redirect for url (https://www.google.com/sorry/index?\
             continue=https://music.youtube.com/&q=EgRm0W)",
        );
        assert!(
            message.contains("paused requests from this network"),
            "{message}"
        );
        assert!(!message.contains("internet connection"), "{message}");
    }

    /// Any other failure (an unparseable page, a missing SAPISID, …) is still
    /// a session problem.
    #[test]
    fn a_bad_session_is_still_reported_as_rejected() {
        let message = session_error(ytmapi_rs::Error::from(std::io::Error::other("boom")));
        assert!(
            message.starts_with("YouTube Music rejected the session"),
            "{message}"
        );
    }

    /// The session-building fetch of music.youtube.com, repeatedly, over the
    /// real network — the call that failed. Run it from a flaky link:
    /// `cargo test -p hm-ytmusic --lib net::tests::live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "requires network access"]
    async fn live_session_fetch_holds_up_on_a_lossy_link() {
        let mut slowest = Duration::ZERO;
        for i in 0..10 {
            let started = std::time::Instant::now();
            let built = ytmapi_rs::YtMusicBuilder::new_with_client(client().unwrap())
                .build()
                .await;
            let took = started.elapsed();
            slowest = slowest.max(took);
            println!(
                "{i}: {} in {took:?}",
                if built.is_ok() { "ok" } else { "FAILED" }
            );
            if let Err(e) = built {
                panic!("fetch {i} failed after {took:?}: {e}");
            }
        }
        println!("slowest: {slowest:?}");
    }

    /// ytmapi-rs only builds its `Web` kind from a reqwest error, so make a
    /// real one: nothing listens on port 1.
    fn a_transport_error() -> ytmapi_rs::Error {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let err = rt.block_on(async {
            reqwest::Client::new()
                .get("http://127.0.0.1:1/")
                .send()
                .await
                .expect_err("nothing listens on port 1")
        });
        ytmapi_rs::Error::from(err)
    }
}
