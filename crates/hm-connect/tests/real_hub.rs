//! End-to-end against a REAL hub (`hypemuzik-management`), not the in-process
//! fake in `client.rs`. Ignored by default; run it with the hub up:
//!
//! ```sh
//! HM_CONNECT_REAL_API=http://localhost:4000/api \
//!   cargo test -p hm-connect --test real_hub -- --ignored --nocapture
//! # or the production hub (every run mints one throw-away anonymous account):
//! HM_CONNECT_REAL_API=https://api.hypemuzik.com/api \
//!   cargo test -p hm-connect --test real_hub -- --ignored --nocapture
//! ```
//!
//! The desktop is the crate's client; the "web controller" is a hand-driven
//! socket that links the way a browser does (create link, desktop approves the
//! code, claim), then commands the desktop, asks it for lyrics and subscribes
//! to its visualiser (v1.1). `http://`/`ws://` for a local hub and
//! `https://`/`wss://` for production; the test carries its own tiny HTTP
//! client (rustls with the OS verifier, like the crate) instead of pulling one
//! into the crate.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use hm_connect::protocol::*;
use hm_connect::rest::ws_url;
use hm_connect::{
    spawn, Cap, ClientConfig, ClientEvent, ConnectionStatus, DeviceKind, Renewal, TokenProvider,
};
use rustls_platform_verifier::BuilderVerifierExt;
use serde_json::{json, Value};
use tokio::net::TcpStream as TokioTcp;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{Connector, MaybeTlsStream, WebSocketStream};

const STEP: Duration = Duration::from_secs(10);

// ------------------------------------------------------------- tiny HTTP

struct Http {
    tls: bool,
    host: String,
    port: u16,
    prefix: String,
}

fn tls_config() -> Arc<rustls::ClientConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    Arc::new(
        rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_platform_verifier()
            .unwrap()
            .with_no_client_auth(),
    )
}

impl Http {
    fn new(api: &str) -> Self {
        let (tls, rest) = match (api.strip_prefix("https://"), api.strip_prefix("http://")) {
            (Some(r), _) => (true, r),
            (None, Some(r)) => (false, r),
            _ => panic!("HM_CONNECT_REAL_API must be an http:// or https:// URL"),
        };
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let default_port = if tls { "443" } else { "80" };
        let (host, port) = authority
            .split_once(':')
            .unwrap_or((authority, default_port));
        Self {
            tls,
            host: host.into(),
            port: port.parse().unwrap(),
            prefix: format!("/{}", path.trim_matches('/')),
        }
    }

    /// One request, `Connection: close`. Returns (status, JSON body).
    fn call(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> (u16, Value) {
        let body = body.map(|b| b.to_string());
        let mut req = format!(
            "{method} {}{path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAccept: application/json\r\n",
            self.prefix, self.host
        );
        if let Some(t) = token {
            req += &format!("Authorization: Bearer {t}\r\n");
        }
        if let Some(b) = &body {
            req += &format!(
                "Content-Type: application/json\r\nContent-Length: {}\r\n",
                b.len()
            );
        }
        req += "\r\n";
        if let Some(b) = &body {
            req += b;
        }
        let tcp = TcpStream::connect((self.host.as_str(), self.port)).expect("hub unreachable");
        tcp.set_read_timeout(Some(STEP)).unwrap();
        tcp.set_write_timeout(Some(STEP)).unwrap();
        let mut raw = Vec::new();
        if self.tls {
            let name = rustls::pki_types::ServerName::try_from(self.host.clone()).unwrap();
            let conn = rustls::ClientConnection::new(tls_config(), name).unwrap();
            let mut s = rustls::StreamOwned::new(conn, tcp);
            s.write_all(req.as_bytes()).unwrap();
            // A server that closes without close_notify surfaces as an error
            // after the body; what was read until then is the response.
            let _ = s.read_to_end(&mut raw);
        } else {
            let mut s = tcp;
            s.write_all(req.as_bytes()).unwrap();
            s.read_to_end(&mut raw).unwrap();
        }
        let split = raw
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("no header end")
            + 4;
        let head = String::from_utf8_lossy(&raw[..split]).to_string();
        let status: u16 = head.split_whitespace().nth(1).unwrap().parse().unwrap();
        let chunked = head
            .to_ascii_lowercase()
            .contains("transfer-encoding: chunked");
        let payload = if chunked {
            dechunk(&raw[split..])
        } else {
            raw[split..].to_vec()
        };
        let json = if payload.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&payload)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&payload).into()))
        };
        (status, json)
    }
}

