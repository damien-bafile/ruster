# DAP Debugger — Implementation Plan

> **Status:** delivered; boxes resolved 2026-08-07.
>
> This plan was executed but never ticked as it went, leaving 37 boxes
> that read as outstanding work. They are **plain bullets now, not back-ticked**:
> a box ticked long after the fact asserts a verification that did not happen,
> and this tree has already been bitten by exactly that. The bullets stand as a
> record of what was built.
>
> Evidence it shipped: all 17 identifiers this plan names in backticks exist in
> the tree, and `docs/verification/debugger-{tui.txt,gui.png}` — a live lldb-dap session paused at a breakpoint with its call stack and scopes.


> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a full DAP debugger: new `ruster-dap` crate for protocol/client/session, and debug UI (toolbar, breakpoints, stack, variables, hover) in ruster-tui.

**Architecture:** New `ruster-dap` crate (transport, client, session, config) mirroring `ruster-lsp`. Debug UI in ruster-tui as overlays and gutter modifications (no new windows). Uses the `dap` crate for protocol types.

**Tech Stack:** `dap` crate (sztomi/dap-rs, v0.4.1-alpha1), `serde_json`, `thiserror`, ruster-tui, ruster-core

## Global Constraints

- `dap = "0.4.1-alpha1"` — exact version pin
- Follow `ruster-lsp` transport pattern exactly (Content-Length framing, stdio, reader thread)
- Debug UI uses existing render surface (no new WindowTree nodes for debug panels)
- Do NOT add debug panels as windows — use overlay rendering

---

### Task 1: Scaffold ruster-dap crate

**Files:**
- Create: `crates/ruster-dap/Cargo.toml`
- Create: `crates/ruster-dap/src/lib.rs`
- Create: `crates/ruster-dap/src/transport.rs`
- Create: `crates/ruster-dap/src/client.rs`
- Create: `crates/ruster-dap/src/session.rs`
- Create: `crates/ruster-dap/src/config.rs`
- Modify: `Cargo.toml` (workspace members)

**Interfaces:**
- Produces: workspace crate `ruster-dap` with transport, client, session, config modules

- **Step 1: Create cargo project structure**

```bash
mkdir -p crates/ruster-dap/src
```

Write `crates/ruster-dap/Cargo.toml`:

```toml
[package]
name = "ruster-dap"
version = "0.1.0"
edition = "2021"

[dependencies]
dap = "0.4.1-alpha1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "1"
```

- **Step 2: Register in workspace**

Add to `Cargo.toml` workspace members:

```toml
members = [
    ...
    "crates/ruster-dap",
]
```

- **Step 3: Commit**

```
git add Cargo.toml crates/ruster-dap/
git commit -m "feat(dap): scaffold ruster-dap crate"
```

---

### Task 2: Implement transport.rs — JSON-RPC 2.0 framing

**Files:**
- Implement: `crates/ruster-dap/src/transport.rs`

**Interfaces:**
- Produces: `read_message<R: Read>(r: &mut R) -> Result<ServerMessage>`, `write_message<W: Write>(w: &mut W, msg: &ClientMessage) -> Result<()>`
- `ServerMessage` enum: `Response(Response)`, `Event(Event)`, `Request(Request)`
- `ClientMessage` enum: `Request(Request)`, `Response(Response)`

- **Step 1: Implement Core types and read/write**

```rust
use std::io::{BufRead, BufReader, Read, Write};
use dap::base_message::{BaseMessage, unmarshall, marshall};
use serde_json;

#[derive(Debug)]
pub enum ServerMessage {
    Response(dap::responses::Response),
    Event(dap::events::Event),
    Request(dap::requests::Request),
}

#[derive(Debug)]
pub enum ClientMessage {
    Request(dap::requests::Request),
    Response(dap::responses::Response),
}

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Protocol error: {0}")]
    Protocol(String),
}

pub type Result<T> = std::result::Result<T, TransportError>;

pub fn read_message<R: Read>(reader: &mut R) -> Result<ServerMessage> {
    let base = BaseMessage::read(reader)
        .map_err(|e| TransportError::Protocol(e.to_string()))?;
    let raw = base.into_content();
    let val: serde_json::Value = serde_json::from_str(&raw)?;

    // Determine message type by inspecting the json
    if val.get("id").is_some() && val.get("method").is_some() {
        // Incoming request from server
        let req: dap::requests::Request = serde_json::from_value(val)?;
        Ok(ServerMessage::Request(req))
    } else if val.get("id").is_some() || val.get("success").is_some() {
        let rsp: dap::responses::Response = serde_json::from_value(val)?;
        Ok(ServerMessage::Response(rsp))
    } else {
        let evt: dap::events::Event = serde_json::from_value(val)?;
        Ok(ServerMessage::Event(evt))
    }
}

pub fn write_message<W: Write>(writer: &mut W, msg: &ClientMessage) -> Result<()> {
    let json = match msg {
        ClientMessage::Request(req) => serde_json::to_value(req)?,
        ClientMessage::Response(rsp) => serde_json::to_value(rsp)?,
    };
    let body = serde_json::to_string(&json)?;
    let base = BaseMessage::new(body);
    base.write(writer)
        .map_err(|e| TransportError::Protocol(e.to_string()))?;
    Ok(())
}
```

