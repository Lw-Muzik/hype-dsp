//! The player-side hub client.
//!
//! [`spawn`] starts a supervisor task that keeps one socket to the hub alive:
//! it sends `hello`, waits for `welcome`, forwards inbound frames to the app,
//! writes the app's outbound frames, and reacts to close codes exactly as
//! §3.10 says (see [`crate::policy`]). The returned [`ConnectHandle`] is the
//! app's side: send a frame, read the status, stop.
//!
//! Protocol-level WebSocket pings from the hub are answered by tungstenite
//! itself while the read half is polled; nothing here has to do it.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use thiserror::Error;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::{sleep, sleep_until, timeout, Instant};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, WebSocketConfig};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::Connector;

use crate::policy::{classify_close, Backoff, CloseAction, StopReason};
use crate::protocol::{
    AckError, DeviceHello, Frame, RpcError, MAX_FRAME_BYTES, MAX_VIZ_PER_SECOND, PROTOCOL_VERSION,
};
use crate::token::{expires_within, Renewal, TokenProvider};

/// Everything the client needs besides tokens.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// `ws(s)://<api-host>/api/connect/ws` — see [`crate::rest::ws_url`].
    pub url: String,
    /// What this device announces in `hello`.
    pub device: DeviceHello,
    pub backoff: Backoff,
    /// TCP + TLS + WebSocket handshake budget.
    pub connect_timeout: Duration,
    /// How long to wait for `welcome` after `hello`.
    pub welcome_timeout: Duration,
    /// Silence after which the socket is presumed dead (the hub pings every
    /// 20 s, so a live socket is never quiet this long). Catches the
    /// half-open sockets a laptop's sleep leaves behind.
    pub idle_timeout: Duration,
    /// Refresh the access token before connecting when it expires within this.
    pub refresh_margin: Duration,
    /// How often to look for a session while there is none.
    pub session_poll: Duration,
    /// Outbound frames buffered while the socket writes.
    pub outbound_capacity: usize,
}

impl ClientConfig {
    pub fn new(url: impl Into<String>, device: DeviceHello) -> Self {
        Self {
            url: url.into(),
            device,
            backoff: Backoff::default(),
            connect_timeout: Duration::from_secs(15),
            welcome_timeout: Duration::from_secs(10),
            idle_timeout: Duration::from_secs(60),
            refresh_margin: Duration::from_secs(60),
            session_poll: Duration::from_secs(5),
            outbound_capacity: 256,
        }
    }
}

/// The connection's lifecycle, as the app shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "state",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ConnectionStatus {
    /// Opening the socket; `attempt` counts retries since the last `welcome`.
    Connecting { attempt: u32 },
    /// `welcome` received; `device_id` is what the hub calls this device.
    Connected { device_id: String },
    /// Between attempts.
    Waiting {
        attempt: u32,
        retry_in_ms: u64,
        reason: String,
    },
    /// No account session yet (or it was lost); polling for one.
    NoSession,
    /// Not reconnecting until started again.
    Stopped { reason: StopReason },
}

