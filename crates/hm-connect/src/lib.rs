//! Hype Connect for the desktop player.
//!
//! A browser (the controller) picks one of the user's running Hype Muzik apps
//! and plays music there; this crate is the desktop's wire to the hub that
//! relays it. Contract: `hypemuzik-management/backend/docs/hype-connect.md`.
//!
//! * [`protocol`] — every frame, REST body and rpc payload as serde types;
//! * [`client`] — the socket supervisor: `hello`/`welcome`, close-code
//!   reactions, backoff, inbound/outbound channels;
//! * [`policy`] — close-code classification and the backoff schedule;
//! * [`token`] — the [`TokenProvider`] seam over the app's account session;
//! * [`rest`] — link / device REST paths and the WebSocket URL.
//!
//! Semantics (what a `play` means, what goes in `state`) live in the app's
//! TypeScript; this crate only moves validated frames.

pub mod client;
pub mod policy;
pub mod protocol;
pub mod rest;
mod tls;
pub mod token;

pub use client::{spawn, ClientConfig, ClientEvent, ConnectHandle, ConnectionStatus, SendError};
pub use policy::{Backoff, StopReason};
pub use protocol::{Cap, DeviceHello, DeviceKind, Frame};
pub use token::{Renewal, TokenProvider};