- **Step 2: Build to verify**

```
cargo build -p ruster-dap 2>&1 | tail -10
```

- **Step 3: Commit**

```
git add crates/ruster-dap/src/transport.rs
git commit -m "feat(dap): implement JSON-RPC transport"
```

---

### Task 3: Implement client.rs — DAP adapter process manager

**Files:**
- Create: `crates/ruster-dap/src/client.rs`

**Interfaces:**
- Produces: `DapClient`
- `DapClient::spawn(config: &AdapterConfig, root: &Path) -> Result<(Self, ThreadJoinHandle)>`
- `DapClient::send_request(req: Request) -> Result<()>`
- `DapClient::send_response(rsp: Response) -> Result<()>`
- `DapClient::poll() -> Option<ServerMessage>`
- `DapClient::shutdown() -> Result<()>`

- **Step 1: Create client module**

```rust
use std::io::{BufReader, BufWriter, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use crate::config::AdapterConfig;
use crate::transport::{self, ClientMessage, ServerMessage};

pub type Result<T> = std::result::Result<T, ClientError>;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Transport error: {0}")]
    Transport(#[from] transport::TransportError),
    #[error("Channel disconnected")]
    ChannelClosed,
}

pub struct DapClient {
    tx: Sender<ClientMessage>,
    rx: Receiver<ServerMessage>,
    child: Option<std::process::Child>,
}

impl DapClient {
    pub fn spawn(config: &AdapterConfig, root: &std::path::Path) -> Result<(Self, JoinHandle<()>)> {
        let mut child = Command::new(&config.command)
            .args(&config.args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;

        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut reader = BufReader::new(stdout);

        let (req_tx, req_rx): (Sender<ClientMessage>, _) = mpsc::channel();
        let (msg_tx, msg_rx): (Sender<ServerMessage>, _) = mpsc::channel();

        // Writer thread: drain requests from channel and write to stdin
        let w_tx = req_tx.clone();
        let writer_h = thread::spawn(move || {
            for msg in req_rx {
                let mut buf = BufWriter::new(&mut stdin);
                if let Err(e) = transport::write_message(&mut buf, &msg) {
                    eprintln!("dap write error: {e}");
                    break;
                }
                buf.flush().ok();
            }
        });

        // Reader thread: loop reading messages from stdout
        let msg_tx_clone = msg_tx.clone();
        let reader_h = thread::spawn(move || {
            loop {
                match transport::read_message(&mut reader) {
                    Ok(msg) => {
                        if msg_tx_clone.send(msg).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        eprintln!("dap read error: {e}");
                        break;
                    }
                }
            }
        });

        let client = DapClient {
            tx: req_tx,
            rx: msg_rx,
            child: Some(child),
        };

        Ok((client, reader_h))
    }

    pub fn send_request(&self, req: dap::requests::Request) -> Result<()> {
        self.tx.send(ClientMessage::Request(req)).map_err(|_| ClientError::ChannelClosed)
    }

    pub fn send_response(&self, rsp: dap::responses::Response) -> Result<()> {
        self.tx.send(ClientMessage::Response(rsp)).map_err(|_| ClientError::ChannelClosed)
    }

    pub fn poll(&self) -> Option<ServerMessage> {
        self.rx.try_recv().ok()
    }

    pub fn shutdown(mut self) -> Result<()> {
        if let Some(ref mut child) = self.child {
            child.kill().ok();
            child.wait().ok();
        }
        Ok(())
    }
}
```

- **Step 2: Build to verify**

```
cargo build -p ruster-dap 2>&1 | tail -10
```

- **Step 3: Commit**

```
git add crates/ruster-dap/src/client.rs
git commit -m "feat(dap): implement DAP client"
```

---

### Task 4: Implement session.rs — Debug session state machine

**Files:**
- Create: `crates/ruster-dap/src/session.rs`

**Interfaces:**
- Produces: `DebugSession`
- `DebugSession::start(config, root) -> Result<Self>`
- `DebugSession::initialize() -> Result<Capabilities>`
- `DebugSession::launch(config_json) -> Result<()>`
- `DebugSession::set_breakpoint(path, line)`, `clear_breakpoint(path, line)`
- `DebugSession::continue_exec()`, `pause()`, `step_over()`, `step_into()`, `step_out()`
- `DebugSession::get_stack_frames(thread_id) -> Result<Vec<StackFrame>>`
- `DebugSession::get_variables(ref_id) -> Result<Vec<Variable>>`
- `DebugSession::evaluate(expr, context) -> Result<String>`
- `DebugSession::poll_events() -> Vec<DapEvent>` (parsed DAP events into simplified enum)

- **Step 1: Implement DebugSession**

