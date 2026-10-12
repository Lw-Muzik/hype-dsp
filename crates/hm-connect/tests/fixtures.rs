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
    // The contract ships 46; a silently missing copy must not pass as green.
    assert_eq!(names.len(), 46, "fixture count changed: {names:?}");

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
