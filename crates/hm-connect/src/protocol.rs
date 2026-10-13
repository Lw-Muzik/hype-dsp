//! Hype Connect protocol v1 (+ the v1.1 `lyrics` rpc and visualiser stream) —
//! every WebSocket frame, REST body and RPC payload
//! from `hypemuzik-management/backend/docs/hype-connect.md`, as serde types.
//!
//! The JSON these produce is the contract, so the shapes follow it literally:
//!
//! * nullable fields (`artist: string | null`) are `Option<T>` **without**
//!   `skip_serializing_if`, so `None` goes out as `null`;
//! * optional fields (`positionMs?`, `error?`, `to`/`from`) are `Option<T>`
//!   **with** it, so `None` is omitted;
//! * millisecond / counter fields accept a JSON float as well as an integer
//!   (a JS controller can produce `90000.5` from a slider) but always
//!   serialize as integers — lenient in, exact out.
//!
//! `connect-protocol/fixtures/*.json` round-trips through these types in
//! `tests/fixtures.rs`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The protocol version carried by `hello` / `welcome`.
pub const PROTOCOL_VERSION: u32 = 1;

/// Largest frame the hub accepts (§3). Outbound frames are checked against it.
pub const MAX_FRAME_BYTES: usize = 256 * 1024;

/// Most upcoming queue entries a `state` frame may carry (§3.6).
pub const MAX_STATE_QUEUE: usize = 50;

/// Largest page the `library` rpc may ask for (§3.8).
pub const MAX_LIBRARY_LIMIT: u64 = 200;

// ----------------------------------------------------------------- devices

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    Web,
    Mobile,
    Desktop,
}

/// Most `viz` frames a player may send per second (§3.11); the hub drops the
/// excess silently, so [`crate::ConnectHandle::send`] refuses them instead.
pub const MAX_VIZ_PER_SECOND: usize = 30;

/// Largest `viz` frame the hub accepts (§3.11).
pub const MAX_VIZ_FRAME_BYTES: usize = 1024;

/// Bands in a [`VizFrame`] — fixed by the contract.
pub const VIZ_BANDS: usize = 32;

/// What a device can do. `player` obeys commands and publishes state, `yt`
/// plays/searches YouTube Music, `local` answers the `library` rpc, `lyrics`
/// answers the `lyrics` rpc and `viz` streams `viz` frames (both v1.1).
///
/// Receivers ignore caps they don't know (§3.1) — see [`lenient_caps`] — so a
/// newer peer's capability never makes a `welcome` or `devices` frame fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Cap {
    Player,
    Yt,
    Local,
    Lyrics,
    Viz,
}

/// Deserialize a `caps` array, dropping entries this build doesn't know
/// instead of rejecting the frame.
fn lenient_caps<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<Cap>, D::Error> {
    let raw = Vec::<Value>::deserialize(d)?;
    Ok(raw
        .into_iter()
        .filter_map(|v| serde_json::from_value::<Cap>(v).ok())
        .collect())
}

/// The device block of `hello`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceHello {
    pub id: String,
    pub kind: DeviceKind,
    pub name: String,
    pub platform: String,
    pub app_version: String,
    #[serde(deserialize_with = "lenient_caps")]
    pub caps: Vec<Cap>,
}

/// A linked device as the hub reports it (`welcome`, `devices`, REST).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub id: String,
    pub kind: DeviceKind,
    pub name: String,
    pub platform: Option<String>,
    pub app_version: Option<String>,
    #[serde(deserialize_with = "lenient_caps")]
    pub caps: Vec<Cap>,
    pub online: bool,
    pub last_seen_at: String,
}

// ------------------------------------------------------------------ tracks