```rust
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use dap::types::*;

use crate::client::DapClient;
use crate::config::AdapterConfig;

pub type Result<T> = std::result::Result<T, SessionError>;

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("Client error: {0}")]
    Client(#[from] crate::client::ClientError),
    #[error("Not initialized")]
    NotInitialized,
    #[error("Session not running")]
    NotRunning,
    #[error("DAP error: {0}")]
    Dap(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum SessionState {
    Inactive,
    Initializing,
    Running,
    Paused,
    Terminated,
}

#[derive(Debug, Clone)]
pub enum DapEvent {
    Stopped { reason: String, thread_id: u64 },
    Continued { thread_id: u64 },
    Exited { exit_code: i64 },
    Terminated,
    Output { category: String, output: String },
    BreakpointValidated { id: u64, verified: bool },
    Module { id: u64, name: String },
    Process { name: String, pid: u64 },
    Thread { reason: String, thread_id: u64 },
}

pub struct DebugSession {
    pub client: DapClient,
    pub state: SessionState,
    pub breakpoints: HashMap<(PathBuf, usize), Breakpoint>,
    pub threads: HashMap<u64, Thread>,
    pub stack_frames: Vec<StackFrame>,
    pub scopes: Vec<Scope>,
    pub variable_cache: HashMap<u64, Variable>,
    pub stopped_thread: Option<u64>,
    next_seq: u64,
}

impl DebugSession {
    pub fn start(config: &AdapterConfig, root: &Path) -> Result<Self> {
        let (client, _reader_thread) = DapClient::spawn(config, root)?;
        Ok(DebugSession {
            client,
            state: SessionState::Initializing,
            breakpoints: HashMap::new(),
            threads: HashMap::new(),
            stack_frames: Vec::new(),
            scopes: Vec::new(),
            variable_cache: HashMap::new(),
            stopped_thread: None,
            next_seq: 1,
        })
    }

    pub fn next_seq(&mut self) -> u64 {
        let s = self.next_seq;
        self.next_seq += 1;
        s
    }

    pub fn send_initialize(&mut self) -> Result<Capabilities> {
        use dap::requests::Request;
        let req = Request::new(
            self.next_seq(),
            "initialize".to_string(),
            serde_json::json!({
                "clientID": "ruster",
                "clientName": "Ruster Editor",
                "adapterID": "lldb-vscode",
                "pathFormat": "path",
                "linesStartAt1": true,
                "columnsStartAt1": true,
                "supportsVariableType": true,
                "supportsVariablePaging": false,
                "supportsRunInTerminalRequest": false,
                "locale": "en"
            }),
        );
        self.client.send_request(req).map_err(Into::into)
    }

    pub fn send_launch(&mut self, config_json: serde_json::Value) -> Result<()> {
        use dap::requests::Request;
        let req = Request::new(
            self.next_seq(),
            "launch".to_string(),
            config_json,
        );
        self.client.send_request(req).map_err(Into::into)
    }

    pub fn send_attach(&mut self, config_json: serde_json::Value) -> Result<()> {
        use dap::requests::Request;
        let req = Request::new(
            self.next_seq(),
            "attach".to_string(),
            config_json,
        );
        self.client.send_request(req).map_err(Into::into)
    }

    pub fn set_breakpoints(&mut self, path: PathBuf, lines: &[usize]) -> Result<()> {
        use dap::requests::Request;
        let src = Source {
            name: Some(path.file_name().unwrap_or_default().to_string_lossy().to_string()),
            path: Some(path.to_string_lossy().to_string()),
            ..Default::default()
        };
        let bps: Vec<SourceBreakpoint> = lines.iter().map(|&line| {
            // clear old breakpoints at this path
            self.breakpoints.retain(|(p, _), _| p != &path);
            SourceBreakpoint {
                line: line as u64,
                column: None,
                condition: None,
                hit_condition: None,
                log_message: None,
            }
        }).collect();
        let args = serde_json::json!({
            "source": src,
            "breakpoints": bps,
            "sourceModified": false,
        });
        let req = Request::new(
            self.next_seq(),
            "setBreakpoints".to_string(),
            args,
        );
        self.client.send_request(req).map_err(Into::into)
    }

    pub fn continue_exec(&mut self) -> Result<()> {
        let tid = self.stopped_thread.unwrap_or(0);
        use dap::requests::Request;
        let req = Request::new(
            self.next_seq(),
            "continue".to_string(),
            serde_json::json!({ "threadId": tid }),
        );
        self.client.send_request(req)?;
        self.state = SessionState::Running;
        self.stopped_thread = None;
        self.stack_frames.clear();
        self.scopes.clear();
        self.variable_cache.clear();
        Ok(())
    }

    pub fn pause(&mut self) -> Result<()> {
        let tid = self.threads.keys().next().copied().unwrap_or(0);
        use dap::requests::Request;
        let req = Request::new(
            self.next_seq(),
            "pause".to_string(),
            serde_json::json!({ "threadId": tid }),
        );
        self.client.send_request(req).map_err(Into::into)
    }

    pub fn step_over(&mut self) -> Result<()> {
        let tid = self.stopped_thread.unwrap_or(0);
        use dap::requests::Request;
        let req = Request::new(
            self.next_seq(),
            "next".to_string(),
            serde_json::json!({ "threadId": tid }),
        );
        self.client.send_request(req)?;
        self.state = SessionState::Running;
        self.stack_frames.clear();
        self.scopes.clear();
        self.variable_cache.clear();
        Ok(())
    }

    pub fn step_into(&mut self) -> Result<()> {
        let tid = self.stopped_thread.unwrap_or(0);
        use dap::requests::Request;
        let req = Request::new(
            self.next_seq(),
            "stepIn".to_string(),
            serde_json::json!({ "threadId": tid }),
        );
        self.client.send_request(req)?;
        self.state = SessionState::Running;
        self.stack_frames.clear();
        self.scopes.clear();
        self.variable_cache.clear();
        Ok(())
    }

    pub fn step_out(&mut self) -> Result<()> {
        let tid = self.stopped_thread.unwrap_or(0);
        use dap::requests::Request;
        let req = Request::new(
            self.next_seq(),
            "stepOut".to_string(),
            serde_json::json!({ "threadId": tid }),
        );
        self.client.send_request(req)?;
        self.state = SessionState::Running;
        self.stack_frames.clear();
        self.scopes.clear();
        self.variable_cache.clear();
        Ok(())
    }

    pub fn get_stack(&mut self, thread_id: u64) -> Result<Vec<StackFrame>> {
        use dap::requests::Request;
        let req = Request::new(
            self.next_seq(),
            "stackTrace".to_string(),
            serde_json::json!({ "threadId": thread_id, "startFrame": 0, "levels": 50 }),
        );
        self.client.send_request(req).map_err(Into::into)
    }

    pub fn get_scopes(&mut self, frame_id: u64) -> Result<Vec<Scope>> {
        use dap::requests::Request;
        let req = Request::new(
            self.next_seq(),
            "scopes".to_string(),
            serde_json::json!({ "frameId": frame_id }),
        );
        self.client.send_request(req).map_err(Into::into)
    }

    pub fn get_variables(&mut self, var_ref: u64) -> Result<Vec<Variable>> {
        use dap::requests::Request;
        let req = Request::new(
            self.next_seq(),
            "variables".to_string(),
            serde_json::json!({ "variablesReference": var_ref }),
        );
        self.client.send_request(req).map_err(Into::into)
    }

    pub fn evaluate(&mut self, expr: &str, frame_id: u64) -> Result<String> {
        use dap::requests::Request;
        let req = Request::new(
            self.next_seq(),
            "evaluate".to_string(),
            serde_json::json!({
                "expression": expr,
                "frameId": frame_id,
                "context": "hover",
            }),
        );
        self.client.send_request(req).map_err(Into::into)
    }

    pub fn disconnect(&mut self) -> Result<()> {
        use dap::requests::Request;
        let req = Request::new(
            self.next_seq(),
            "disconnect".to_string(),
            serde_json::json!({ "terminateDebuggee": true }),
        );
        self.client.send_request(req).map_err(Into::into)
    }

    pub fn poll_events(&mut self) -> Vec<DapEvent> {
        let mut events = Vec::new();
        while let Some(msg) = self.client.poll() {
            match msg {
                ServerMessage::Response(rsp) => {
                    self.handle_response(rsp);
                }
                ServerMessage::Event(evt) => {
                    if let Some(de) = self.handle_event(evt) {
                        events.push(de);
                    }
                }
                ServerMessage::Request(req) => {
                    // Handle server requests (like runInTerminal)
                    let rsp = dap::responses::Response::new(
                        req.id,
                        serde_json::Value::Null,
                    );
                    self.client.send_response(rsp).ok();
                }
            }
        }
        events
    }

    fn handle_response(&mut self, rsp: dap::responses::Response) {
        // Store response data for pending requests
        // For simplicity, responses that carry data (stackTrace, scopes, variables, evaluate)
        // are stored in their respective fields.
        // Since the transport is synchronous, the caller will have already sent the request
        // and can poll for the response via the client.
        // This is a simplified pattern — in practice responses match to pending request IDs.
        // For now: response data is handled at the call site by blocking on poll().
    }

    fn handle_event(&mut self, evt: dap::events::Event) -> Option<DapEvent> {
        let evt_type = evt.event_type;
        let body = evt.body.unwrap_or(serde_json::Value::Null);
        match evt_type.as_str() {
            "stopped" => {
                let reason = body["reason"].as_str().unwrap_or("unknown").to_string();
                let tid = body["threadId"].as_u64().unwrap_or(0);
                self.state = SessionState::Paused;
                self.stopped_thread = Some(tid);
                if tid > 0 && !self.threads.contains_key(&tid) {
                    self.threads.insert(tid, Thread { id: tid, name: format!("Thread {tid}") });
                }
                Some(DapEvent::Stopped { reason, thread_id: tid })
            }
            "continued" => {
                let tid = body["threadId"].as_u64().unwrap_or(0);
                self.state = SessionState::Running;
                Some(DapEvent::Continued { thread_id: tid })
            }
            "exited" => {
                let code = body["exitCode"].as_i64().unwrap_or(0);
                Some(DapEvent::Exited { exit_code: code })
            }
            "terminated" => {
                self.state = SessionState::Terminated;
                Some(DapEvent::Terminated)
            }
            "output" => {
                let cat = body["category"].as_str().unwrap_or("console").to_string();
                let out = body["output"].as_str().unwrap_or("").to_string();
                Some(DapEvent::Output { category: cat, output: out })
            }
            "thread" => {
                let reason = body["reason"].as_str().unwrap_or("started").to_string();
                let tid = body["threadId"].as_u64().unwrap_or(0);
                if reason == "started" {
                    self.threads.insert(tid, Thread { id: tid, name: format!("Thread {tid}") });
                }
                Some(DapEvent::Thread { reason, thread_id: tid })
            }
            "breakpoint" => {
                let bp_id = body["breakpoint"]["id"].as_u64().unwrap_or(0);
                let verified = body["breakpoint"]["verified"].as_bool().unwrap_or(false);
                // Update cached breakpoint
                for (_, bp) in self.breakpoints.iter_mut() {
                    if bp.id == Some(bp_id) {
                        bp.verified = verified;
                        break;
                    }
                }
                Some(DapEvent::BreakpointValidated { id: bp_id, verified })
            }
            "module" => {
                let mod_id = body["moduleId"].as_u64().unwrap_or(0);
                let name = body["module"]["name"].as_str().unwrap_or("?").to_string();
                Some(DapEvent::Module { id: mod_id, name })
            }
            "process" => {
                let name = body["name"].as_str().unwrap_or("?").to_string();
                let pid = body["systemProcessId"].as_u64().unwrap_or(0);
                Some(DapEvent::Process { name, pid })
            }
            _ => None,
        }
    }
}
```