/// What the client reports to the app.
#[derive(Debug, Clone, PartialEq)]
pub enum ClientEvent {
    Status(ConnectionStatus),
    /// An inbound frame addressed to a player — see [`is_for_player`].
    Frame(Box<Frame>),
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SendError {
    #[error("not connected to the Hype Connect hub")]
    NotConnected,
    #[error("a player doesn't send `{0}` frames")]
    NotAllowed(&'static str),
    #[error("frame is {0} bytes; the hub accepts at most 256 KiB")]
    TooLarge(usize),
    #[error("too many frames waiting to be sent")]
    Busy,
    #[error("a `viz` frame must carry exactly 32 bands, all in 0..1")]
    BadViz,
    #[error("more than {MAX_VIZ_PER_SECOND} `viz` frames in a second")]
    VizRateLimited,
    #[error("couldn't encode the frame: {0}")]
    Encode(String),
}

struct Shared {
    /// The live socket's writer queue; `Some` only between `welcome` and the
    /// end of that socket.
    outbound: Mutex<Option<mpsc::Sender<String>>>,
    status: Mutex<ConnectionStatus>,
    /// When the last `viz` frames were queued — the hub's per-socket budget.
    viz_budget: Mutex<SlidingWindow>,
}

/// At most `max` events inside any `span`: the shape of the hub's `viz`
/// budget (§3.11), which drops the excess silently.
#[derive(Debug)]
struct SlidingWindow {
    max: usize,
    span: Duration,
    events: VecDeque<std::time::Instant>,
}

impl SlidingWindow {
    fn new(max: usize, span: Duration) -> Self {
        Self {
            max,
            span,
            events: VecDeque::with_capacity(max),
        }
    }

    /// Record an event at `now` unless that would exceed the budget.
    fn try_acquire(&mut self, now: std::time::Instant) -> bool {
        while let Some(&oldest) = self.events.front() {
            if now.saturating_duration_since(oldest) >= self.span {
                self.events.pop_front();
            } else {
                break;
            }
        }
        if self.events.len() >= self.max {
            return false;
        }
        self.events.push_back(now);
        true
    }
}

/// Whether an inbound frame is addressed to a player and so reaches the app.
/// Everything else (`pong` aside, which the app needs for the hub clock) is
/// controller-bound chatter — acks, `viz`, `select` … — and is dropped.
pub fn is_for_player(frame: &Frame) -> bool {
    matches!(
        frame,
        Frame::Welcome { .. }
            | Frame::Cmd { .. }
            | Frame::Rpc { .. }
            | Frame::Devices { .. }
            | Frame::State { .. }
            | Frame::Error { .. }
            | Frame::VizSub { .. }
            | Frame::Pong { .. }
    )
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The app's handle on a running client. Dropping it stops the client.
pub struct ConnectHandle {
    shared: Arc<Shared>,
    shutdown: watch::Sender<bool>,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl ConnectHandle {
    /// Queue a frame for the hub. Only what a player legitimately sends is
    /// accepted (`ack`, `rpc_result`, its own `state` and `viz`, and `ping`
    /// for the clock), so a bug on the app side can't speak for a controller
    /// or re-`hello`.
    pub fn send(&self, frame: &Frame) -> Result<(), SendError> {
        let viz = match frame {
            Frame::Ack { .. } | Frame::RpcResult { .. } | Frame::Ping { .. } => false,
            Frame::State {
                device_id: None,
                state,
            } if state.at.is_none() => false,
            Frame::Viz {
                device_id: None,
                frame,
            } => {
                if !frame.is_valid() {
                    return Err(SendError::BadViz);
                }
                true
            }
            other => return Err(SendError::NotAllowed(other.tag())),
        };
        let text = serde_json::to_string(frame).map_err(|e| SendError::Encode(e.to_string()))?;
        // A valid `viz` frame (32 finite numbers in 0..1) serializes to well
        // under the hub's 1 KiB cap (`MAX_VIZ_FRAME_BYTES`) by construction —
        // pinned in `tests/client.rs` — so only the general limit is checked.
        if text.len() > MAX_FRAME_BYTES {
            return Err(SendError::TooLarge(text.len()));
        }
        let guard = lock(&self.shared.outbound);
        let tx = guard.as_ref().ok_or(SendError::NotConnected)?;
        // Spend the budget only for a frame that will really be queued.
        if viz
            && tx.capacity() > 0
            && !lock(&self.shared.viz_budget).try_acquire(std::time::Instant::now())
        {
            return Err(SendError::VizRateLimited);
        }
        tx.try_send(text).map_err(|e| match e {
            mpsc::error::TrySendError::Full(_) => SendError::Busy,
            mpsc::error::TrySendError::Closed(_) => SendError::NotConnected,
        })
    }

    pub fn status(&self) -> ConnectionStatus {
        lock(&self.shared.status).clone()
    }

    /// Ask the client to close its socket (code 1000) and stop.
    pub fn stop(&self) {
        let _ = self.shutdown.send(true);
    }

    /// Stop and wait until the client has finished.
    pub async fn shutdown(&self) {
        self.stop();
        let task = lock(&self.task).take();
        if let Some(task) = task {
            let _ = task.await;
        }
    }

    /// Whether the supervisor task has ended (stopped by a close code or by
    /// [`stop`](Self::stop)).
    pub fn is_finished(&self) -> bool {
        lock(&self.task)
            .as_ref()
            .is_none_or(JoinHandle::is_finished)
    }
}

impl Drop for ConnectHandle {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
    }
}

/// Start the client on `runtime`. Status changes and inbound frames arrive on
/// the returned receiver, in order.
pub fn spawn(
    config: ClientConfig,
    tokens: Arc<dyn TokenProvider>,
    runtime: &tokio::runtime::Handle,
) -> (ConnectHandle, mpsc::UnboundedReceiver<ClientEvent>) {
    let (events_tx, events_rx) = mpsc::unbounded_channel();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let shared = Arc::new(Shared {
        outbound: Mutex::new(None),
        status: Mutex::new(ConnectionStatus::Connecting { attempt: 0 }),
        viz_budget: Mutex::new(SlidingWindow::new(
            MAX_VIZ_PER_SECOND,
            Duration::from_secs(1),
        )),
    });
    let supervisor = Supervisor {
        config,
        tokens,
        shared: shared.clone(),
        events: events_tx,
        shutdown: shutdown_rx,
    };
    let task = runtime.spawn(supervisor.run());
    let handle = ConnectHandle {
        shared,
        shutdown: shutdown_tx,
        task: Mutex::new(Some(task)),
    };
    (handle, events_rx)
}

/// How one socket ended.
#[derive(Debug)]
enum SessionEnd {
    Shutdown,
    /// The hub closed it, with this code (`None`: no close frame at all).
    Closed(Option<u16>),
    /// Couldn't connect, the hub went silent, or the socket errored.
    Failed(String),
}

struct Supervisor {
    config: ClientConfig,
    tokens: Arc<dyn TokenProvider>,
    shared: Arc<Shared>,
    events: mpsc::UnboundedSender<ClientEvent>,
    shutdown: watch::Receiver<bool>,
}

impl Supervisor {
    fn set_status(&self, status: ConnectionStatus) {
        *lock(&self.shared.status) = status.clone();
        let _ = self.events.send(ClientEvent::Status(status));
    }

    fn emit(&self, frame: Frame) {
        let _ = self.events.send(ClientEvent::Frame(Box::new(frame)));
    }

    async fn current_token(&self) -> Option<String> {
        let tokens = self.tokens.clone();
        tokio::task::spawn_blocking(move || tokens.current())
            .await
            .unwrap_or(None)
    }

    async fn renew(&self, rejected: &str) -> Renewal {
        let tokens = self.tokens.clone();
        let rejected = rejected.to_string();
        tokio::task::spawn_blocking(move || tokens.renew(&rejected))
            .await
            .unwrap_or(Renewal::Unavailable)
    }

    /// Sleep for `d`, or return `true` early if shutdown was requested.
    async fn pause(&mut self, d: Duration) -> bool {
        tokio::select! {
            _ = sleep(d) => false,
            _ = shutdown_requested(&mut self.shutdown) => true,
        }
    }

    async fn run(mut self) {
        let connector = match self.connector() {
            Ok(c) => c,
            Err(_) => {
                // Without TLS a wss hub is unreachable; retrying can't help.
                self.set_status(ConnectionStatus::Stopped {
                    reason: StopReason::TlsUnavailable,
                });
                return;
            }
        };
        let mut attempt: u32 = 0;
        let mut fresh: Option<String> = None;
        loop {
            if *self.shutdown.borrow() {
                break;
            }
            let token = match fresh.take() {
                Some(t) => Some(t),
                None => self.current_token().await,
            };
            let Some(mut token) = token else {
                self.set_status(ConnectionStatus::NoSession);
                if self.pause(self.config.session_poll).await {
                    break;
                }
                continue;
            };
            if expires_within(&token, self.config.refresh_margin, SystemTime::now()) {
                match self.renew(&token).await {
                    Renewal::Renewed(t) => token = t,
                    Renewal::SessionGone => {
                        self.set_status(ConnectionStatus::NoSession);
                        if self.pause(self.config.session_poll).await {
                            break;
                        }
                        continue;
                    }
                    // Try the old one; the hub answers 4001 if it's dead.
                    Renewal::Unavailable => {}
                }
            }

            self.set_status(ConnectionStatus::Connecting { attempt });
            let (welcomed, end) = self.session(&token, connector.clone()).await;
            if welcomed {
                attempt = 0;
            }
            let (action, reason) = match end {
                SessionEnd::Shutdown => break,
                SessionEnd::Closed(code) => (
                    classify_close(code),
                    match code {
                        Some(c) => format!("closed by the hub ({c})"),
                        None => "connection lost".to_string(),
                    },
                ),
                SessionEnd::Failed(why) => (CloseAction::Reconnect, why),
            };
            match action {
                CloseAction::Stop(reason) => {
                    self.set_status(ConnectionStatus::Stopped { reason });
                    return;
                }
                CloseAction::RefreshThenReconnect => match self.renew(&token).await {
                    Renewal::Renewed(t) => fresh = Some(t),
                    Renewal::SessionGone => {
                        self.set_status(ConnectionStatus::NoSession);
                        if self.pause(self.config.session_poll).await {
                            break;
                        }
                        continue;
                    }
                    Renewal::Unavailable => {}
                },
                CloseAction::Reconnect => {}
            }
            let delay = self.config.backoff.delay(attempt, fastrand::f64());
            attempt = attempt.saturating_add(1);
            self.set_status(ConnectionStatus::Waiting {
                attempt,
                retry_in_ms: delay.as_millis() as u64,
                reason,
            });
            if self.pause(delay).await {
                break;
            }
        }
        self.set_status(ConnectionStatus::Stopped {
            reason: StopReason::Shutdown,
        });
    }

    fn connector(&self) -> Result<Option<Connector>, rustls::Error> {
        if self.config.url.starts_with("wss://") {
            crate::tls::connector().map(Some)
        } else {
            Ok(None)
        }
    }

    /// Run one socket from connect to close. Returns whether `welcome`
    /// arrived, and how it ended.
    async fn session(&mut self, token: &str, connector: Option<Connector>) -> (bool, SessionEnd) {
        let ws_config = WebSocketConfig::default()
            // Inbound is not ours to police beyond sanity: the hub enforces
            // its own 256 KiB rule, and a too-strict limit here would turn one
            // oversized fan-out into a reconnect loop.
            .max_message_size(Some(4 * MAX_FRAME_BYTES))
            .max_frame_size(Some(4 * MAX_FRAME_BYTES));
        let connect = tokio_tungstenite::connect_async_tls_with_config(
            self.config.url.as_str(),
            Some(ws_config),
            true,
            connector,
        );
        let ws = tokio::select! {
            _ = shutdown_requested(&mut self.shutdown) => return (false, SessionEnd::Shutdown),
            r = timeout(self.config.connect_timeout, connect) => match r {
                Ok(Ok((ws, _response))) => ws,
                Ok(Err(e)) => return (false, SessionEnd::Failed(format!("couldn't connect: {e}"))),
                Err(_) => return (false, SessionEnd::Failed("timed out connecting".into())),
            },
        };
        let (mut sink, mut stream) = ws.split();

        let hello = Frame::Hello {
            v: PROTOCOL_VERSION,
            token: token.to_string(),
            device: self.config.device.clone(),
        };
        let hello = serde_json::to_string(&hello).expect("hello always serializes");
        if let Err(e) = sink.send(Message::text(hello)).await {
            return (
                false,
                SessionEnd::Failed(format!("couldn't send hello: {e}")),
            );
        }

        let (tx, mut rx) = mpsc::channel::<String>(self.config.outbound_capacity.max(1));
        let mut welcomed = false;
        let welcome_deadline = Instant::now() + self.config.welcome_timeout;
        let mut idle_deadline = Instant::now() + self.config.idle_timeout;

        let end = loop {
            let deadline = if welcomed {
                idle_deadline
            } else {
                welcome_deadline.min(idle_deadline)
            };
            tokio::select! {
                _ = shutdown_requested(&mut self.shutdown) => {
                    let _ = sink
                        .send(Message::Close(Some(CloseFrame {
                            code: CloseCode::Normal,
                            reason: "".into(),
                        })))
                        .await;
                    break SessionEnd::Shutdown;
                }
                _ = sleep_until(deadline) => {
                    break SessionEnd::Failed(if welcomed {
                        "the hub went silent".into()
                    } else {
                        "no welcome from the hub".into()
                    });
                }
                Some(text) = rx.recv() => {
                    if let Err(e) = sink.send(Message::text(text)).await {
                        break SessionEnd::Failed(format!("send failed: {e}"));
                    }
                }
                msg = stream.next() => {
                    idle_deadline = Instant::now() + self.config.idle_timeout;
                    match msg {
                        None => break SessionEnd::Closed(None),
                        Some(Err(e)) => break SessionEnd::Failed(format!("socket error: {e}")),
                        Some(Ok(Message::Close(frame))) => {
                            break SessionEnd::Closed(frame.map(|f| u16::from(f.code)));
                        }
                        Some(Ok(Message::Text(text))) => {
                            match serde_json::from_str::<Frame>(&text) {
                                Ok(frame) => {
                                    if let Frame::Welcome { device_id, .. } = &frame {
                                        welcomed = true;
                                        *lock(&self.shared.outbound) = Some(tx.clone());
                                        self.set_status(ConnectionStatus::Connected {
                                            device_id: device_id.clone(),
                                        });
                                    }
                                    // ack/viz/… are not addressed to a player.
                                    if is_for_player(&frame) {
                                        self.emit(frame);
                                    }
                                }
                                Err(_) => {
                                    // A command or rpc this build can't parse
                                    // still gets an answer, so the controller
                                    // isn't left waiting for the hub timeout.
                                    if let Some(reply) = unsupported_reply(&text) {
                                        let reply = serde_json::to_string(&reply)
                                            .expect("reply always serializes");
                                        if let Err(e) = sink.send(Message::text(reply)).await {
                                            break SessionEnd::Failed(format!("send failed: {e}"));
                                        }
                                    }
                                }
                            }
                        }
                        // Pings are answered by tungstenite while reading.
                        Some(Ok(_)) => {}
                    }
                }
            }
        };
        *lock(&self.shared.outbound) = None;
        (welcomed, end)
    }
}

/// `ack`/`rpc_result {ok:false, error:"unsupported"}` for a `cmd`/`rpc` frame
/// that carries an id but didn't parse (e.g. a command type from a newer
/// controller).
fn unsupported_reply(text: &str) -> Option<Frame> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let id = v.get("id")?.as_str()?.to_string();
    match v.get("t")?.as_str()? {
        "cmd" => Some(Frame::ack_error(id, AckError::Unsupported)),
        "rpc" => Some(Frame::rpc_error(id, RpcError::Unsupported)),
        _ => None,
    }
}

/// Resolves once shutdown is requested (or the handle is gone).
async fn shutdown_requested(rx: &mut watch::Receiver<bool>) {
    loop {
        if *rx.borrow_and_update() {
            return;
        }
        if rx.changed().await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unparseable_commands_and_rpcs_get_unsupported() {
        let cmd = r#"{"t":"cmd","id":"c-9","from":"w","cmd":{"type":"dance"}}"#;
        assert_eq!(
            unsupported_reply(cmd),
            Some(Frame::ack_error("c-9", AckError::Unsupported))
        );
        let rpc = r#"{"t":"rpc","id":"r-9","from":"w","method":"karaoke","params":{}}"#;
        assert_eq!(
            unsupported_reply(rpc),
            Some(Frame::rpc_error("r-9", RpcError::Unsupported))
        );
        assert_eq!(unsupported_reply(r#"{"t":"devices","bad":true}"#), None);
        assert_eq!(
            unsupported_reply(r#"{"t":"cmd"}"#),
            None,
            "no id, nothing to answer"
        );
        assert_eq!(unsupported_reply("not json"), None);
    }

    #[test]
    fn only_player_bound_frames_pass_the_filter() {
        use crate::protocol::VizFrame;
        let pass = |json: &str| is_for_player(&serde_json::from_str::<Frame>(json).unwrap());
        assert!(pass(r#"{"t":"viz_sub","on":true}"#));
        assert!(pass(r#"{"t":"pong","id":"p","serverTime":1}"#));
        assert!(pass(r#"{"t":"error","code":"x","message":"y"}"#));
        assert!(!pass(r#"{"t":"ping","id":"p"}"#));
        assert!(!pass(r#"{"t":"ack","id":"a","ok":true}"#));
        assert!(!pass(r#"{"t":"select","deviceId":null}"#));
        let viz = Frame::Viz {
            device_id: Some("p".into()),
            frame: VizFrame {
                heard_at: 1,
                position_ms: 1,
                bands: vec![0.0; 32],
                level: 0.0,
                beat: 0.0,
            },
        };
        assert!(!is_for_player(&viz), "viz fans out to controllers only");
    }

    #[test]
    fn the_sliding_window_allows_max_per_span_and_recovers() {
        use std::time::Instant;
        let t0 = Instant::now();
        let mut w = SlidingWindow::new(3, Duration::from_secs(1));
        assert!(w.try_acquire(t0));
        assert!(w.try_acquire(t0 + Duration::from_millis(100)));
        assert!(w.try_acquire(t0 + Duration::from_millis(200)));
        assert!(
            !w.try_acquire(t0 + Duration::from_millis(999)),
            "4th inside a second"
        );
        assert!(
            w.try_acquire(t0 + Duration::from_millis(1000)),
            "the first aged out exactly a span later"
        );
        assert!(!w.try_acquire(t0 + Duration::from_millis(1001)));
        assert!(w.try_acquire(t0 + Duration::from_millis(1100)));
    }
}
