//! Phone Link control server (the "cast" / push direction).
//!
//! A small HTTP server phones POST to in order to play a track *on the desktop*
//! (the desktop then pulls that track from the phone's media server and runs it
//! through the DSP chain). The desktop also advertises itself over mDNS as a
//! Phone-Link `player` so phones can discover it.
//!
//! Auth: every request carries the same bearer token minted during pairing.
//! The token both authenticates the phone and identifies *which* paired phone
//! is casting (so we know where to pull `/stream` from).

use hm_audio::AudioEngine;
use hm_link::LinkState;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};
use tiny_http::{Header, Method, Request, Response, Server};

/// Now-playing payload pushed to the UI when a phone casts.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct NowPlaying {
    title: String,
    artist: Option<String>,
    source: String,
}

/// Start the control server on an ephemeral port and advertise it. Returns the
/// chosen port (for logging), or `None` if it couldn't bind. Runs for the app's
/// lifetime on its own thread.
pub fn start(app: AppHandle) -> Option<u16> {
    let server = Server::http("0.0.0.0:0").ok()?;
    let port = server.server_addr().to_ip()?.port();

    // Advertise as a player so phones can find us to cast. Kept alive by moving
    // it into the server thread below.
    let self_id = { app.state::<LinkState>().self_id() };
    let advertiser = hm_link::advertise_player(&hm_link::device_name(), &self_id, port).ok();

    std::thread::Builder::new()
        .name("hm-control".into())
        .spawn(move || {
            let _advertiser = advertiser; // hold the mDNS registration open
            for request in server.incoming_requests() {
                handle(&app, request);
            }
        })
        .ok()?;
    Some(port)
}

fn handle(app: &AppHandle, request: Request) {
    let method = request.method().clone();
    let path = request.url().split('?').next().unwrap_or("").to_string();
    let token = bearer(&request);

    match (method, path.as_str()) {
        (Method::Get, "/ping") => {
            let self_id = { app.state::<LinkState>().self_id() };
            respond_json(
                request,
                &json!({ "name": hm_link::device_name(), "id": self_id, "v": 1 }).to_string(),
            );
        }
        (Method::Post, "/cast") => handle_cast(app, request, &token),
        (Method::Post, "/transport") => handle_transport(app, request, &token),
        (Method::Get, "/now") => handle_now(app, request, &token),
        _ => {
            let _ = request.respond(Response::from_string("not found").with_status_code(404));
        }
    }
}

fn handle_cast(app: &AppHandle, mut request: Request, token: &str) {
    let body = read_body(&mut request);
    let v: Value = serde_json::from_str(&body).unwrap_or_else(|_| json!({}));
    let track_id = v["trackId"].as_str().unwrap_or("");
    let ext = v["ext"].as_str().unwrap_or("mp3");
    let title = v["title"].as_str().unwrap_or("Phone audio").to_string();
    let artist = v["artist"].as_str().map(str::to_string);

    let target = { app.state::<LinkState>().stream_target_for_token(token, track_id, ext) };
    let Some((url, headers)) = target else {
        return unauthorized(request);
    };

    let played = app.state::<AudioEngine>().play_stream(url, headers).is_ok();
    if !played {
        let _ = request.respond(Response::from_string("playback failed").with_status_code(500));
        return;
    }
    let _ = app.emit(
        "link:now_playing",
        NowPlaying {
            title,
            artist,
            source: "phone".into(),
        },
    );
    respond_json(request, &json!({ "ok": true }).to_string());
}

fn handle_transport(app: &AppHandle, mut request: Request, token: &str) {
    if !{ app.state::<LinkState>().is_known_token(token) } {
        return unauthorized(request);
    }
    let body = read_body(&mut request);
    let v: Value = serde_json::from_str(&body).unwrap_or_else(|_| json!({}));
    let engine = app.state::<AudioEngine>();
    match v["action"].as_str().unwrap_or("") {
        "pause" => engine.pause(),
        "resume" | "play" => engine.resume(),
        "stop" => engine.stop(),
        _ => {}
    }
    respond_json(request, &json!({ "ok": true }).to_string());
}

fn handle_now(app: &AppHandle, request: Request, token: &str) {
    if !{ app.state::<LinkState>().is_known_token(token) } {
        return unauthorized(request);
    }
    let engine = app.state::<AudioEngine>();
    let pos = engine.pos();
    let now = json!({
        "playing": engine.is_playing(),
        "positionMs": (pos.position_secs() * 1000.0) as i64,
        "durationMs": pos.duration_secs().map(|d| (d * 1000.0) as i64),
    });
    respond_json(request, &now.to_string());
}

// ------------------------------------------------------------------ helpers

fn bearer(request: &Request) -> String {
    for h in request.headers() {
        if h.field.equiv("Authorization") {
            if let Some(token) = h.value.as_str().strip_prefix("Bearer ") {
                return token.to_string();
            }
        }
    }
    String::new()
}

fn read_body(request: &mut Request) -> String {
    let mut body = String::new();
    let _ = request.as_reader().read_to_string(&mut body);
    body
}

fn respond_json(request: Request, body: &str) {
    let header = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
        .expect("valid header");
    let _ = request.respond(Response::from_string(body).with_header(header));
}

fn unauthorized(request: Request) {
    let _ = request.respond(Response::from_string("unauthorized").with_status_code(401));
}