- **Step 2: Build to verify**

```
cargo build -p ruster-dap 2>&1 | tail -10
```

- **Step 3: Commit**

```
git add crates/ruster-dap/src/session.rs crates/ruster-dap/src/lib.rs
git commit -m "feat(dap): implement DebugSession state machine"
```

---

### Task 5: Implement config.rs — debug adapter detection

**Files:**
- Create: `crates/ruster-dap/src/config.rs`

**Interfaces:**
- Produces: `AdapterConfig`, `detect_config(language: &str, root: &Path) -> Option<AdapterConfig>`

- **Step 1: Create config module**

```rust
use std::path::Path;

#[derive(Debug, Clone)]
pub struct AdapterConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub launch_config: serde_json::Value,
}

/// Detect the debug adapter for a given language at the project root.
/// Returns the adapter config and a default launch configuration.
pub fn detect_config(language: &str, root: &Path, program: Option<&str>) -> Option<AdapterConfig> {
    let lang = language.to_lowercase();
    if lang.contains("rust") || lang.contains("rs") {
        // Use lldb-vscode (comes with rustup's LLVM)
        let program = program.unwrap_or("target/debug/<binary>");
        Some(AdapterConfig {
            name: "lldb-vscode".to_string(),
            command: "lldb-vscode".to_string(),
            args: vec![],
            launch_config: serde_json::json!({
                "type": "lldb",
                "request": "launch",
                "program": program,
                "args": [],
                "cwd": root.to_string_lossy().to_string(),
                "stopOnEntry": false,
            }),
        })
    } else if lang.contains("python") || lang.contains("py") {
        Some(AdapterConfig {
            name: "debugpy".to_string(),
            command: "python".to_string(),
            args: vec!["-m".to_string(), "debugpy.adapter".to_string()],
            launch_config: serde_json::json!({
                "type": "python",
                "request": "launch",
                "program": program.unwrap_or("${file}"),
                "console": "integratedTerminal",
            }),
        })
    } else if lang.contains("go") || lang.contains("golang") {
        Some(AdapterConfig {
            name: "dlv-dap".to_string(),
            command: "dlv".to_string(),
            args: vec!["dap".to_string()],
            launch_config: serde_json::json!({
                "type": "go",
                "request": "launch",
                "mode": "auto",
                "program": program.unwrap_or("."),
            }),
        })
    } else {
        None
    }
}
```

