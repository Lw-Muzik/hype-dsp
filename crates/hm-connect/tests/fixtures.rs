//! Every normative fixture parses into its typed model and re-serializes to
//! the same JSON (`connect-protocol/README.md`).

use std::path::PathBuf;

use hm_connect::protocol::*;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../connect-protocol/fixtures")
}

fn load(name: &str) -> Value {
    let path = fixtures_dir().join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// Parse `json` as `T`, serialize it back, and demand identical JSON.
fn round_trip<T: DeserializeOwned + Serialize>(name: &str, json: &Value) -> T {
    let typed: T = serde_json::from_value(json.clone())
        .unwrap_or_else(|e| panic!("{name} didn't parse as {}: {e}", std::any::type_name::<T>()));
    let back = serde_json::to_value(&typed).unwrap();
    assert_eq!(&back, json, "{name} changed on the round trip");
    typed
}

#[test]
fn every_fixture_round_trips() {
    let mut names: Vec<String> = std::fs::read_dir(fixtures_dir())
        .expect("fixtures dir")
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.ends_with(".json"))
        .collect();
    names.sort();
    // The contract ships 56 (46 v1 + 10 v1.1); a silently missing copy must
    // not pass as green.
    assert_eq!(names.len(), 56, "fixture count changed: {names:?}");

    for name in &names {
        let json = load(name);
        match name.as_str() {
            "rest.devices.json" => {
                round_trip::<DevicesResponse>(name, &json);
            }
            "rest.link-ticket.json" => {
                round_trip::<LinkTicket>(name, &json);
            }
            "rest.link-preview.json" => {
                round_trip::<LinkPreview>(name, &json);
            }
            n if n.starts_with("rest.link-claim.") => {
                round_trip::<LinkClaim>(name, &json);
            }
            n if n.starts_with("rest.") => panic!("unmapped REST fixture {n}"),
            _ => {
                let frame = round_trip::<Frame>(name, &json);
                let tag = name.split('.').next().unwrap();
                assert_eq!(frame.tag(), tag, "{name} parsed as the wrong frame");
            }
        }
    }
}

#[test]
fn rpc_payloads_round_trip_typed() {
    for (name, method) in [
        ("rpc.search.from.json", RpcMethod::Search),
        ("rpc.suggest.from.json", RpcMethod::Suggest),
        ("rpc.open.from.json", RpcMethod::Open),
        ("rpc.artist.from.json", RpcMethod::Artist),
        ("rpc.library.from.json", RpcMethod::Library),
        ("rpc.lyrics.from.json", RpcMethod::Lyrics),
    ] {
        let Frame::Rpc {
            method: m, params, ..
        } = round_trip::<Frame>(name, &load(name))
        else {
            panic!("{name} is not an rpc");
        };
        assert_eq!(m, method, "{name}");
        RpcCall::parse(m, &params).unwrap_or_else(|e| panic!("{name} params: {e}"));
    }

    let result = |name: &str| match round_trip::<Frame>(name, &load(name)) {
        Frame::RpcResult {
            ok: true,
            result: Some(r),
            ..
        } => r,
        other => panic!("{name}: {other:?}"),
    };
    round_trip::<ShelvesResult>("search result", &result("rpc_result.search.json"));
    round_trip::<ShelvesResult>("artist result", &result("rpc_result.artist.json"));
    round_trip::<SuggestResult>("suggest result", &result("rpc_result.suggest.json"));
    round_trip::<OpenResult>("open result", &result("rpc_result.open.json"));
    let lib = round_trip::<LibraryResult>("library result", &result("rpc_result.library.json"));
    assert!(lib.tracks.iter().all(|t| matches!(t, TrackRef::Local(_))));
}

#[test]
fn lyrics_results_round_trip_typed() {
    let result = |name: &str| match round_trip::<Frame>(name, &load(name)) {
        Frame::RpcResult {
            ok: true,
            result: Some(r),
            ..
        } => round_trip::<Lyrics>(name, &r),
        other => panic!("{name}: {other:?}"),
    };

    let synced = result("rpc_result.lyrics.json");
    assert!(synced.found && synced.synced);
    assert_eq!(synced.source.as_deref(), Some("lrclib"));
    assert!(matches!(synced.track, Some(TrackRef::Yt(_))));
    assert_eq!(synced.lines[0].start_ms, Some(18_000));
    assert_eq!(synced.lines[0].words.as_ref().map(Vec::len), Some(5));
    assert_eq!(synced.lines[1].words, None, "no word timing is null");
    assert_eq!(
        synced.lines[2].text, "",
        "an instrumental gap keeps its line"
    );

    let plain = result("rpc_result.lyrics.plain.json");
    assert!(plain.found && !plain.synced);
    assert!(plain.lines.iter().all(|l| l.start_ms.is_none()));

    let none = result("rpc_result.lyrics.none.json");
    assert!(!none.found && none.lines.is_empty() && none.source.is_none());
    assert!(none.track.is_some(), "a miss still names the track");
}