/// A playable (or, for `other`, displayable) track reference (§3.5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "src", rename_all = "lowercase")]
pub enum TrackRef {
    Yt(YtRef),
    Local(LocalRef),
    Other(OtherRef),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YtRef {
    pub video_id: String,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    #[serde(default, with = "lenient::opt_u64")]
    pub duration_ms: Option<u64>,
    pub art_url: Option<String>,
}

/// A track in one device's own library. `id` is that device's library id (the
/// desktop uses the file path).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalRef {
    pub device_id: String,
    pub id: String,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    #[serde(default, with = "lenient::opt_u64")]
    pub duration_ms: Option<u64>,
    /// Always `null` on the wire: the browser can't reach on-device art.
    #[serde(default)]
    pub art_url: AlwaysNull,
}

/// Cloud, phone-link, radio … — appears only in `state`, never in commands.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OtherRef {
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    #[serde(default, with = "lenient::opt_u64")]
    pub duration_ms: Option<u64>,
    pub art_url: Option<String>,
}

/// A field the contract pins to `null`. Whatever arrives is ignored; `null`
/// is what leaves.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AlwaysNull;

impl Serialize for AlwaysNull {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_none()
    }
}

impl<'de> Deserialize<'de> for AlwaysNull {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        serde::de::IgnoredAny::deserialize(d)?;
        Ok(AlwaysNull)
    }
}

// ---------------------------------------------------------------- commands

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EnqueueMode {
    Next,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RepeatMode {
    Off,
    All,
    One,
}

/// A controller → player command (§3.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
pub enum Command {
    Play {
        items: Vec<TrackRef>,
        index: usize,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "lenient::opt_u64"
        )]
        position_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        radio: Option<bool>,
    },
    Enqueue {
        items: Vec<TrackRef>,
        mode: EnqueueMode,
    },
    Pause,
    Resume,
    Stop,
    Next,
    Prev,
    Seek {
        #[serde(with = "lenient::u64")]
        position_ms: u64,
    },
    Jump {
        uid: String,
    },
    Volume {
        value: f64,
    },
    Shuffle {
        on: bool,
    },
    Repeat {
        mode: RepeatMode,
    },
}

/// Why a player refused (or the hub answered for) a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AckError {
    Offline,
    NotAPlayer,
    NothingPlayable,
    Unsupported,
    Failed,
    Timeout,
}

// ------------------------------------------------------------------- state

/// One upcoming queue entry. `uid` is stable for as long as the entry stays
/// queued, so a controller can `jump` to it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueueEntry {
    pub uid: String,
    pub track: TrackRef,
}

/// A player's state (§3.6). `at` is the hub's arrival stamp: absent on the
/// player → hub frame, present on everything the hub fans out
/// (`StampedState`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerState {
    #[serde(with = "lenient::u64")]
    pub seq: u64,
    pub track: Option<TrackRef>,
    pub playing: bool,
    #[serde(with = "lenient::u64")]
    pub position_ms: u64,
    #[serde(default, with = "lenient::opt_u64")]
    pub duration_ms: Option<u64>,
    pub volume: f64,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub queue: Vec<QueueEntry>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "lenient::opt_u64"
    )]
    pub at: Option<u64>,
}