- **Step 2: Export from lib.rs**

Add to `crates/ruster-dap/src/lib.rs`:

```rust
pub mod transport;
pub mod client;
pub mod session;
pub mod config;
```

- **Step 3: Build to verify**

```
cargo build -p ruster-dap 2>&1 | tail -5
```

- **Step 4: Commit**

```
git add crates/ruster-dap/src/config.rs crates/ruster-dap/src/lib.rs
git commit -m "feat(dap): add adapter config detection"
```

---

### Task 6: Wire ruster-dap into ruster-tui (debug session lifecycle)

**Files:**
- Modify: `crates/ruster-tui/Cargo.toml` (add ruster-dap dep)
- Modify: `crates/ruster-tui/src/app.rs` (App fields, init, shutdown, event polling)

**Interfaces:**
- Consumes: `DebugSession`, `DapEvent`, `AdapterConfig`, `detect_config`
- Modifies: `App` struct with `debug_session: Option<DebugSession>`

- **Step 1: Add ruster-dap dependency**

Add to `crates/ruster-tui/Cargo.toml`:

```toml
ruster-dap = { path = "../ruster-dap" }
```

- **Step 2: Add debug_session field to App**

```rust
pub struct App {
    // ... existing fields
    pub debug_session: Option<ruster_dap::session::DebugSession>,
}
```

