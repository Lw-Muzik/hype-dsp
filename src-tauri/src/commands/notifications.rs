//! Broadcast notifications from the admin (`/notifications/feed`).
//!
//! The poll runs here, on a std thread, not in the webview: a hidden window's
//! timers are throttled and the app spends hours in the tray (closing hides).
//! It fetches shortly after launch, then every 15 minutes, shows a native OS
//! notification per new item (one summary when more than [`SUMMARY_ABOVE`]
//! arrive at once), reports `delivered`, caches the feed for the top-bar bell
//! and emits [`UPDATED_EVENT`] so the bell refreshes live. Network failures are
//! silent; the next poll retries because `since` only advances on success.
//!
//! Blocking `reqwest` is fine on our own thread. The Tauri commands below go
//! through `crate::blocking::io` (see that module for why).
//!
//! `opened` is reported only when the user opens an item in the bell. A click on
//! the native toast cannot be detected reliably on desktop with
//! `tauri-plugin-notification` (no click callback on macOS/Windows), so we do
//! not fake `opened` from it.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use super::install::{api_base, resolve_install_id};

const FILE_NAME: &str = "notifications.json";
const FIRST_POLL_DELAY: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_secs(15 * 60);
/// More new items than this at once collapse into a single summary toast.
const SUMMARY_ABOVE: usize = 3;
/// The bell shows 14 days and the server caps a poll at 20; keep a little more.
const CACHE_MAX: usize = 50;
pub const UPDATED_EVENT: &str = "notifications://updated";

static STATE_LOCK: Mutex<()> = Mutex::new(());

/// One item as the server sends it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FeedItem {
    id: String,
    title: String,
    body: String,
    #[serde(default)]
    link: Option<String>,
    created_at: String,
}

#[derive(Debug, Deserialize)]
struct FeedResponse {
    #[serde(default)]
    items: Vec<FeedItem>,
}

/// A cached item plus whether the user has opened it. Serialised to the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedItem {
    pub id: String,
    pub title: String,
    pub body: String,
    pub link: Option<String>,
    pub created_at: String,
    #[serde(default)]
    pub read: bool,
}

/// Persisted in app data.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    /// Newest `createdAt` seen; `None` on first run (server default: 7 days).
    #[serde(default)]
    since: Option<String>,
    /// Newest first.
    #[serde(default)]
    items: Vec<CachedItem>,
    /// OS permission has been requested once already.
    #[serde(default)]
    permission_asked: bool,
}

/// What to show natively for a batch of new items.
#[derive(Debug, PartialEq)]
enum Toasts {
    Each(Vec<(String, String)>),
    Summary(String, String),
}

fn feed_url(base: &str, since: Option<&str>) -> String {
    let mut url = format!("{}/notifications/feed?platform=desktop", base.trim_end_matches('/'));
    if let Some(s) = since {
        // ISO timestamps contain ':' and '+' which must be escaped in a query.
        let enc: String = url::form_urlencoded::byte_serialize(s.as_bytes()).collect();
        url.push_str("&since=");
        url.push_str(&enc);
    }
    url
}

fn parse_feed(body: &str) -> Result<Vec<FeedItem>, serde_json::Error> {
    serde_json::from_str::<FeedResponse>(body).map(|r| r.items)
}

/// Only an https link may leave the app.
fn safe_link(link: &Option<String>) -> Option<&str> {
    let l = link.as_deref()?.trim();
    (l.len() <= 500 && l.starts_with("https://") && url::Url::parse(l).is_ok()).then_some(l)
}

fn toasts_for(new: &[FeedItem]) -> Option<Toasts> {
    match new.len() {
        0 => None,
        n if n > SUMMARY_ABOVE => {
            Some(Toasts::Summary("Hype Muzik".into(), format!("{n} new messages from Hype Muzik")))
        }
        _ => Some(Toasts::Each(new.iter().map(|i| (i.title.clone(), i.body.clone())).collect())),
    }
}

/// Fold a poll into the stored state. Returns the genuinely new items (not
/// already cached). `since` becomes the max `createdAt` seen. The server
/// emits fixed-width UTC ISO strings, so lexicographic order is time order.
fn apply(stored: &mut Stored, items: Vec<FeedItem>) -> Vec<FeedItem> {
    let mut fresh = Vec::new();
    for it in items {
        if stored.since.as_deref().map_or(true, |s| it.created_at.as_str() > s) {
            stored.since = Some(it.created_at.clone());
        }
        if stored.items.iter().any(|c| c.id == it.id) {
            continue;
        }
        stored.items.push(CachedItem {
            id: it.id.clone(),
            title: it.title.clone(),
            body: it.body.clone(),
            link: it.link.clone(),
            created_at: it.created_at.clone(),
            read: false,
        });
        fresh.push(it);
    }
    stored.items.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    stored.items.truncate(CACHE_MAX);
    fresh
}

