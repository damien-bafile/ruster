//! A minimal, non-blocking Language Server Protocol client.
//!
//! Uses plain std threads and channels (no async runtime): a background thread
//! reads framed JSON-RPC messages off the server's stdout into an mpsc channel,
//! which the application drains each frame via [`client::LspClient::poll`].

pub mod client;
pub mod manager;
pub mod position;
pub mod protocol;
pub mod registry;
pub mod results;
pub mod transport;

pub use client::LspClient;
pub use manager::{LspManager, RoutedMessage};
pub use position::{offset_to_position, position_to_offset, LspPosition};
pub use registry::{default_server, language_id, ServerConfig};
pub use results::{
    apply_edits, parse_call_hierarchy_calls, parse_call_hierarchy_prepare, parse_diagnostics,
    parse_document_symbols, parse_hover, parse_locations, parse_text_edits, parse_workspace_edit,
    parse_workspace_symbols, CallEntry, Diagnostic, Location, SymbolEntry, TextEdit,
};
pub use transport::{classify, read_message, write_message, ServerMessage};