Initialize as `debug_session: None` in the constructor.

- **Step 3: Add debug start/stop methods**

```rust
use ruster_dap::session::DebugSession;
use ruster_dap::config::AdapterConfig;

pub fn start_debugging(&mut self) {
    if self.debug_session.is_some() {
        return; // already debugging
    }
    // Detect language from active buffer
    let lang = self.active_buffer_language();
    let root = self.project_root.as_deref()
        .or_else(|| std::env::current_dir().ok());
    let (lang, root) = match (lang, root) {
        (Some(l), Some(r)) => (l, r),
        _ => {
            self.message = Some("Cannot determine project root".to_string());
            return;
        }
    };
    // Find the binary from cargo (first check Cargo.toml)
    let program = self.detect_debug_program(&root);
    let config = ruster_dap::config::detect_config(&lang, &root, program.as_deref());
    let config = match config {
        Some(c) => c,
        None => {
            self.message = Some(format!("No debug adapter for {lang}"));
            return;
        }
    };
    match DebugSession::start(&config, &root) {
        Ok(mut session) => {
            // Initialize handshake
            session.send_initialize().ok();
            // Launch
            let launch = config.launch_config.clone();
            session.send_launch(launch).ok();
            self.debug_session = Some(session);
            self.message = Some("Debugging started".to_string());
        }
        Err(e) => {
            self.message = Some(format!("Debug start failed: {e}"));
        }
    }
}

pub fn stop_debugging(&mut self) {
    if let Some(mut session) = self.debug_session.take() {
        session.disconnect().ok();
        self.message = Some("Debugging stopped".to_string());
    }
}

/// Detect debug program from Cargo.toml or use target/debug/<name>.
fn detect_debug_program(&self, root: &Path) -> Option<String> {
    let cargo_toml = root.join("Cargo.toml");
    if cargo_toml.exists() {
        // Parse the first binary name from Cargo.toml
        let content = std::fs::read_to_string(&cargo_toml).ok()?;
        if let Some(name) = content.lines()
            .find(|l| l.trim().starts_with("name ="))
            .and_then(|l| l.split('=').nth(1))
            .map(|s| s.trim().trim_matches('"').to_string())
        {
            return Some(format!("target/debug/{name}"));
        }
    }
    None
}
```

- **Step 4: Poll debug events each frame**

In the main loop's per-frame update (or in `App::tick()` / `App::update()`), add:

```rust
// Poll debug session events
if let Some(ref mut session) = self.debug_session {
    for event in session.poll_events() {
        match event {
            DapEvent::Stopped { reason, thread_id } => {
                // Fetch stack
                session.get_stack(thread_id).ok();
                // The response will be handled by the response handler
                // For now: set state to Paused, UI will show stack
            }
            DapEvent::Terminated => {
                self.stop_debugging();
            }
            DapEvent::Output { category, output } => {
                // Log to messages
                self.message = Some(format!("[debug/{category}] {output}"));
            }
            _ => {}
        }
    }
}
```

- **Step 5: Build to verify**

```
cargo build -p ruster-tui 2>&1 | tail -10
```

- **Step 6: Commit**

```
git add crates/ruster-tui/Cargo.toml crates/ruster-tui/src/app.rs
git commit -m "feat(dap): wire debug session lifecycle in App"
```

---

### Task 7: Add debug keybindings (F5, Shift+F5, F10, F11, Shift+F11, Ctrl+F8, K)

**Files:**
- Modify: `crates/ruster-tui/src/app.rs` (handle_key dispatch)

**Interfaces:**
- Consumes: `start_debugging()`, `stop_debugging()`, session step/continue methods

- **Step 1: Add debug key dispatch to handle_key**

In `handle_key()`, at the start of the dispatch chain (before other handlers), add:

