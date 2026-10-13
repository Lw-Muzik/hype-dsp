//! The client against a tiny in-process hub (tokio-tungstenite server on
//! 127.0.0.1): handshake, relaying, close-code reactions, backoff, tokens.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use hm_connect::protocol::*;
use hm_connect::{
    spawn, Backoff, ClientConfig, ClientEvent, ConnectHandle, ConnectionStatus, Renewal, SendError,
    StopReason, TokenProvider,
};
use serde_json::{json, Value};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

const DEVICE_ID: &str = "0b9a8c7d-6e5f-4a3b-8c2d-1e0f9a8b7c6d";
const STEP: Duration = Duration::from_secs(5);

// ------------------------------------------------------------------ harness

fn fixture(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../connect-protocol/fixtures")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn device() -> DeviceHello {
    DeviceHello {
        id: DEVICE_ID.into(),
        kind: DeviceKind::Desktop,
        name: "Bruno's MacBook".into(),
        platform: "macos".into(),
        app_version: "0.1.26".into(),
        caps: vec![Cap::Player, Cap::Yt, Cap::Local, Cap::Lyrics, Cap::Viz],
    }
}

/// A deterministic, fast schedule: 10 ms, 20 ms, 40 ms … capped at 80 ms.
fn config(url: &str) -> ClientConfig {
    let mut c = ClientConfig::new(url, device());
    c.backoff = Backoff {
        base: Duration::from_millis(10),
        cap: Duration::from_millis(80),
        jitter: 0.0,
    };
    c.welcome_timeout = Duration::from_secs(3);
    c.idle_timeout = Duration::from_secs(10);
    c.session_poll = Duration::from_millis(30);
    c
}

/// Scripted tokens: `current` is whatever was last set or renewed; `renew`
/// pops the next scripted outcome (default: mint `token-N`).
#[derive(Default)]
struct FakeTokens {
    current: Mutex<Option<String>>,
    script: Mutex<VecDeque<Renewal>>,
    rejected: Mutex<Vec<String>>,
}

impl FakeTokens {
    fn with(token: &str) -> Arc<Self> {
        let t = Arc::new(Self::default());
        t.set(Some(token));
        t
    }
    fn set(&self, token: Option<&str>) {
        *self.current.lock().unwrap() = token.map(str::to_string);
    }
    fn script(&self, outcome: Renewal) {
        self.script.lock().unwrap().push_back(outcome);
    }
    fn rejected(&self) -> Vec<String> {
        self.rejected.lock().unwrap().clone()
    }
}

impl TokenProvider for FakeTokens {
    fn current(&self) -> Option<String> {
        self.current.lock().unwrap().clone()
    }
    fn renew(&self, rejected: &str) -> Renewal {
        let n = {
            let mut r = self.rejected.lock().unwrap();
            r.push(rejected.to_string());
            r.len()
        };
        let outcome = self
            .script
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Renewal::Renewed(format!("token-{n}")));
        match &outcome {
            Renewal::Renewed(t) => self.set(Some(t)),
            Renewal::SessionGone => self.set(None),
            Renewal::Unavailable => {}
        }
        outcome
    }
}

struct Hub {
    listener: TcpListener,
    url: String,
}

impl Hub {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/api/connect/ws", listener.local_addr().unwrap());
        Self { listener, url }
    }

    async fn accept(&self) -> Conn {
        let (tcp, _) = timeout(STEP, self.listener.accept())
            .await
            .expect("the client never connected")
            .unwrap();
        Conn {
            ws: tokio_tungstenite::accept_async(tcp).await.unwrap(),
        }
    }

    async fn assert_no_connection(&self, within: Duration) {
        assert!(
            timeout(within, self.listener.accept()).await.is_err(),
            "the client reconnected but must not have"
        );
    }
}

struct Conn {
    ws: WebSocketStream<TcpStream>,
}

impl Conn {
    async fn message(&mut self) -> Message {
        timeout(STEP, self.ws.next())
            .await
            .expect("no message from the client")
            .expect("socket ended")
            .unwrap()
    }

