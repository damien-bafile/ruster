//! A minimal, non-blocking Language Server Protocol client.
//!
//! Uses plain std threads and channels (no async runtime): a background thread
//! reads framed JSON-RPC messages off the server's stdout into an mpsc channel,
//! which the application drains each frame via [`client::LspClient::poll`].

pub mod client;
pub mod transport;

pub use client::LspClient;
pub use transport::{ServerMessage, classify, read_message, write_message};