fn state_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join(FILE_NAME))
}

fn load(app: &AppHandle) -> Stored {
    state_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save(app: &AppHandle, s: &Stored) {
    if let Some(p) = state_path(app) {
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string(s) {
            let _ = std::fs::write(p, json);
        }
    }
}

fn http() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap_or_else(|_| reqwest::blocking::Client::new())
}

fn event_body(install_id: &str, kind: &str) -> serde_json::Value {
    serde_json::json!({ "installId": install_id, "platform": "desktop", "kind": kind })
}

fn report(client: &reqwest::blocking::Client, base: &str, app: &AppHandle, id: &str, kind: &str) {
    let url = format!("{}/notifications/{id}/events", base.trim_end_matches('/'));
    let _ = client.post(url).json(&event_body(&resolve_install_id(app), kind)).send();
}

/// Ask the OS once. Desktop plugins that never prompt report Granted.
fn ensure_permission(app: &AppHandle) -> bool {
    use tauri_plugin_notification::PermissionState;
    let n = app.notification();
    match n.permission_state() {
        Ok(PermissionState::Granted) => true,
        Ok(PermissionState::Denied) => false,
        _ => {
            let _g = STATE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let mut st = load(app);
            if st.permission_asked {
                return false;
            }
            st.permission_asked = true;
            save(app, &st);
            matches!(n.request_permission(), Ok(PermissionState::Granted))
        }
    }
}

fn poll_once(app: &AppHandle, client: &reqwest::blocking::Client, base: &str) {
    let since = {
        let _g = STATE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        load(app).since
    };
    let Ok(resp) = client.get(feed_url(base, since.as_deref())).send() else { return };
    if !resp.status().is_success() {
        return;
    }
    let Ok(body) = resp.text() else { return };
    let Ok(items) = parse_feed(&body) else { return };

    let fresh = {
        let _g = STATE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut st = load(app);
        let fresh = apply(&mut st, items);
        save(app, &st);
        fresh
    };
    if fresh.is_empty() {
        return;
    }

    if ensure_permission(app) {
        match toasts_for(&fresh) {
            Some(Toasts::Each(list)) => {
                for (title, body) in list {
                    let _ = app.notification().builder().title(title).body(body).show();
                }
            }
            Some(Toasts::Summary(title, body)) => {
                let _ = app.notification().builder().title(title).body(body).show();
            }
            None => {}
        }
        for it in &fresh {
            report(client, base, app, &it.id, "delivered");
        }
    }
    let _ = app.emit(UPDATED_EVENT, fresh.len());
}

/// Start the background poller. Fire-and-forget.
pub fn spawn_poller(app: AppHandle) {
    std::thread::Builder::new()
        .name("hm-notifications".into())
        .spawn(move || {
            let client = http();
            let base = api_base();
            std::thread::sleep(FIRST_POLL_DELAY);
            loop {
                poll_once(&app, &client, &base);
                std::thread::sleep(POLL_INTERVAL);
            }
        })
        .ok();
}

/// The cached feed (newest first) with read state. The UI filters to 14 days.
#[tauri::command(async)]
pub fn notifications_list(app: AppHandle) -> Vec<CachedItem> {
    crate::blocking::io(|| {
        let _g = STATE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        load(&app).items
    })
}

/// The user opened an item in the bell: mark it read, report `opened`, and open
/// its https link in the default browser. Returns the updated list.
#[tauri::command(async)]
pub fn notifications_open(app: AppHandle, id: String) -> Vec<CachedItem> {
    crate::blocking::io(|| {
        let (link, first_open, items) = {
            let _g = STATE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let mut st = load(&app);
            let mut link = None;
            let mut first_open = false;
            if let Some(it) = st.items.iter_mut().find(|i| i.id == id) {
                first_open = !it.read;
                it.read = true;
                link = safe_link(&it.link).map(str::to_string);
            }
            save(&app, &st);
            (link, first_open, st.items)
        };
        if let Some(l) = link {
            crate::cloud::open_url(&l);
        }
        if first_open {
            report(&http(), &api_base(), &app, &id, "opened");
        }
        items
    })
}

