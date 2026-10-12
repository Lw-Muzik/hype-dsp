# Hype Connect protocol fixtures (copy)

A verbatim copy of `hypemuzik-management/backend/connect-protocol/fixtures/` —
the normative example frames for `backend/docs/hype-connect.md` (protocol v1).

Both test suites in this repo read this one folder:

- `crates/hm-connect/tests/fixtures.rs` — every fixture parses into the Rust
  protocol types and re-serializes to the same JSON.
- `src/features/connect/protocol.test.ts` — every WebSocket frame fixture
  decodes into the TypeScript protocol types and re-encodes to the same JSON.

Never edit these files here. When the protocol changes upstream, re-copy the
folder (`cp ../hypemuzik-management/backend/connect-protocol/fixtures/*.json
connect-protocol/fixtures/`) and fix whatever the tests then flag.