fn dechunk(mut b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let eol = b.windows(2).position(|w| w == b"\r\n").expect("chunk size");
        let size =
            usize::from_str_radix(std::str::from_utf8(&b[..eol]).unwrap().trim(), 16).unwrap();
        b = &b[eol + 2..];
        if size == 0 {
            return out;
        }
        out.extend_from_slice(&b[..size]);
        b = &b[size + 2..];
    }
}

/// A random RFC 4122 v4 UUID.
fn uuid_v4() -> String {
    let mut b: [u8; 16] = std::array::from_fn(|_| fastrand::u8(..));
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

// ------------------------------------------------------------- tokens

struct Tokens(Mutex<String>);

impl TokenProvider for Tokens {
    fn current(&self) -> Option<String> {
        Some(self.0.lock().unwrap().clone())
    }
    fn renew(&self, _rejected: &str) -> Renewal {
        // A fresh anonymous account would be a different user; the test never
        // runs long enough to need a refresh.
        Renewal::Unavailable
    }
}

// ------------------------------------------------------------- web side

type Ws = WebSocketStream<MaybeTlsStream<TokioTcp>>;

/// A browser-like socket to `url` (TLS for `wss://`).
async fn open_ws(url: &str) -> Ws {
    let connector = url
        .starts_with("wss://")
        .then(|| Connector::Rustls(tls_config()));
    tokio_tungstenite::connect_async_tls_with_config(url, None, true, connector)
        .await
        .expect("web ws")
        .0
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

async fn next_json(ws: &mut Ws) -> Value {
    loop {
        match timeout(STEP, ws.next())
            .await
            .expect("web socket silent")
            .expect("closed")
            .unwrap()
        {
            Message::Text(t) => return serde_json::from_str(&t).unwrap(),
            Message::Close(c) => panic!("hub closed the web socket: {c:?}"),
            _ => continue,
        }
    }
}

/// Next frame with `t == tag` (skipping the broadcast chatter between).
async fn next_of(ws: &mut Ws, tag: &str) -> Value {
    loop {
        let v = next_json(ws).await;
        if v["t"] == tag {
            return v;
        }
    }
}

async fn desktop_frame(
    rx: &mut UnboundedReceiver<ClientEvent>,
    pick: impl Fn(&Frame) -> bool,
) -> Frame {
    loop {
        match timeout(STEP, rx.recv())
            .await
            .expect("no event from the client")
            .expect("ended")
        {
            ClientEvent::Frame(f) if pick(&f) => return *f,
            _ => continue,
        }
    }
}

// ----------------------------------------------------------------- test

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs a running hub: set HM_CONNECT_REAL_API=http://localhost:4000/api"]
async fn desktop_is_commanded_from_a_linked_web_controller() {
    let api = std::env::var("HM_CONNECT_REAL_API")
        .expect("set HM_CONNECT_REAL_API=http://localhost:4000/api");
    let http = Arc::new(Http::new(&api));

    // 1. The desktop's anonymous account. The body is exactly these three
    //    fields: the backend rejects anything extra.
    let install_id = uuid_v4();
    let (status, account) = http.call(
        "POST",
        "/auth/anonymous",
        None,
        Some(json!({ "installId": install_id, "os": "macos", "appVersion": "0.1.26-test" })),
    );
    assert!(
        status == 200 || status == 201,
        "anonymous bootstrap: {status} {account}"
    );
    let desktop_token = account["accessToken"]
        .as_str()
        .expect("accessToken")
        .to_string();

    // 2. Connect as a desktop player.
    let device = DeviceHello {
        id: install_id.clone(),
        kind: DeviceKind::Desktop,
        name: "hm-connect real-hub test".into(),
        platform: "macos".into(),
        app_version: "0.1.26-test".into(),
        caps: vec![Cap::Player, Cap::Yt, Cap::Local, Cap::Lyrics, Cap::Viz],
    };
    let url = ws_url(&api).unwrap();
    let (handle, mut rx) = spawn(
        ClientConfig::new(&url, device.clone()),
        Arc::new(Tokens(Mutex::new(desktop_token.clone()))),
        &tokio::runtime::Handle::current(),
    );
    let welcome = desktop_frame(&mut rx, |f| matches!(f, Frame::Welcome { .. })).await;
    let Frame::Welcome { v, device_id, .. } = welcome else {
        unreachable!()
    };
    assert_eq!(v, 1);
    assert_eq!(
        device_id, install_id,
        "a player's device id is its install id"
    );
    assert_eq!(
        handle.status(),
        ConnectionStatus::Connected {
            device_id: install_id.clone()
        }
    );

    // 3. A browser links: create the link (public), the desktop approves the
    //    code, the browser claims it.
    let (s, ticket) = http.call(
        "POST",
        "/connect/links",
        None,
        Some(json!({"kind":"web","name":"Test"})),
    );
    assert!(s == 200 || s == 201, "create link: {s} {ticket}");
    let code = ticket["code"].as_str().unwrap().to_string();
    let link_id = ticket["linkId"].as_str().unwrap().to_string();
    let secret = ticket["secret"].as_str().unwrap().to_string();

    let (s, preview) = http.call(
        "GET",
        &format!("/connect/links/{code}"),
        Some(&desktop_token),
        None,
    );
    assert_eq!(s, 200, "preview: {preview}");
    assert_eq!(preview["kind"], "web");

    let (s, ok) = http.call(
        "POST",
        &format!("/connect/links/{code}/approve"),
        Some(&desktop_token),
        None,
    );
    assert!(s == 200 || s == 201, "approve: {s} {ok}");

    let (s, claim) = http.call(
        "POST",
        &format!("/connect/links/{link_id}/claim"),
        None,
        Some(json!({"secret": secret})),
    );
    assert!(s == 200 || s == 201, "claim: {s} {claim}");
    assert_eq!(claim["status"], "approved", "claim: {claim}");
    let web_token = claim["accessToken"].as_str().unwrap();
    let web_id = claim["deviceId"].as_str().unwrap().to_string();

    // 4. The browser connects and sees the desktop online.
    let mut web = open_ws(&url).await;
    web.send(Message::text(
        json!({"t":"hello","v":1,"token":web_token,"device":{
            "id": web_id, "kind":"web","name":"Test","platform":"macos","appVersion":"0.1.0","caps":[]}})
        .to_string(),
    ))
    .await
    .unwrap();
    let w = next_of(&mut web, "welcome").await;
    let me = w["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == install_id.as_str())
        .expect("desktop in the web's device list");
    assert_eq!(me["kind"], "desktop");
    assert_eq!(me["online"], true);
    assert_eq!(
        me["caps"],
        json!(["player", "yt", "local", "lyrics", "viz"]),
        "the hub relays the v1.1 caps"
    );

    // 5. cmd play → the crate surfaces it with the controller's id and `from`.
    web.send(Message::text(
        json!({"t":"cmd","id":"real-1","to":install_id,"cmd":{
            "type":"play","index":0,"items":[{
                "src":"yt","videoId":"dQw4w9WgXcQ","title":"Never Gonna Give You Up",
                "artist":"Rick Astley","album":null,"durationMs":213000,"artUrl":null}]}})
        .to_string(),
    ))
    .await
    .unwrap();
    let cmd = desktop_frame(&mut rx, |f| matches!(f, Frame::Cmd { .. })).await;
    let Frame::Cmd { id, from, cmd, .. } = cmd else {
        unreachable!()
    };
    assert_eq!(id, "real-1");
    assert_eq!(from.as_deref(), Some(web_id.as_str()));
    let Command::Play { items, index, .. } = cmd else {
        panic!("expected play, got {cmd:?}")
    };
    assert_eq!(index, 0);
    assert!(matches!(&items[0], TrackRef::Yt(y) if y.video_id == "dQw4w9WgXcQ"));

    // 6. The ack travels back to the web socket.
    handle
        .send(&Frame::Ack {
            id: "real-1".into(),
            ok: true,
            error: None,
        })
        .unwrap();
    let ack = next_of(&mut web, "ack").await;
    assert_eq!(ack["id"], "real-1");
    assert_eq!(ack["ok"], true);

    // 7. A refusal keeps its error code on the way.
    web.send(Message::text(
        json!({"t":"cmd","id":"real-2","to":install_id,"cmd":{"type":"pause"}}).to_string(),
    ))
    .await
    .unwrap();
    let Frame::Cmd { id, .. } = desktop_frame(&mut rx, |f| matches!(f, Frame::Cmd { .. })).await
    else {
        unreachable!()
    };
    handle
        .send(&Frame::Ack {
            id,
            ok: false,
            error: Some(AckError::NothingPlayable),
        })
        .unwrap();
    let ack = next_of(&mut web, "ack").await;
    assert_eq!(
        (
            ack["id"].as_str(),
            ack["ok"].as_bool(),
            ack["error"].as_str()
        ),
        (Some("real-2"), Some(false), Some("nothing_playable"))
    );

    // 8. rpc: the desktop's answer reaches the browser.
    web.send(Message::text(
        json!({"t":"rpc","id":"real-3","to":install_id,"method":"suggest","params":{"query":"ab"}})
            .to_string(),
    ))
    .await
    .unwrap();
    let Frame::Rpc { id, method, .. } =
        desktop_frame(&mut rx, |f| matches!(f, Frame::Rpc { .. })).await
    else {
        unreachable!()
    };
    assert_eq!(method, RpcMethod::Suggest);
    handle
        .send(&Frame::RpcResult {
            id,
            ok: true,
            result: Some(json!({"suggestions":["abba"]})),
            error: None,
        })
        .unwrap();
    let r = next_of(&mut web, "rpc_result").await;
    assert_eq!(r["id"], "real-3");
    assert_eq!(r["result"]["suggestions"][0], "abba");

    // 9. The desktop's state is stamped and fanned out to the browser.
    handle
        .send(&Frame::State {
            device_id: None,
            state: PlayerState {
                seq: 1_760_000_000,
                track: None,
                playing: false,
                position_ms: 0,
                duration_ms: None,
                volume: 0.5,
                shuffle: false,
                repeat: RepeatMode::Off,
                queue: vec![],
                at: None,
            },
        })
        .unwrap();
    let st = next_of(&mut web, "state").await;
    assert_eq!(st["deviceId"], install_id.as_str());
    assert_eq!(st["state"]["seq"], 1_760_000_000u64);
    assert!(st["state"]["at"].is_number(), "the hub stamps `at`: {st}");

    // 10. v1.1 clock: ping → pong carries the hub's epoch time.
    let sent = now_ms();
    handle
        .send(&Frame::Ping {
            id: "clock-1".into(),
        })
        .unwrap();
    let Frame::Pong { id, server_time } =
        desktop_frame(&mut rx, |f| matches!(f, Frame::Pong { .. })).await
    else {
        unreachable!()
    };
    let received = now_ms();
    assert_eq!(id, "clock-1");
    let offset = server_time as i64 - ((sent + received) / 2) as i64;
    println!(
        "hub clock: offset {offset} ms, round trip {} ms",
        received - sent
    );
    assert!(
        offset.abs() < 5 * 60_000,
        "the hub's clock is minutes away from this machine's: {offset} ms"
    );

    // 11. v1.1 lyrics rpc: the desktop's answer — including a miss — reaches
    //     the browser intact.
    let lyrics_json = |name: &str| -> Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../connect-protocol/fixtures")
            .join(name);
        let frame: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        frame["result"].clone()
    };
    for (rpc_id, fixture) in [
        ("real-4", "rpc_result.lyrics.json"),
        ("real-5", "rpc_result.lyrics.none.json"),
    ] {
        web.send(Message::text(
            json!({"t":"rpc","id":rpc_id,"to":install_id,"method":"lyrics","params":{}})
                .to_string(),
        ))
        .await
        .unwrap();
        let Frame::Rpc {
            id,
            method,
            from,
            params,
            ..
        } = desktop_frame(&mut rx, |f| matches!(f, Frame::Rpc { .. })).await
        else {
            unreachable!()
        };
        assert_eq!((id.as_str(), method), (rpc_id, RpcMethod::Lyrics));
        assert_eq!(from.as_deref(), Some(web_id.as_str()));
        assert_eq!(RpcCall::parse(method, &params).unwrap(), RpcCall::Lyrics);
        let result = lyrics_json(fixture);
        handle
            .send(&Frame::RpcResult {
                id,
                ok: true,
                result: Some(result.clone()),
                error: None,
            })
            .unwrap();
        let r = next_of(&mut web, "rpc_result").await;
        assert_eq!(r["id"], rpc_id);
        assert_eq!(r["ok"], true);
        assert_eq!(r["result"], result, "{fixture} changed on the way");
    }

    // 12. v1.1 visualiser: the browser subscribes, the hub tells the desktop
    //     once (0 → 1), and the desktop's frames reach the browser.
    web.send(Message::text(
        json!({"t":"viz_sub","to":install_id,"on":true}).to_string(),
    ))
    .await
    .unwrap();
    let on = desktop_frame(&mut rx, |f| matches!(f, Frame::VizSub { .. })).await;
    assert_eq!(on, Frame::VizSub { to: None, on: true });

    let template = {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../connect-protocol/fixtures/viz.player.json");
        let Frame::Viz { frame, .. } =
            serde_json::from_str::<Frame>(&std::fs::read_to_string(path).unwrap()).unwrap()
        else {
            unreachable!()
        };
        frame
    };
    let mut sent_frames = Vec::new();
    for i in 0..6u64 {
        let mut frame = template.clone();
        frame.heard_at = now_ms() + 200;
        frame.position_ms = 61_400 + i * 50;
        frame.beat = if i == 2 { 0.9 } else { 0.0 };
        handle
            .send(&Frame::Viz {
                device_id: None,
                frame: frame.clone(),
            })
            .unwrap();
        sent_frames.push(frame);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    for expected in &sent_frames {
        let v = next_of(&mut web, "viz").await;
        assert_eq!(
            v["deviceId"],
            install_id.as_str(),
            "the hub stamps the player"
        );
        // Compared as numbers, not text: the hub's JSON writes `0.0` as `0`.
        let got: VizFrame = serde_json::from_value(v["frame"].clone()).unwrap();
        assert_eq!(&got, expected, "a viz frame is relayed verbatim, in order");
        assert_eq!(v["frame"]["bands"].as_array().unwrap().len(), 32);
    }

    // 12b. The hub repeats the subscription when the player comes back: a
    //      subscription belongs to the controller's socket, not the player's.
    handle.shutdown().await;
    let (handle, mut rx) = spawn(
        ClientConfig::new(&url, device),
        Arc::new(Tokens(Mutex::new(desktop_token.clone()))),
        &tokio::runtime::Handle::current(),
    );
    let mut seen = Vec::new();
    let again = timeout(STEP, async {
        loop {
            match rx.recv().await.expect("client ended") {
                ClientEvent::Frame(f) => {
                    let done = matches!(*f, Frame::VizSub { .. });
                    seen.push(f.tag());
                    if done {
                        return *f;
                    }
                }
                ClientEvent::Status(_) => {}
            }
        }
    })
    .await
    .expect("no viz_sub after reconnecting");
    assert_eq!(again, Frame::VizSub { to: None, on: true });
    assert_eq!(seen.first(), Some(&"welcome"), "welcome first: {seen:?}");

    // 12c. 1 → 0: the browser unsubscribes and the desktop is told.
    web.send(Message::text(
        json!({"t":"viz_sub","to":install_id,"on":false}).to_string(),
    ))
    .await
    .unwrap();
    let off = desktop_frame(&mut rx, |f| matches!(f, Frame::VizSub { .. })).await;
    assert_eq!(
        off,
        Frame::VizSub {
            to: None,
            on: false
        }
    );

    handle.shutdown().await;
}
