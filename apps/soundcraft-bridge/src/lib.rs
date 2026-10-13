//! `soundcraft-bridge`: Wild Animal's headless SoundCraft backend. It runs upstream's engine
//! without a window and speaks upstream's control protocol (newline-delimited JSON) over stdio,
//! and over TCP with `--control <port>`. See `README.md` for the protocol reference.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod bridge;
pub mod logger;
pub mod serve;
pub mod wire;

pub use bridge::Bridge;