```rust
// Debug keys (available in any mode when session exists).
match ck.code {
    KeyCode::F(5) if ck.modifiers.is_empty() => {
        if self.debug_session.is_some() {
            // Continue
            if let Some(ref mut session) = self.debug_session {
                if session.state == ruster_dap::session::SessionState::Paused {
                    session.continue_exec().ok();
                }
            }
        } else {
            self.start_debugging();
        }
        return true;
    }
    KeyCode::F(5) if ck.modifiers.contains(KeyModifiers::SHIFT) => {
        self.stop_debugging();
        return true;
    }
    KeyCode::F(10) => {
        if let Some(ref mut session) = self.debug_session {
            session.step_over().ok();
        }
        return true;
    }
    KeyCode::F(11) if ck.modifiers.is_empty() => {
        if let Some(ref mut session) = self.debug_session {
            session.step_into().ok();
        }
        return true;
    }
    KeyCode::F(11) if ck.modifiers.contains(KeyModifiers::SHIFT) => {
        if let Some(ref mut session) = self.debug_session {
            session.step_out().ok();
        }
        return true;
    }
    KeyCode::Char('8') if ck.modifiers.contains(KeyModifiers::CONTROL) => {
        // Ctrl+F8: toggle breakpoint on current line
        self.toggle_breakpoint();
        return true;
    }
    _ => {}
}
```

- **Step 2: Add toggle_breakpoint method**

```rust
fn toggle_breakpoint(&mut self) {
    let Some(ref mut session) = self.debug_session else { return };
    let active = self.active_window_id();
    let Some(win) = self.windows.window(active) else { return };
    let Some(buf) = self.buffers.get(win.buffer) else { return };
    let path = match &buf.path {
        Some(p) => p.clone(),
        None => return,
    };
    let line = buf.offset_to_line(win.cursors.primary().head).unwrap_or(0) + 1;
    let key = (path.clone(), line);
    if session.breakpoints.contains_key(&key) {
        // Remove
        session.breakpoints.remove(&key);
        // Re-send remaining breakpoints for this file
        let lines: Vec<usize> = session.breakpoints.keys()
            .filter(|(p, _)| p == &path)
            .map(|(_, l)| *l)
            .collect();
        session.set_breakpoints(path, &lines).ok();
    } else {
        // Add
        session.breakpoints.insert(key, Breakpoint::default());
        let lines: Vec<usize> = session.breakpoints.keys()
            .filter(|(p, _)| p == &path)
            .map(|(_, l)| *l)
            .collect();
        session.set_breakpoints(path, &lines).ok();
    }
}
```

- **Step 3: Add K (hover evaluate) dispatch**

In the `handle_key` section where `K` is handled for LSP hover, add a check:

```rust
KeyCode::Char('K') => {
    if let Some(ref mut session) = self.debug_session {
        // Debug hover overrides LSP hover when session is paused
        if session.state == ruster_dap::session::SessionState::Paused {
            self.debug_hover();
            return true;
        }
    }
    // Fall through to existing LSP hover...
}
```

Add the `debug_hover()` method:

```rust
fn debug_hover(&mut self) {
    let Some(ref mut session) = self.debug_session else { return };
    let active = self.active_window_id();
    let Some(win) = self.windows.window(active) else { return };
    let Some(buf) = self.buffers.get(win.buffer) else { return };
    let pos = win.cursors.primary().head;
    let word = self.word_at(pos);
    let word = match word {
        Some(w) => w,
        None => return,
    };
    // Use the top stack frame
    let frame_id = session.stack_frames.first()
        .and_then(|f| f.id).unwrap_or(0);
    match session.evaluate(&word, frame_id) {
        Ok(result) => {
            self.message = Some(format!("{} = {}", word, result));
        }
        Err(_) => {}
    }
}
```

- **Step 4: Build to verify**

```
cargo build -p ruster-tui 2>&1 | tail -10
```

- **Step 5: Commit**

```
git add crates/ruster-tui/src/app.rs
git commit -m "feat(dap): wire F5/S-F5/F10/F11/S-F11/Ctrl+F8/K for debug"
```

---

### Task 8: Render breakpoint gutter signs

**Files:**
- Modify: `crates/ruster-tui/src/app.rs` (render method, gutter/signs section)
- Modify: `crates/ruster-render/src/lib.rs` (SignsView maybe, or reuse existing)

**Interfaces:**
- Consumes: `debug_session.breakpoints` keys
- Modifies: `WindowView.signs` to include breakpoint markers

- **Step 1: Add breakpoint signs to SignsView or gutter**

In `App::render()`, when building `WindowView` for each window, check the current buffer path against `debug_session.breakpoints`:

```rust
// In render(), build signs:
let mut bp_lines: Vec<usize> = Vec::new();
if let Some(ref session) = self.debug_session {
    if let Some(ref path) = buf.path {
        bp_lines = session.breakpoints.keys()
            .filter(|(p, _)| p == path)
            .map(|(_, line)| *line)
            .collect();
    }
}
```

Then pass breakpoint info to the renderer. The simplest approach: add a `breakpoints: Vec<usize>` field to `GutterView` or `SignsView`, and the TUI renderer draws a red `●` on those lines.

Add to `crates/ruster-render/src/lib.rs`:

```rust
pub struct GutterView {
    pub lines: Vec<GutterLine>,
    pub width: u16,
    pub breakpoints: Vec<usize>, // 1-based line numbers with breakpoints
}
```

