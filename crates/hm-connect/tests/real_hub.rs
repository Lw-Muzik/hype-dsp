//! End-to-end against a REAL hub (`hypemuzik-management`), not the in-process
//! fake in `client.rs`. Ignored by default; run it with the hub up:
//!
//! ```sh
//! HM_CONNECT_REAL_API=http://localhost:4000/api \
//!   cargo test -p hm-connect --test real_hub -- --ignored --nocapture
//! ```
//!
//! The desktop is the crate's client; the "web controller" is a hand-driven
//! socket that links the way a browser does (create link, desktop approves the
//! code, claim), then commands the desktop. Plain `http://` / `ws://` only —
//! this is a local-development check, so the test carries its own tiny HTTP
//! client instead of pulling one into the crate.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use hm_connect::protocol::*;
use hm_connect::rest::ws_url;
use hm_connect::{
    spawn, Cap, ClientConfig, ClientEvent, ConnectionStatus, DeviceKind, Renewal, TokenProvider,
};
use serde_json::{json, Value};
use tokio::net::TcpStream as TokioTcp;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

const STEP: Duration = Duration::from_secs(10);

// ------------------------------------------------------------- tiny HTTP

struct Http {
    host: String,
    port: u16,
    prefix: String,
}

impl Http {
    fn new(api: &str) -> Self {
        let rest = api
            .strip_prefix("http://")
            .expect("HM_CONNECT_REAL_API must be a plain http:// URL");
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let (host, port) = authority.split_once(':').unwrap_or((authority, "80"));
        Self {
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
            "{method} {}{path} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\nAccept: application/json\r\n",
            self.prefix, self.host, self.port
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
        let mut s = TcpStream::connect((self.host.as_str(), self.port)).expect("hub unreachable");
        s.set_read_timeout(Some(STEP)).unwrap();
        s.write_all(req.as_bytes()).unwrap();
        let mut raw = Vec::new();
        s.read_to_end(&mut raw).unwrap();
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
        caps: vec![Cap::Player, Cap::Yt, Cap::Local],
    };
    let url = ws_url(&api).unwrap();
    let (handle, mut rx) = spawn(
        ClientConfig::new(&url, device),
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
    let (mut web, _): (Ws, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("web ws");
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

    handle.shutdown().await;
}