    async fn json(&mut self) -> Value {
        loop {
            match self.message().await {
                Message::Text(t) => return serde_json::from_str(&t).unwrap(),
                Message::Pong(_) | Message::Ping(_) => continue,
                other => panic!("expected a text frame, got {other:?}"),
            }
        }
    }

    async fn hello(&mut self) -> (String, DeviceHello) {
        match serde_json::from_value::<Frame>(self.json().await).unwrap() {
            Frame::Hello { v, token, device } => {
                assert_eq!(v, 1);
                (token, device)
            }
            other => panic!("first frame must be hello, got {other:?}"),
        }
    }

    async fn send(&mut self, v: Value) {
        self.ws.send(Message::text(v.to_string())).await.unwrap();
    }

    async fn welcome(&mut self) {
        let mut w = fixture("welcome.json");
        w["deviceId"] = json!(DEVICE_ID);
        self.send(w).await;
    }

    async fn close(&mut self, code: u16) {
        let _ = self
            .ws
            .send(Message::Close(Some(CloseFrame {
                code: CloseCode::from(code),
                reason: "".into(),
            })))
            .await;
    }
}

async fn status(rx: &mut UnboundedReceiver<ClientEvent>) -> ConnectionStatus {
    loop {
        match timeout(STEP, rx.recv())
            .await
            .expect("no status")
            .expect("events ended")
        {
            ClientEvent::Status(s) => return s,
            ClientEvent::Frame(_) => continue,
        }
    }
}

async fn status_where(
    rx: &mut UnboundedReceiver<ClientEvent>,
    pred: impl Fn(&ConnectionStatus) -> bool,
) -> ConnectionStatus {
    loop {
        let s = status(rx).await;
        if pred(&s) {
            return s;
        }
    }
}

/// The next `Waiting` status as `(attempt, retry_in_ms)`.
async fn waiting(rx: &mut UnboundedReceiver<ClientEvent>) -> (u32, u64) {
    match status_where(rx, |s| matches!(s, ConnectionStatus::Waiting { .. })).await {
        ConnectionStatus::Waiting {
            attempt,
            retry_in_ms,
            ..
        } => (attempt, retry_in_ms),
        _ => unreachable!(),
    }
}

async fn frame(rx: &mut UnboundedReceiver<ClientEvent>) -> Frame {
    loop {
        match timeout(STEP, rx.recv())
            .await
            .expect("no frame")
            .expect("events ended")
        {
            ClientEvent::Frame(f) => return *f,
            ClientEvent::Status(_) => continue,
        }
    }
}

/// Spawn a client, accept its socket, check `hello`, send `welcome`.
async fn connected(
    hub: &Hub,
    tokens: Arc<FakeTokens>,
) -> (ConnectHandle, UnboundedReceiver<ClientEvent>, Conn) {
    connected_with(hub, tokens, config(&hub.url)).await
}

async fn connected_with(
    hub: &Hub,
    tokens: Arc<FakeTokens>,
    cfg: ClientConfig,
) -> (ConnectHandle, UnboundedReceiver<ClientEvent>, Conn) {
    let (handle, mut rx) = spawn(cfg, tokens, &tokio::runtime::Handle::current());
    let mut conn = hub.accept().await;
    conn.hello().await;
    conn.welcome().await;
    let s = status_where(&mut rx, |s| matches!(s, ConnectionStatus::Connected { .. })).await;
    assert_eq!(
        s,
        ConnectionStatus::Connected {
            device_id: DEVICE_ID.into()
        }
    );
    (handle, rx, conn)
}

fn frame_of(name: &str) -> Frame {
    serde_json::from_value(fixture(name)).unwrap()
}

// -------------------------------------------------------------------- tests

#[tokio::test]
async fn hello_announces_this_desktop_as_a_player() {
    let hub = Hub::start().await;
    let (_handle, _rx) = spawn(
        config(&hub.url),
        FakeTokens::with("token-0"),
        &tokio::runtime::Handle::current(),
    );
    let mut conn = hub.accept().await;
    let (token, dev) = conn.hello().await;
    assert_eq!(token, "token-0");
    assert_eq!(dev, device());
    assert_eq!(dev.kind, DeviceKind::Desktop);
    assert_eq!(
        dev.caps,
        vec![Cap::Player, Cap::Yt, Cap::Local, Cap::Lyrics, Cap::Viz]
    );
}