Initialize as `breakpoints: Vec::new()` where `GutterView` is constructed.

- **Step 2: Render the breakpoint dots in the TUI renderer**

In `crates/ruster-tui/src/renderer.rs`, in the gutter rendering section, for each line number, check if the line has a breakpoint. If so, draw a red `●` instead of the line number. Or draw it to the left of the line number in a separate column.

Update the gutter rendering to check `gutter.breakpoints.contains(&line_no)` and render `●` with red styling.

- **Step 3: Build to verify**

```
cargo build -p ruster-tui -p ruster-render 2>&1 | tail -5
```

- **Step 4: Commit**

```
git add crates/ruster-render/src/lib.rs crates/ruster-tui/src/renderer.rs crates/ruster-tui/src/app.rs
git commit -m "feat(dap): render breakpoint dots in gutter"
```

---

### Task 9: Render debug toolbar and stack/variables overlay

**Files:**
- Create: `crates/ruster-tui/src/debug_ui.rs`
- Modify: `crates/ruster-tui/src/app.rs` (render method, import)
- Modify: `crates/ruster-tui/src/lib.rs` (export new module)

**Interfaces:**
- Consumes: `App.debug_session`, `SessionState`, stack frames, scopes, variables
- Produces: Rendered debug toolbar + stack + variables overlay

- **Step 1: Create debug_ui.rs**

```rust
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};

use crate::app::App;
use ruster_dap::session::{DebugSession, SessionState};

const TOOLBAR_HEIGHT: u16 = 1;

pub fn render_debug_ui(f: &mut Frame, app: &App, area: Rect) {
    let session = match &app.debug_session {
        Some(s) => s,
        None => return,
    };

    // Toolbar at the bottom of the screen
    let toolbar_rect = Rect::new(0, area.height.saturating_sub(TOOLBAR_HEIGHT + 1), area.width, TOOLBAR_HEIGHT);
    render_toolbar(f, session, toolbar_rect);

    // Right panel: stack frames
    let panel_width = 40.min(area.width / 3);
    let panel_rect = Rect::new(area.width.saturating_sub(panel_width), 0, panel_width, area.height.saturating_sub(TOOLBAR_HEIGHT + 1));
    render_stack_panel(f, session, panel_rect);
}

fn render_toolbar(f: &mut Frame, session: &DebugSession, area: Rect) {
    let (continue_btn, step_over_btn, step_into_btn, step_out_btn, stop_btn) = match session.state {
        SessionState::Paused => (
            "[ ▶ Continue ]", "[ ⤵ Over ]", "[ ↘ Into ]", "[ ↖ Out ]", "[ ⏹ Stop ]"
        ),
        SessionState::Running => (
            "[ ⏸ Pause ]", "", "", "", "[ ⏹ Stop ]"
        ),
        _ => ("", "", "", "", ""),
    };

    let text = Line::from(vec![
        Span::styled(continue_btn, Style::default().fg(Color::Green)),
        Span::raw(" "),
        Span::styled(step_over_btn, Style::default().fg(Color::Cyan)),
        Span::raw(" "),
        Span::styled(step_into_btn, Style::default().fg(Color::Cyan)),
        Span::raw(" "),
        Span::styled(step_out_btn, Style::default().fg(Color::Cyan)),
        Span::raw(" "),
        Span::styled(stop_btn, Style::default().fg(Color::Red)),
    ]);

    let toolbar = Paragraph::new(text)
        .block(Block::default().borders(Borders::TOP).style(Style::default().bg(Color::DarkGray)));
    f.render_widget(toolbar, area);
}

fn render_stack_panel(f: &mut Frame, session: &DebugSession, area: Rect) {
    let items: Vec<ListItem> = session.stack_frames.iter().map(|sf| {
        let location = sf.name.clone();
        let src = sf.source.as_ref()
            .and_then(|s| s.path.as_ref())
            .map(|p| format!("{}:{}", p, sf.line.unwrap_or(0)))
            .unwrap_or_default();
        ListItem::new(format!("{} at {}", location, src))
    }).collect();

    let list = List::new(items)
        .block(Block::default().title(" Call Stack ").borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));
    f.render_widget(list, area);
}
```

- **Step 2: Wire into App::render()**

In `App::render()`, after all windows are rendered, if `debug_session.is_some()`:

```rust
// Debug overlay
if self.debug_session.is_some() {
    crate::debug_ui::render_debug_ui(f, self, area);
}
```

- **Step 3: Register module**

Add to `crates/ruster-tui/src/lib.rs`:

```rust
pub mod debug_ui;
```

- **Step 4: Build to verify**

```
cargo build -p ruster-tui 2>&1 | tail -5
```

- **Step 5: Run tests**

```
cargo test -p ruster-tui 2>&1 | tail -5
```

- **Step 6: Commit**

```
git add crates/ruster-tui/src/debug_ui.rs crates/ruster-tui/src/lib.rs crates/ruster-tui/src/app.rs
git commit -m "feat(dap): render debug toolbar and stack overlay"
```