// --------------------------------------------------------------------- rpc

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RpcMethod {
    Search,
    Suggest,
    Open,
    Artist,
    Library,
    /// v1.1: the lyrics of the track playing now (needs cap `lyrics`).
    Lyrics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RpcError {
    Offline,
    Unsupported,
    NotSignedIn,
    Failed,
    Timeout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchFilter {
    All,
    Songs,
    Videos,
    Albums,
    Artists,
    Playlists,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchParams {
    pub query: String,
    pub filter: SearchFilter,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SuggestParams {
    pub query: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenParams {
    pub item: Item,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtistParams {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LibraryParams {
    #[serde(with = "lenient::u64")]
    pub offset: u64,
    #[serde(with = "lenient::u64")]
    pub limit: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
}

/// A typed view of an `rpc` frame's `method` + `params`.
#[derive(Debug, Clone, PartialEq)]
pub enum RpcCall {
    Search(SearchParams),
    Suggest(SuggestParams),
    Open(OpenParams),
    Artist(ArtistParams),
    Library(LibraryParams),
    /// `lyrics` takes `{}`; anything an object carries is ignored.
    Lyrics,
}

impl RpcCall {
    /// Parse `params` according to `method`.
    pub fn parse(method: RpcMethod, params: &Value) -> Result<Self, serde_json::Error> {
        let p = params.clone();
        Ok(match method {
            RpcMethod::Search => Self::Search(serde_json::from_value(p)?),
            RpcMethod::Suggest => Self::Suggest(serde_json::from_value(p)?),
            RpcMethod::Open => Self::Open(serde_json::from_value(p)?),
            RpcMethod::Artist => Self::Artist(serde_json::from_value(p)?),
            RpcMethod::Library => Self::Library(serde_json::from_value(p)?),
            RpcMethod::Lyrics => {
                serde_json::from_value::<serde_json::Map<String, Value>>(p)?;
                Self::Lyrics
            }
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemKind {
    Song,
    Video,
    Album,
    Playlist,
    Artist,
}

/// A search / artist-page entry. For `song`/`video`, `id` is the videoId.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub kind: ItemKind,
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub thumbnail: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    #[serde(default, with = "lenient::opt_u64")]
    pub duration_ms: Option<u64>,
    pub has_video: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shelf {
    pub title: String,
    pub items: Vec<Item>,
}

/// `search` and `artist` results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShelvesResult {
    pub shelves: Vec<Shelf>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SuggestResult {
    pub suggestions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenResult {
    pub title: String,
    pub tracks: Vec<TrackRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LibraryResult {
    #[serde(with = "lenient::u64")]
    pub total: u64,
    pub tracks: Vec<TrackRef>,
}

/// One timed word of a lyric line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricWord {
    #[serde(with = "lenient::u64")]
    pub start_ms: u64,
    #[serde(with = "lenient::u64")]
    pub end_ms: u64,
    pub text: String,
}

/// One line of lyrics. `start_ms` is `null` when the lyrics aren't synced;
/// `text` may be `""` for an instrumental gap; `words` is `null` unless the
/// source has word timing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricLine {
    #[serde(with = "lenient::opt_u64")]
    pub start_ms: Option<u64>,
    pub text: String,
    pub words: Option<Vec<LyricWord>>,
}

/// The `lyrics` rpc result (§3.8): the lyrics of `track`, the player's
/// current track when it answered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lyrics {
    pub track: Option<TrackRef>,
    pub found: bool,
    pub synced: bool,
    pub source: Option<String>,
    pub lines: Vec<LyricLine>,
}

// ---------------------------------------------------------------------- viz

/// What a player's audio looks like right now (§3.11).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VizFrame {
    /// HUB-clock epoch ms at which this frame's audio is heard.
    #[serde(with = "lenient::u64")]
    pub heard_at: u64,
    /// The playback position the frame describes.
    #[serde(with = "lenient::u64")]
    pub position_ms: u64,
    /// Exactly [`VIZ_BANDS`] values in `0..1`, log-spaced ~30 Hz → 16 kHz,
    /// low first.
    pub bands: Vec<f64>,
    /// Overall loudness `0..1`.
    pub level: f64,
    /// Bass-onset strength `0..1` (`0` = no onset this frame).
    pub beat: f64,
}

impl VizFrame {
    /// Whether the frame obeys the contract's shape: 32 bands, every number
    /// finite and inside `0..=1`.
    pub fn is_valid(&self) -> bool {
        let unit = |v: &f64| v.is_finite() && (0.0..=1.0).contains(v);
        self.bands.len() == VIZ_BANDS
            && self.bands.iter().all(unit)
            && unit(&self.level)
            && unit(&self.beat)
    }
}

// ------------------------------------------------------------------ frames

/// Every WebSocket frame, tagged by `t`. Direction-specific fields (`to` vs
/// `from`, the hub's `deviceId` on `state`) are optional so one type covers
/// both legs of each relay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum Frame {
    /// client → hub, the first frame on every socket.
    Hello {
        v: u32,
        token: String,
        device: DeviceHello,
    },
    /// hub → client, after a valid `hello`.
    Welcome {
        v: u32,
        device_id: String,
        #[serde(with = "lenient::u64")]
        server_time: u64,
        devices: Vec<DeviceInfo>,
        active_device_id: Option<String>,
        states: BTreeMap<String, PlayerState>,
    },
    /// hub → clients, whenever the device list or the active device changes.
    Devices {
        devices: Vec<DeviceInfo>,
        active_device_id: Option<String>,
    },
    /// player → hub (no `deviceId`, no `at`) and hub → clients (both set).
    State {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        device_id: Option<String>,
        state: PlayerState,
    },
    Error {
        code: String,
        message: String,
    },
    Select {
        device_id: Option<String>,
    },
    Cmd {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<String>,
        cmd: Command,
    },
    Ack {
        id: String,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<AckError>,
    },
    Rpc {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<String>,
        method: RpcMethod,
        params: Value,
    },
    RpcResult {
        id: String,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<RpcError>,
    },
    Ping {
        id: String,
    },
    Pong {
        id: String,
        #[serde(with = "lenient::u64")]
        server_time: u64,
    },
    /// controller → hub (`to`) and hub → player (no `to`; the hub's 0 ↔ 1
    /// subscriber edge). v1.1.
    VizSub {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<String>,
        on: bool,
    },
    /// player → hub (no `deviceId`) and hub → each subscribed controller
    /// (`deviceId` = the player). v1.1.
    Viz {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        device_id: Option<String>,
        frame: VizFrame,
    },
}

impl Frame {
    /// The wire tag (`t`) of this frame.
    pub fn tag(&self) -> &'static str {
        match self {
            Frame::Hello { .. } => "hello",
            Frame::Welcome { .. } => "welcome",
            Frame::Devices { .. } => "devices",
            Frame::State { .. } => "state",
            Frame::Error { .. } => "error",
            Frame::Select { .. } => "select",
            Frame::Cmd { .. } => "cmd",
            Frame::Ack { .. } => "ack",
            Frame::Rpc { .. } => "rpc",
            Frame::RpcResult { .. } => "rpc_result",
            Frame::Ping { .. } => "ping",
            Frame::Pong { .. } => "pong",
            Frame::VizSub { .. } => "viz_sub",
            Frame::Viz { .. } => "viz",
        }
    }

    /// The `ack` a player owes for a command, on failure.
    pub fn ack_error(id: impl Into<String>, error: AckError) -> Self {
        Frame::Ack {
            id: id.into(),
            ok: false,
            error: Some(error),
        }
    }

    /// The `rpc_result` a player owes for an rpc, on failure.
    pub fn rpc_error(id: impl Into<String>, error: RpcError) -> Self {
        Frame::RpcResult {
            id: id.into(),
            ok: false,
            result: None,
            error: Some(error),
        }
    }
}

// -------------------------------------------------------------------- rest

/// `POST /connect/links` and `/connect/links/device` response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkTicket {
    pub link_id: String,
    pub code: String,
    pub qr: String,
    pub secret: String,
    pub expires_at: String,
}

/// `GET /connect/links/:code` response — what the approver is asked about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkPreview {
    pub kind: DeviceKind,
    pub name: String,
    pub platform: Option<String>,
    pub created_at: String,
}

/// `POST /connect/links/:linkId/claim` response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "status",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
pub enum LinkClaim {
    Pending,
    Denied,
    Expired,
    Approved {
        access_token: String,
        refresh_token: String,
        #[serde(with = "lenient::u64")]
        expires_in: u64,
        device_id: String,
        user_id: String,
    },
}

/// `GET /connect/devices` response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevicesResponse {
    pub devices: Vec<DeviceInfo>,
    pub active_device_id: Option<String>,
}

/// `POST /connect/links/device` body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceLinkRequest {
    pub kind: DeviceKind,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    pub install_id: String,
}

/// `POST /connect/links/:linkId/claim` body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClaimRequest {
    pub secret: String,
}

/// The 409 body of `approve` (`{code:"BOTH_HAVE_ACCOUNTS"}`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConflictBody {
    pub code: String,
}

// -------------------------------------------------------- lenient numbers

/// Integer fields that also accept a JSON float (rounded, clamped at 0).
mod lenient {
    use serde::{Deserialize, Deserializer, Serializer};

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Num {
        U(u64),
        I(i64),
        F(f64),
    }

    fn to_u64<E: serde::de::Error>(n: Num) -> Result<u64, E> {
        match n {
            Num::U(v) => Ok(v),
            Num::I(v) => Ok(v.max(0) as u64),
            Num::F(v) if v.is_finite() => Ok(v.round().max(0.0) as u64),
            Num::F(_) => Err(E::custom("expected a finite number")),
        }
    }

    pub mod u64 {
        use super::*;

        pub fn serialize<S: Serializer>(v: &u64, s: S) -> Result<S::Ok, S::Error> {
            s.serialize_u64(*v)
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
            to_u64(Num::deserialize(d)?)
        }
    }

    pub mod opt_u64 {
        use super::*;

        pub fn serialize<S: Serializer>(v: &Option<u64>, s: S) -> Result<S::Ok, S::Error> {
            match v {
                Some(v) => s.serialize_u64(*v),
                None => s.serialize_none(),
            }
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
            Option::<Num>::deserialize(d)?.map(to_u64).transpose()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn float_milliseconds_are_accepted_and_emitted_as_integers() {
        let cmd: Command =
            serde_json::from_value(json!({"type": "seek", "positionMs": 90000.6})).unwrap();
        assert_eq!(cmd, Command::Seek { position_ms: 90001 });
        assert_eq!(
            serde_json::to_value(&cmd).unwrap(),
            json!({"type": "seek", "positionMs": 90001})
        );
    }

    #[test]
    fn negative_or_non_finite_milliseconds_never_wrap() {
        let cmd: Command =
            serde_json::from_value(json!({"type": "seek", "positionMs": -5})).unwrap();
        assert_eq!(cmd, Command::Seek { position_ms: 0 });
    }

    #[test]
    fn local_refs_always_emit_a_null_art_url() {
        let r: TrackRef = serde_json::from_value(json!({
            "src": "local", "deviceId": "d", "id": "/m/a.flac", "title": "A",
            "artist": null, "album": null, "durationMs": null,
            "artUrl": "file:///never/leaves/the/device.jpg"
        }))
        .unwrap();
        assert_eq!(serde_json::to_value(&r).unwrap()["artUrl"], Value::Null);
    }

    #[test]
    fn nullable_fields_serialize_as_null_but_optional_ones_are_omitted() {
        let ack = Frame::Ack {
            id: "c".into(),
            ok: true,
            error: None,
        };
        assert_eq!(
            serde_json::to_value(&ack).unwrap(),
            json!({"t": "ack", "id": "c", "ok": true})
        );
        let sel = Frame::Select { device_id: None };
        assert_eq!(
            serde_json::to_value(&sel).unwrap(),
            json!({"t": "select", "deviceId": null})
        );
    }

    #[test]
    fn rpc_params_parse_by_method() {
        let call = RpcCall::parse(
            RpcMethod::Library,
            &json!({"offset": 0, "limit": 100, "query": "sauti"}),
        )
        .unwrap();
        assert_eq!(
            call,
            RpcCall::Library(LibraryParams {
                offset: 0,
                limit: 100,
                query: Some("sauti".into())
            })
        );
        assert!(
            RpcCall::parse(RpcMethod::Search, &json!({"query": "x", "filter": "top"})).is_err()
        );
    }

    #[test]
    fn unknown_frame_tags_and_commands_are_rejected() {
        assert!(serde_json::from_value::<Frame>(json!({"t": "teleport"})).is_err());
        assert!(serde_json::from_value::<Frame>(
            json!({"t": "cmd", "id": "c", "from": "w", "cmd": {"type": "dance"}})
        )
        .is_err());
    }
}