#[tokio::test]
async fn inbound_frames_are_forwarded_in_order() {
    let hub = Hub::start().await;
    let (_handle, mut rx, mut conn) = connected(&hub, FakeTokens::with("t")).await;
    assert!(matches!(frame(&mut rx).await, Frame::Welcome { .. }));

    conn.send(fixture("cmd.play.from.json")).await;
    conn.send(fixture("rpc.search.from.json")).await;
    // Not addressed to a player: dropped, never forwarded. (An `ack` is
    // forwarded since v1.2: it answers a cmd this desktop sent as a remote.)
    conn.send(fixture("ack.ok.json")).await;
    conn.send(fixture("viz.stamped.json")).await;
    conn.send(fixture("select.json")).await;
    // v1.1: the hub's subscriber edge and the clock's answer are the app's.
    conn.send(fixture("viz_sub.player.on.json")).await;
    conn.send(fixture("pong.json")).await;
    conn.send(fixture("viz_sub.player.off.json")).await;
    conn.send(fixture("rpc.lyrics.from.json")).await;
    conn.send(fixture("devices.json")).await;
    conn.send(fixture("state.stamped.json")).await;
    conn.send(fixture("error.json")).await;

    assert_eq!(frame(&mut rx).await, frame_of("cmd.play.from.json"));
    assert_eq!(frame(&mut rx).await, frame_of("rpc.search.from.json"));
    assert_eq!(frame(&mut rx).await, frame_of("ack.ok.json"));
    assert_eq!(frame(&mut rx).await, frame_of("viz_sub.player.on.json"));
    assert_eq!(frame(&mut rx).await, frame_of("pong.json"));
    assert_eq!(frame(&mut rx).await, frame_of("viz_sub.player.off.json"));
    assert_eq!(frame(&mut rx).await, frame_of("rpc.lyrics.from.json"));
    assert_eq!(frame(&mut rx).await, frame_of("devices.json"));
    assert_eq!(frame(&mut rx).await, frame_of("state.stamped.json"));
    assert_eq!(frame(&mut rx).await, frame_of("error.json"));
}

#[tokio::test]
async fn outbound_frames_reach_the_hub_verbatim() {
    let hub = Hub::start().await;
    let (handle, _rx, mut conn) = connected(&hub, FakeTokens::with("t")).await;

    for name in [
        "ack.ok.json",
        "ack.error.json",
        // v1.2: a player acting as a remote.
        "cmd.play.to.json",
        "select.json",
        "state.player.json",
        "rpc_result.search.json",
        "rpc_result.lyrics.json",
        "rpc_result.lyrics.none.json",
        // v1.1: the clock probe and the visualiser stream.
        "ping.json",
        "viz.player.json",
    ] {
        handle.send(&frame_of(name)).unwrap();
        assert_eq!(conn.json().await, fixture(name), "{name}");
    }
}

#[tokio::test]
async fn a_player_cannot_send_controller_or_hub_frames() {
    let hub = Hub::start().await;
    let (handle, _rx, _conn) = connected(&hub, FakeTokens::with("t")).await;
    for (name, tag) in [
        ("hello.mobile.json", "hello"),
        // A `cmd` carries `to` outbound; one with the hub's `from` is forged.
        ("cmd.play.from.json", "cmd"),
        ("rpc.search.to.json", "rpc"),
        ("pong.json", "pong"),
        // A player is told `viz_sub`, never sends it; `viz` fans out from the hub.
        ("viz_sub.player.on.json", "viz_sub"),
        ("viz_sub.to.json", "viz_sub"),
        ("viz.stamped.json", "viz"),
        // A stamped state is the hub's; a player never names itself or stamps.
        ("state.stamped.json", "state"),
    ] {
        assert_eq!(
            handle.send(&frame_of(name)),
            Err(SendError::NotAllowed(tag)),
            "{name}"
        );
    }
}