/// Mark everything read (no events: nothing was opened).
#[tauri::command(async)]
pub fn notifications_mark_all_read(app: AppHandle) -> Vec<CachedItem> {
    crate::blocking::io(|| {
        let _g = STATE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut st = load(&app);
        st.items.iter_mut().for_each(|i| i.read = true);
        save(&app, &st);
        st.items
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, at: &str) -> FeedItem {
        FeedItem {
            id: id.into(),
            title: format!("t{id}"),
            body: format!("b{id}"),
            link: None,
            created_at: at.into(),
        }
    }

    #[test]
    fn feed_url_without_since_uses_server_default() {
        assert_eq!(
            feed_url("https://api.hypemuzik.com/api", None),
            "https://api.hypemuzik.com/api/notifications/feed?platform=desktop"
        );
        assert!(feed_url("https://x/api/", None).starts_with("https://x/api/notifications/feed"));
    }

    #[test]
    fn feed_url_encodes_since() {
        let u = feed_url("https://x/api", Some("2026-10-27T10:00:00.000Z"));
        assert_eq!(
            u,
            "https://x/api/notifications/feed?platform=desktop&since=2026-10-27T10%3A00%3A00.000Z"
        );
    }

    #[test]
    fn parses_the_contract_payload() {
        let body = r#"{"items":[
          {"id":"a","title":"Hi","body":"There","link":"https://hypemuzik.com","createdAt":"2026-10-27T10:00:00.000Z"},
          {"id":"b","title":"Yo","body":"Dude","link":null,"createdAt":"2026-10-27T11:00:00.000Z","extra":1}]}"#;
        let items = parse_feed(body).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].link.as_deref(), Some("https://hypemuzik.com"));
        assert_eq!(items[1].link, None);
        assert_eq!(parse_feed(r#"{"items":[]}"#).unwrap().len(), 0);
        assert!(parse_feed("not json").is_err());
    }

    #[test]
    fn since_persists_as_newest_created_at_and_dedupes() {
        let mut st = Stored::default();
        let fresh = apply(&mut st, vec![item("a", "2026-10-27T10:00:00.000Z"), item("b", "2026-10-27T11:00:00.000Z")]);
        assert_eq!(fresh.len(), 2);
        assert_eq!(st.since.as_deref(), Some("2026-10-27T11:00:00.000Z"));
        assert_eq!(st.items[0].id, "b"); // newest first

        // Round-trips through the file format.
        let back: Stored = serde_json::from_str(&serde_json::to_string(&st).unwrap()).unwrap();
        assert_eq!(back.since, st.since);

        // A repeat poll adds nothing and never moves `since` backwards.
        let fresh = apply(&mut st, vec![item("a", "2026-10-27T10:00:00.000Z")]);
        assert!(fresh.is_empty());
        assert_eq!(st.since.as_deref(), Some("2026-10-27T11:00:00.000Z"));
        assert_eq!(st.items.len(), 2);
    }

    #[test]
    fn an_empty_poll_keeps_since() {
        let mut st = Stored { since: Some("2026-10-27T11:00:00.000Z".into()), ..Default::default() };
        assert!(apply(&mut st, vec![]).is_empty());
        assert_eq!(st.since.as_deref(), Some("2026-10-27T11:00:00.000Z"));
    }

    #[test]
    fn summary_only_above_three() {
        assert_eq!(toasts_for(&[]), None);
        let three: Vec<_> = (0..3).map(|i| item(&i.to_string(), "2026-10-27T10:00:00.000Z")).collect();
        match toasts_for(&three) {
            Some(Toasts::Each(v)) => assert_eq!(v.len(), 3),
            other => panic!("expected 3 toasts, got {other:?}"),
        }
        let four: Vec<_> = (0..4).map(|i| item(&i.to_string(), "2026-10-27T10:00:00.000Z")).collect();
        match toasts_for(&four) {
            Some(Toasts::Summary(_, body)) => assert!(body.starts_with("4 new")),
            other => panic!("expected summary, got {other:?}"),
        }
    }

    #[test]
    fn only_https_links_open() {
        assert_eq!(safe_link(&Some("https://hypemuzik.com/x".into())), Some("https://hypemuzik.com/x"));
        assert_eq!(safe_link(&Some("http://hypemuzik.com".into())), None);
        assert_eq!(safe_link(&Some("file:///etc/passwd".into())), None);
        assert_eq!(safe_link(&Some("javascript:alert(1)".into())), None);
        assert_eq!(safe_link(&None), None);
    }

    #[test]
    fn event_body_matches_contract() {
        let b = event_body("abc", "delivered");
        assert_eq!(b["installId"], "abc");
        assert_eq!(b["platform"], "desktop");
        assert_eq!(b["kind"], "delivered");
    }
}