#[test]
fn viz_frames_round_trip_and_keep_their_direction() {
    match round_trip::<Frame>("viz.player", &load("viz.player.json")) {
        Frame::Viz {
            device_id: None,
            frame,
        } => {
            assert!(frame.is_valid());
            assert_eq!(frame.bands.len(), 32);
            assert_eq!(frame.heard_at, 1_760_295_600_350);
            assert_eq!(frame.position_ms, 61_400);
        }
        other => panic!("{other:?}"),
    }
    match round_trip::<Frame>("viz.stamped", &load("viz.stamped.json")) {
        Frame::Viz {
            device_id: Some(_), ..
        } => {}
        other => panic!("{other:?}"),
    }
    // The hub tells a player `on`; a controller names its target.
    assert!(matches!(
        round_trip::<Frame>("viz_sub.player.on", &load("viz_sub.player.on.json")),
        Frame::VizSub { to: None, on: true }
    ));
    assert!(matches!(
        round_trip::<Frame>("viz_sub.player.off", &load("viz_sub.player.off.json")),
        Frame::VizSub {
            to: None,
            on: false
        }
    ));
    assert!(matches!(
        round_trip::<Frame>("viz_sub.to", &load("viz_sub.to.json")),
        Frame::VizSub {
            to: Some(_),
            on: true
        }
    ));
}

#[test]
fn a_viz_frame_off_contract_is_invalid() {
    let Frame::Viz { frame: good, .. } =
        round_trip::<Frame>("viz.player", &load("viz.player.json"))
    else {
        unreachable!()
    };
    assert!(good.is_valid());

    let mut short = good.clone();
    short.bands.pop();
    assert!(!short.is_valid(), "31 bands");

    let mut long = good.clone();
    long.bands.push(0.5);
    assert!(!long.is_valid(), "33 bands");

    let mut loud = good.clone();
    loud.bands[3] = 1.01;
    assert!(!loud.is_valid(), "band above 1");

    let mut negative = good.clone();
    negative.bands[3] = -0.01;
    assert!(!negative.is_valid(), "band below 0");

    let mut nan = good.clone();
    nan.level = f64::NAN;
    assert!(!nan.is_valid(), "NaN level");

    let mut beat = good;
    beat.beat = 2.0;
    assert!(!beat.is_valid(), "beat above 1");
}

#[test]
fn unknown_caps_are_dropped_not_rejected() {
    // hello.mobile.v11 carries the v1.1 caps.
    let Frame::Hello { device, .. } =
        round_trip::<Frame>("hello.mobile.v11", &load("hello.mobile.v11.json"))
    else {
        unreachable!()
    };
    assert_eq!(
        device.caps,
        vec![Cap::Player, Cap::Yt, Cap::Local, Cap::Lyrics, Cap::Viz]
    );

    // A cap from the future must not break a frame this build understands.
    let mut welcome = load("welcome.json");
    welcome["devices"][0]["caps"] = serde_json::json!(["player", "hologram", "viz", 7, null]);
    let Frame::Welcome { devices, .. } = serde_json::from_value(welcome).expect("tolerant") else {
        unreachable!()
    };
    assert_eq!(devices[0].caps, vec![Cap::Player, Cap::Viz]);

    let mut hello = load("hello.web.json");
    hello["device"]["caps"] = serde_json::json!(["hologram"]);
    let Frame::Hello { device, .. } = serde_json::from_value(hello).expect("tolerant") else {
        unreachable!()
    };
    assert!(device.caps.is_empty());

    // …but a caps field that isn't a list is still a protocol error.
    let mut bad = load("devices.json");
    bad["devices"][0]["caps"] = serde_json::json!("player");
    assert!(serde_json::from_value::<Frame>(bad).is_err());
}

#[test]
fn directional_fields_survive() {
    // hub → player carries `from`; controller → hub carries `to`.
    match round_trip::<Frame>("cmd.play.from", &load("cmd.play.from.json")) {
        Frame::Cmd {
            from: Some(_),
            to: None,
            cmd:
                Command::Play {
                    items,
                    radio: Some(false),
                    position_ms: Some(0),
                    ..
                },
            ..
        } => {
            assert!(matches!(items[2], TrackRef::Local(_)));
        }
        other => panic!("{other:?}"),
    }
    match round_trip::<Frame>("cmd.play.radio", &load("cmd.play.radio.json")) {
        Frame::Cmd {
            cmd:
                Command::Play {
                    radio: Some(true),
                    position_ms: None,
                    ..
                },
            ..
        } => {}
        other => panic!("{other:?}"),
    }
    // A stamped state names its device and carries `at`; a player's own doesn't.
    match round_trip::<Frame>("state.stamped", &load("state.stamped.json")) {
        Frame::State {
            device_id: Some(_),
            state,
        } => assert_eq!(state.at, Some(1_760_295_600_000)),
        other => panic!("{other:?}"),
    }
    match round_trip::<Frame>("state.player", &load("state.player.json")) {
        Frame::State {
            device_id: None,
            state,
        } => {
            assert_eq!(state.at, None);
            assert_eq!(state.queue[1].uid, "q-8");
        }
        other => panic!("{other:?}"),
    }
}