#[tokio::test]
async fn sending_before_welcome_or_after_a_drop_is_refused() {
    let hub = Hub::start().await;
    let (handle, _rx) = spawn(
        config(&hub.url),
        FakeTokens::with("t"),
        &tokio::runtime::Handle::current(),
    );
    let mut conn = hub.accept().await;
    conn.hello().await;
    assert_eq!(
        handle.send(&frame_of("ack.ok.json")),
        Err(SendError::NotConnected)
    );

    conn.welcome().await;
    // Wait until the client has processed the welcome.
    let mut ok = false;
    for _ in 0..100 {
        if handle.send(&frame_of("ack.ok.json")).is_ok() {
            ok = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(ok, "send never became possible after welcome");

    conn.close(4000).await;
    drop(conn);
    let mut refused = false;
    for _ in 0..100 {
        if handle.send(&frame_of("ack.ok.json")) == Err(SendError::NotConnected) {
            refused = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        refused,
        "frames were still accepted after the socket closed"
    );
}

#[tokio::test]
async fn oversized_frames_are_refused() {
    let hub = Hub::start().await;
    let (handle, _rx, _conn) = connected(&hub, FakeTokens::with("t")).await;
    let big = Frame::RpcResult {
        id: "r".into(),
        ok: true,
        result: Some(json!({ "blob": "x".repeat(MAX_FRAME_BYTES) })),
        error: None,
    };
    assert!(matches!(handle.send(&big), Err(SendError::TooLarge(_))));
}

#[tokio::test]
async fn the_handle_remembers_the_hubs_visualiser_subscription() {
    let hub = Hub::start().await;
    let (handle, mut rx, mut conn) = connected(&hub, FakeTokens::with("t")).await;
    assert!(matches!(frame(&mut rx).await, Frame::Welcome { .. }));
    assert!(!handle.viz_subscribed(), "nobody subscribed yet");

    conn.send(fixture("viz_sub.player.on.json")).await;
    assert_eq!(frame(&mut rx).await, frame_of("viz_sub.player.on.json"));
    assert!(handle.viz_subscribed());

    conn.send(fixture("viz_sub.player.off.json")).await;
    assert_eq!(frame(&mut rx).await, frame_of("viz_sub.player.off.json"));
    assert!(!handle.viz_subscribed());

    // A standing subscription does not survive its socket: a new one starts
    // clean, and only the hub's post-welcome `viz_sub` brings it back.
    conn.send(fixture("viz_sub.player.on.json")).await;
    assert_eq!(frame(&mut rx).await, frame_of("viz_sub.player.on.json"));
    conn.close(4000).await;
    drop(conn);
    let mut conn = hub.accept().await;
    conn.hello().await;
    assert!(
        !handle.viz_subscribed(),
        "the old socket's subscription is gone"
    );
    conn.welcome().await;
    conn.send(fixture("viz_sub.player.on.json")).await;
    for _ in 0..200 {
        if handle.viz_subscribed() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the repeated subscription was never remembered");
}

fn viz_frame() -> Frame {
    frame_of("viz.player.json")
}

#[tokio::test]
async fn a_viz_frame_off_contract_is_refused_before_it_is_sent() {
    let hub = Hub::start().await;
    let (handle, _rx, _conn) = connected(&hub, FakeTokens::with("t")).await;
    let Frame::Viz { frame, .. } = viz_frame() else {
        unreachable!()
    };
    let mut short = frame.clone();
    short.bands.truncate(31);
    assert_eq!(
        handle.send(&Frame::Viz {
            device_id: None,
            frame: short
        }),
        Err(SendError::BadViz)
    );
    let mut loud = frame;
    loud.level = 1.5;
    assert_eq!(
        handle.send(&Frame::Viz {
            device_id: None,
            frame: loud
        }),
        Err(SendError::BadViz)
    );
}

#[tokio::test]
async fn the_longest_valid_viz_frame_still_fits_one_kib() {
    let hub = Hub::start().await;
    let (handle, _rx, mut conn) = connected(&hub, FakeTokens::with("t")).await;
    let Frame::Viz { mut frame, .. } = viz_frame() else {
        unreachable!()
    };
    // Unrounded f64s in scientific notation are the longest a band can print.
    frame.bands = vec![1.234_567_890_123_456_7e-5; 32];
    frame.level = 1.234_567_890_123_456_7e-5;
    frame.beat = 1.234_567_890_123_456_7e-5;
    frame.heard_at = u64::MAX;
    frame.position_ms = u64::MAX;
    let frame = Frame::Viz {
        device_id: None,
        frame,
    };
    let text = serde_json::to_string(&frame).unwrap();
    assert!(
        text.len() <= MAX_VIZ_FRAME_BYTES,
        "{} bytes > 1 KiB",
        text.len()
    );
    handle.send(&frame).unwrap();
    assert_eq!(conn.json().await["t"], "viz");
}

#[tokio::test]
async fn viz_frames_beyond_thirty_a_second_are_refused() {
    let hub = Hub::start().await;
    let (handle, _rx, _conn) = connected(&hub, FakeTokens::with("t")).await;
    let start = std::time::Instant::now();
    let mut accepted = 0;
    let mut limited = 0;
    for _ in 0..45 {
        match handle.send(&viz_frame()) {
            Ok(()) => accepted += 1,
            Err(SendError::VizRateLimited) => limited += 1,
            Err(other) => panic!("{other:?}"),
        }
    }
    assert!(
        start.elapsed() < Duration::from_millis(900),
        "the burst must fit one window for this assertion"
    );
    assert_eq!(
        (accepted, limited),
        (MAX_VIZ_PER_SECOND, 45 - MAX_VIZ_PER_SECOND)
    );
    // Other frames have their own budget: an ack still goes.
    handle.send(&frame_of("ack.ok.json")).unwrap();
}

#[tokio::test]
async fn protocol_pings_are_answered() {
    let hub = Hub::start().await;
    let (_handle, _rx, mut conn) = connected(&hub, FakeTokens::with("t")).await;
    conn.ws
        .send(Message::Ping(b"hm-ping".to_vec().into()))
        .await
        .unwrap();
    loop {
        match conn.message().await {
            Message::Pong(p) => {
                assert_eq!(&p[..], b"hm-ping");
                break;
            }
            Message::Text(_) => continue,
            other => panic!("expected a pong, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn token_rejected_4001_renews_and_reconnects_with_the_new_token() {
    let hub = Hub::start().await;
    let tokens = FakeTokens::with("token-0");
    let (_handle, _rx) = spawn(
        config(&hub.url),
        tokens.clone(),
        &tokio::runtime::Handle::current(),
    );
    let mut first = hub.accept().await;
    assert_eq!(first.hello().await.0, "token-0");
    first.close(4001).await;

    let mut second = hub.accept().await;
    assert_eq!(second.hello().await.0, "token-1");
    assert_eq!(tokens.rejected(), vec!["token-0".to_string()]);
}

#[tokio::test]
async fn account_merged_4005_renews_and_reconnects() {
    let hub = Hub::start().await;
    let tokens = FakeTokens::with("token-0");
    let (_handle, _rx, mut conn) = connected(&hub, tokens.clone()).await;
    conn.close(4005).await;

    let mut next = hub.accept().await;
    assert_eq!(next.hello().await.0, "token-1");
    assert_eq!(tokens.rejected(), vec!["token-0".to_string()]);
}

#[tokio::test]
async fn ordinary_closes_reconnect_without_touching_the_token() {
    let hub = Hub::start().await;
    let tokens = FakeTokens::with("token-0");
    let (_handle, _rx, mut conn) = connected(&hub, tokens.clone()).await;
    for code in [4000u16, 4002, 1000, 1011] {
        conn.close(code).await;
        conn = hub.accept().await;
        assert_eq!(conn.hello().await.0, "token-0", "after {code}");
        conn.welcome().await;
    }
    // Dropped without a close frame at all.
    drop(conn);
    let mut conn = hub.accept().await;
    assert_eq!(conn.hello().await.0, "token-0");
    assert!(
        tokens.rejected().is_empty(),
        "no close here asks for a new token"
    );
}

#[tokio::test]
async fn unlinked_4003_stops_and_never_reconnects() {
    let hub = Hub::start().await;
    let (handle, mut rx, mut conn) = connected(&hub, FakeTokens::with("t")).await;
    conn.close(4003).await;
    let s = status_where(&mut rx, |s| matches!(s, ConnectionStatus::Stopped { .. })).await;
    assert_eq!(
        s,
        ConnectionStatus::Stopped {
            reason: StopReason::Unlinked
        }
    );
    hub.assert_no_connection(Duration::from_millis(300)).await;
    assert!(handle.is_finished());
    assert_eq!(
        handle.status(),
        ConnectionStatus::Stopped {
            reason: StopReason::Unlinked
        }
    );
}

#[tokio::test]
async fn replaced_4004_stops_and_never_reconnects() {
    let hub = Hub::start().await;
    let (handle, mut rx, mut conn) = connected(&hub, FakeTokens::with("t")).await;
    conn.close(4004).await;
    let s = status_where(&mut rx, |s| matches!(s, ConnectionStatus::Stopped { .. })).await;
    assert_eq!(
        s,
        ConnectionStatus::Stopped {
            reason: StopReason::Replaced
        }
    );
    hub.assert_no_connection(Duration::from_millis(300)).await;
    assert!(handle.is_finished());
}

#[tokio::test]
async fn backoff_grows_until_a_welcome_then_starts_over() {
    let hub = Hub::start().await;
    let (_handle, mut rx) = spawn(
        config(&hub.url),
        FakeTokens::with("t"),
        &tokio::runtime::Handle::current(),
    );

    // Two sockets rejected before any welcome: 10 ms, then 20 ms.
    let mut c = hub.accept().await;
    c.hello().await;
    c.close(4000).await;
    assert_eq!(waiting(&mut rx).await, (1, 10));
    let mut c = hub.accept().await;
    c.hello().await;
    c.close(4000).await;
    assert_eq!(waiting(&mut rx).await, (2, 20));

    // A welcomed socket resets the schedule.
    let mut c = hub.accept().await;
    c.hello().await;
    c.welcome().await;
    status_where(&mut rx, |s| matches!(s, ConnectionStatus::Connected { .. })).await;
    c.close(4000).await;
    assert_eq!(waiting(&mut rx).await, (1, 10));
}

#[tokio::test]
async fn backoff_is_capped() {
    let hub = Hub::start().await;
    let (_handle, mut rx) = spawn(
        config(&hub.url),
        FakeTokens::with("t"),
        &tokio::runtime::Handle::current(),
    );
    let mut delays = Vec::new();
    for _ in 0..6 {
        let mut c = hub.accept().await;
        c.hello().await;
        c.close(4000).await;
        if let ConnectionStatus::Waiting { retry_in_ms, .. } =
            status_where(&mut rx, |s| matches!(s, ConnectionStatus::Waiting { .. })).await
        {
            delays.push(retry_in_ms);
        }
    }
    assert_eq!(delays, vec![10, 20, 40, 80, 80, 80]);
}

#[tokio::test]
async fn an_unknown_command_or_method_is_answered_unsupported() {
    let hub = Hub::start().await;
    let (_handle, _rx, mut conn) = connected(&hub, FakeTokens::with("t")).await;
    conn.send(json!({"t": "cmd", "id": "c-77", "from": "w", "cmd": {"type": "dance"}}))
        .await;
    assert_eq!(
        conn.json().await,
        json!({"t": "ack", "id": "c-77", "ok": false, "error": "unsupported"})
    );
    conn.send(json!({"t": "rpc", "id": "r-77", "from": "w", "method": "karaoke", "params": {}}))
        .await;
    assert_eq!(
        conn.json().await,
        json!({"t": "rpc_result", "id": "r-77", "ok": false, "error": "unsupported"})
    );
}

fn jwt_expiring_in(secs: u64) -> String {
    use base64::Engine as _;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let enc = |v: Value| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v.to_string());
    format!(
        "{}.{}.sig",
        enc(json!({"alg": "HS256"})),
        enc(json!({"exp": now + secs}))
    )
}

#[tokio::test]
async fn a_token_about_to_expire_is_renewed_before_connecting() {
    let hub = Hub::start().await;
    let stale = jwt_expiring_in(10);
    let tokens = FakeTokens::with(&stale);
    let (_handle, _rx) = spawn(
        config(&hub.url),
        tokens.clone(),
        &tokio::runtime::Handle::current(),
    );
    let mut conn = hub.accept().await;
    assert_eq!(conn.hello().await.0, "token-1");
    assert_eq!(tokens.rejected(), vec![stale]);
}

#[tokio::test]
async fn a_fresh_token_is_used_as_is() {
    let hub = Hub::start().await;
    let fresh = jwt_expiring_in(3600);
    let tokens = FakeTokens::with(&fresh);
    let (_handle, _rx) = spawn(
        config(&hub.url),
        tokens.clone(),
        &tokio::runtime::Handle::current(),
    );
    let mut conn = hub.accept().await;
    assert_eq!(conn.hello().await.0, fresh);
    assert!(tokens.rejected().is_empty());
}

#[tokio::test]
async fn without_a_session_it_waits_then_connects_when_one_appears() {
    let hub = Hub::start().await;
    let tokens = Arc::new(FakeTokens::default());
    let (_handle, mut rx) = spawn(
        config(&hub.url),
        tokens.clone(),
        &tokio::runtime::Handle::current(),
    );
    assert_eq!(
        status_where(&mut rx, |s| matches!(s, ConnectionStatus::NoSession)).await,
        ConnectionStatus::NoSession
    );
    hub.assert_no_connection(Duration::from_millis(150)).await;

    tokens.set(Some("late-session"));
    let mut conn = hub.accept().await;
    assert_eq!(conn.hello().await.0, "late-session");
}

#[tokio::test]
async fn a_lost_session_after_4001_waits_instead_of_hammering_the_hub() {
    let hub = Hub::start().await;
    let tokens = FakeTokens::with("token-0");
    tokens.script(Renewal::SessionGone);
    let (_handle, mut rx) = spawn(
        config(&hub.url),
        tokens.clone(),
        &tokio::runtime::Handle::current(),
    );
    let mut conn = hub.accept().await;
    conn.hello().await;
    status_where(&mut rx, |s| {
        matches!(s, ConnectionStatus::Connecting { .. })
    })
    .await;
    conn.close(4001).await;
    // Straight to "no session": a gone session is not a retry countdown.
    assert_eq!(status(&mut rx).await, ConnectionStatus::NoSession);
    hub.assert_no_connection(Duration::from_millis(150)).await;

    tokens.set(Some("bootstrapped"));
    let mut conn = hub.accept().await;
    assert_eq!(conn.hello().await.0, "bootstrapped");
}

#[tokio::test]
async fn an_unavailable_renewal_backs_off_and_retries() {
    let hub = Hub::start().await;
    let tokens = FakeTokens::with("token-0");
    tokens.script(Renewal::Unavailable);
    let (_handle, _rx) = spawn(
        config(&hub.url),
        tokens.clone(),
        &tokio::runtime::Handle::current(),
    );
    let mut conn = hub.accept().await;
    conn.hello().await;
    conn.close(4001).await;
    // Renewal failed transiently: the old token is tried again, then renewed.
    let mut conn = hub.accept().await;
    assert_eq!(conn.hello().await.0, "token-0");
    conn.close(4001).await;
    let mut conn = hub.accept().await;
    assert_eq!(conn.hello().await.0, "token-2");
}

#[tokio::test]
async fn shutdown_closes_the_socket_normally() {
    let hub = Hub::start().await;
    let (handle, mut rx, mut conn) = connected(&hub, FakeTokens::with("t")).await;
    handle.shutdown().await;
    loop {
        match conn.message().await {
            Message::Close(Some(f)) => {
                assert_eq!(f.code, CloseCode::Normal);
                break;
            }
            Message::Text(_) | Message::Pong(_) => continue,
            other => panic!("expected a close, got {other:?}"),
        }
    }
    assert_eq!(
        status_where(&mut rx, |s| matches!(s, ConnectionStatus::Stopped { .. })).await,
        ConnectionStatus::Stopped {
            reason: StopReason::Shutdown
        }
    );
    hub.assert_no_connection(Duration::from_millis(150)).await;
}

#[tokio::test]
async fn dropping_the_handle_stops_the_client() {
    let hub = Hub::start().await;
    let (handle, _rx, mut conn) = connected(&hub, FakeTokens::with("t")).await;
    drop(handle);
    loop {
        match conn.message().await {
            Message::Close(_) => break,
            Message::Text(_) | Message::Pong(_) => continue,
            other => panic!("expected a close, got {other:?}"),
        }
    }
    hub.assert_no_connection(Duration::from_millis(150)).await;
}

#[tokio::test]
async fn a_silent_hub_is_presumed_dead_and_redialed() {
    let hub = Hub::start().await;
    let mut cfg = config(&hub.url);
    cfg.idle_timeout = Duration::from_millis(250);
    let (_handle, mut rx, _conn) = connected_with(&hub, FakeTokens::with("t"), cfg).await;
    match status_where(&mut rx, |s| matches!(s, ConnectionStatus::Waiting { .. })).await {
        ConnectionStatus::Waiting { reason, .. } => assert!(reason.contains("silent"), "{reason}"),
        _ => unreachable!(),
    }
    let mut again = hub.accept().await;
    again.hello().await;
}

#[tokio::test]
async fn a_chatty_hub_is_not_mistaken_for_a_silent_one() {
    let hub = Hub::start().await;
    let mut cfg = config(&hub.url);
    cfg.idle_timeout = Duration::from_millis(300);
    let (_handle, mut rx, mut conn) = connected_with(&hub, FakeTokens::with("t"), cfg).await;
    // Traffic every 100 ms for over 3× the idle timeout keeps the socket.
    for _ in 0..10 {
        conn.ws
            .send(Message::Ping(b"keep".to_vec().into()))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    hub.assert_no_connection(Duration::from_millis(10)).await;
    while let Ok(ev) = rx.try_recv() {
        assert!(
            !matches!(ev, ClientEvent::Status(ConnectionStatus::Waiting { .. })),
            "a live socket was dropped: {ev:?}"
        );
    }
}

#[tokio::test]
async fn a_hub_that_never_welcomes_is_redialed() {
    let hub = Hub::start().await;
    let mut cfg = config(&hub.url);
    cfg.welcome_timeout = Duration::from_millis(200);
    let (_handle, mut rx) = spawn(
        cfg,
        FakeTokens::with("t"),
        &tokio::runtime::Handle::current(),
    );
    let mut conn = hub.accept().await;
    conn.hello().await;
    match status_where(&mut rx, |s| matches!(s, ConnectionStatus::Waiting { .. })).await {
        ConnectionStatus::Waiting { reason, .. } => assert!(reason.contains("welcome"), "{reason}"),
        _ => unreachable!(),
    }
    let mut again = hub.accept().await;
    again.hello().await;
}

#[tokio::test]
async fn an_unreachable_hub_is_retried() {
    // Bind then drop: the port refuses connections.
    let addr = {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        l.local_addr().unwrap()
    };
    let (_handle, mut rx) = spawn(
        config(&format!("ws://{addr}/api/connect/ws")),
        FakeTokens::with("t"),
        &tokio::runtime::Handle::current(),
    );
    let first = status_where(&mut rx, |s| matches!(s, ConnectionStatus::Waiting { .. })).await;
    let second = status_where(&mut rx, |s| matches!(s, ConnectionStatus::Waiting { .. })).await;
    match (first, second) {
        (
            ConnectionStatus::Waiting {
                attempt: 1, reason, ..
            },
            ConnectionStatus::Waiting { attempt: 2, .. },
        ) => assert!(reason.contains("connect"), "{reason}"),
        other => panic!("{other:?}"),
    }
}
