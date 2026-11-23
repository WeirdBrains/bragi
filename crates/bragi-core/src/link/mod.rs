//! The desktop link: a phone runs code mode against a project on a desktop.
//!
//! The desktop (`bragi linker`) serves the code tools ([`workspace`]) on a Weirdnet node of
//! its own ([`server`]); the phone ([`client`]) pairs once from a QR code or typed code and then
//! calls it directly over the LAN or VPN, or through a Weirdnet relay. Every connection is
//! Noise-encrypted end to end and authenticated by both peer ids; every call also carries the
//! phone's token. Paths are confined to the project roots the desktop shares ([`confine`]).
//! The protocol is written up in `docs/desktop-link.md`.

pub mod client;
pub mod confine;
pub mod proto;
pub mod server;
pub mod workspace;

pub use client::{DesktopView, LinkClient, Route};
pub use confine::Root;
pub use proto::{Entry, Hello, PairLink, RootInfo, ShellFrame, ShellRequest, ShortCode, WireChange};
pub use server::{Confirm, DeviceView, LinkEvent, LinkServer, Offer, ServerOptions, Settings, ShellAsk};

#[cfg(test)]
mod tests;
