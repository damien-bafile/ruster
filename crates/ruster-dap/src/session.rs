use std::collections::HashMap;
use std::path::{Path, PathBuf};

use dap::requests::*;
use dap::types::*;

use crate::client::DapClient;
use crate::config::AdapterConfig;
use crate::transport::ServerMessage;

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
    /// Variables by the `variablesReference` they were fetched for. A scope's
    /// reference names a whole list, not one variable.
    pub variable_cache: HashMap<u64, Vec<Variable>>,
    pub stopped_thread: Option<u64>,
    pub variables: Vec<(String, Vec<(String, String)>)>,
    next_seq: i64,
    /// `configurationDone` is sent once per session, whichever path gets there
    /// first — see [`DebugSession::send_configuration_done`].
    configuration_done: bool,
    /// Request seq → the `variablesReference` it asked about. A `variables`
    /// response carries no hint of which reference it answers, so the only way
    /// to file it correctly is to remember what we asked.
    pending_variables: HashMap<i64, u64>,
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
            variables: Vec::new(),
            next_seq: 1,
            configuration_done: false,
            pending_variables: HashMap::new(),
        })
    }

    pub fn stopped(&self) -> bool {
        self.state == SessionState::Paused
    }

    pub fn set_breakpoints_all(&mut self, files: Vec<(PathBuf, Vec<u16>)>) -> Result<()> {
        for (path, lines) in files {
            let lines_usize: Vec<usize> = lines.into_iter().map(|l| l as usize).collect();
            let src = Source {
                name: Some(path.file_name().unwrap_or_default().to_string_lossy().to_string()),
                path: Some(path.to_string_lossy().to_string()),
                ..Default::default()
            };
            let bps: Vec<SourceBreakpoint> = lines_usize.iter().map(|&line| {
                SourceBreakpoint {
                    line: to_dap_line(line),
                    column: None,
                    condition: None,
                    hit_condition: None,
                    log_message: None,
                }
            }).collect();
            let args = SetBreakpointsArguments {
                source: src,
                breakpoints: Some(bps),
                source_modified: Some(false),
                ..Default::default()
            };
            let req = Request { seq: self.next_seq(), command: Command::SetBreakpoints(args) };
            self.client.send_request(req)?;
        }
        Ok(())
    }

    fn next_seq(&mut self) -> i64 {
        let s = self.next_seq;
        self.next_seq += 1;
        s
    }

    pub fn send_initialize(&mut self) -> Result<()> {
        let args = InitializeArguments {
            client_id: Some("ruster".into()),
            client_name: Some("Ruster Editor".into()),
            adapter_id: "lldb-vscode".into(),
            lines_start_at1: Some(true),
            columns_start_at1: Some(true),
            supports_variable_type: Some(true),
            supports_variable_paging: Some(false),
            supports_run_in_terminal_request: Some(false),
            ..Default::default()
        };
        let req = Request { seq: self.next_seq(), command: Command::Initialize(args) };
        self.client.send_request(req)?;
        Ok(())
    }

    pub fn send_launch(&mut self, config_json: serde_json::Value) -> Result<()> {
        let args = LaunchRequestArguments {
            no_debug: Some(false),
            additional_data: Some(config_json),
            ..Default::default()
        };
        let req = Request { seq: self.next_seq(), command: Command::Launch(args) };
        self.client.send_request(req)?;
        self.state = SessionState::Running;
        Ok(())
    }

    /// Tell the adapter configuration is finished and it may run the program.
    ///
    /// Not optional: an adapter that reports `supportsConfigurationDoneRequest`
    /// holds the reply to `launch` until this arrives, so without it the target
    /// is never started, never hits a breakpoint, and the UI sits on RUNNING
    /// with no frames for as long as you care to wait.
    ///
    /// Sent eagerly, once the breakpoints are in, rather than in reply to the
    /// `initialized` event — lldb-dap does not emit that event until *after*
    /// this request, so waiting for it deadlocks the handshake. Adapters that
    /// do emit it early are covered by the call in `handle_event`, and this is
    /// idempotent so whichever arrives first is the one that counts.
    pub fn send_configuration_done(&mut self) -> Result<()> {
        if self.configuration_done {
            return Ok(());
        }
        let req = Request { seq: self.next_seq(), command: Command::ConfigurationDone };
        self.client.send_request(req)?;
        self.configuration_done = true;
        Ok(())
    }

    pub fn set_breakpoints(&mut self, path: PathBuf, lines: &[usize]) -> Result<()> {
        let src = Source {
            name: Some(path.file_name().unwrap_or_default().to_string_lossy().to_string()),
            path: Some(path.to_string_lossy().to_string()),
            ..Default::default()
        };
        let bps: Vec<SourceBreakpoint> = lines.iter().map(|&line| {
            self.breakpoints.retain(|(p, _), _| p != &path);
            SourceBreakpoint {
                line: to_dap_line(line),
                column: None,
                condition: None,
                hit_condition: None,
                log_message: None,
            }
        }).collect();
        let args = SetBreakpointsArguments {
            source: src,
            breakpoints: Some(bps),
            source_modified: Some(false),
            ..Default::default()
        };
        let req = Request { seq: self.next_seq(), command: Command::SetBreakpoints(args) };
        self.client.send_request(req)?;
        Ok(())
    }

    pub fn continue_exec(&mut self) -> Result<()> {
        let tid = self.stopped_thread.unwrap_or(0) as i64;
        let args = ContinueArguments { thread_id: tid, single_thread: None };
        let req = Request { seq: self.next_seq(), command: Command::Continue(args) };
        self.client.send_request(req)?;
        self.state = SessionState::Running;
        self.stopped_thread = None;
        self.stack_frames.clear();
        self.scopes.clear();
        self.variable_cache.clear();
        Ok(())
    }

    pub fn pause(&mut self) -> Result<()> {
        let tid = self.threads.keys().next().copied().unwrap_or(0) as i64;
        let args = PauseArguments { thread_id: tid };
        let req = Request { seq: self.next_seq(), command: Command::Pause(args) };
        self.client.send_request(req)?;
        Ok(())
    }

    pub fn step_over(&mut self) -> Result<()> {
        let tid = self.stopped_thread.unwrap_or(0) as i64;
        let args = NextArguments { thread_id: tid, single_thread: None, granularity: None };
        let req = Request { seq: self.next_seq(), command: Command::Next(args) };
        self.client.send_request(req)?;
        self.state = SessionState::Running;
        self.stack_frames.clear();
        self.scopes.clear();
        self.variable_cache.clear();
        Ok(())
    }

    pub fn step_into(&mut self) -> Result<()> {
        let tid = self.stopped_thread.unwrap_or(0) as i64;
        let args = StepInArguments { thread_id: tid, single_thread: None, target_id: None, granularity: None };
        let req = Request { seq: self.next_seq(), command: Command::StepIn(args) };
        self.client.send_request(req)?;
        self.state = SessionState::Running;
        self.stack_frames.clear();
        self.scopes.clear();
        self.variable_cache.clear();
        Ok(())
    }

    pub fn step_out(&mut self) -> Result<()> {
        let tid = self.stopped_thread.unwrap_or(0) as i64;
        let args = StepOutArguments { thread_id: tid, single_thread: None, granularity: None };
        let req = Request { seq: self.next_seq(), command: Command::StepOut(args) };
        self.client.send_request(req)?;
        self.state = SessionState::Running;
        self.stack_frames.clear();
        self.scopes.clear();
        self.variable_cache.clear();
        Ok(())
    }

    pub fn get_stack(&mut self, thread_id: u64) -> Result<()> {
        let args = StackTraceArguments {
            thread_id: thread_id as i64,
            start_frame: Some(0),
            levels: Some(50),
            format: None,
        };
        let req = Request { seq: self.next_seq(), command: Command::StackTrace(args) };
        self.client.send_request(req)?;
        Ok(())
    }

    pub fn get_scopes(&mut self, frame_id: u64) -> Result<()> {
        let args = ScopesArguments { frame_id: frame_id as i64 };
        let req = Request { seq: self.next_seq(), command: Command::Scopes(args) };
        self.client.send_request(req)?;
        Ok(())
    }

    pub fn get_variables(&mut self, var_ref: u64) -> Result<()> {
        let args = VariablesArguments {
            variables_reference: var_ref as i64,
            filter: None,
            start: None,
            count: None,
            format: None,
        };
        let seq = self.next_seq();
        self.pending_variables.insert(seq, var_ref);
        let req = Request { seq, command: Command::Variables(args) };
        self.client.send_request(req)?;
        Ok(())
    }

    pub fn evaluate(&mut self, expr: &str, frame_id: u64) -> Result<()> {
        let args = EvaluateArguments {
            expression: expr.into(),
            frame_id: Some(frame_id as i64),
            context: Some(EvaluateArgumentsContext::Hover),
            format: None,
        };
        let req = Request { seq: self.next_seq(), command: Command::Evaluate(args) };
        self.client.send_request(req)?;
        Ok(())
    }

    pub fn disconnect(&mut self) -> Result<()> {
        let args = DisconnectArguments {
            terminate_debuggee: Some(true),
            ..Default::default()
        };
        let req = Request { seq: self.next_seq(), command: Command::Disconnect(args) };
        self.client.send_request(req)?;
        self.state = SessionState::Terminated;
        Ok(())
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
                    let rsp = dap::responses::Response {
                        request_seq: req.seq,
                        success: true,
                        message: None,
                        body: None,
                        error: None,
                    };
                    self.client.send_response(rsp).ok();
                }
            }
        }
        events
    }

    /// File a response into the state the UI reads.
    ///
    /// This used to be empty, so every reply the adapter sent was dropped on
    /// the floor: the session stopped at a breakpoint, asked for the stack,
    /// got it, and still rendered "(no frames)" because nothing ever stored
    /// the answer.
    fn handle_response(&mut self, rsp: dap::responses::Response) {
        use dap::responses::ResponseBody;
        let Some(body) = rsp.body else { return };
        match body {
            ResponseBody::StackTrace(b) => self.stack_frames = b.stack_frames,
            ResponseBody::Scopes(b) => self.scopes = b.scopes,
            ResponseBody::Variables(b) => {
                if let Some(reference) = self.pending_variables.remove(&rsp.request_seq) {
                    self.variable_cache.insert(reference, b.variables);
                }
            }
            ResponseBody::Threads(b) => {
                self.threads = b.threads.into_iter().map(|t| (t.id as u64, t)).collect();
            }
            _ => {}
        }
    }

    fn handle_event(&mut self, evt: dap::events::Event) -> Option<DapEvent> {
        use dap::events::Event;
        match evt {
            Event::Stopped(body) => {
                let reason = format!("{:?}", body.reason);
                let tid = body.thread_id.unwrap_or(0) as u64;
                self.state = SessionState::Paused;
                self.stopped_thread = Some(tid);
                if tid > 0 && !self.threads.contains_key(&tid) {
                    self.threads.insert(tid, Thread { id: tid as i64, name: format!("Thread {tid}") });
                }
                Some(DapEvent::Stopped { reason, thread_id: tid })
            }
            Event::Continued(body) => {
                let tid = body.thread_id as u64;
                self.state = SessionState::Running;
                Some(DapEvent::Continued { thread_id: tid })
            }
            Event::Exited(body) => {
                Some(DapEvent::Exited { exit_code: body.exit_code })
            }
            Event::Terminated(_) => {
                self.state = SessionState::Terminated;
                Some(DapEvent::Terminated)
            }
            Event::Output(body) => {
                let cat = format!("{:?}", body.category.unwrap_or(dap::types::OutputEventCategory::Console));
                Some(DapEvent::Output { category: cat, output: body.output })
            }
            Event::Thread(body) => {
                let reason = format!("{:?}", body.reason);
                let tid = body.thread_id as u64;
                self.threads.insert(tid, Thread { id: tid as i64, name: format!("Thread {tid}") });
                Some(DapEvent::Thread { reason, thread_id: tid })
            }
            Event::Breakpoint(body) => {
                let id = body.breakpoint.id.unwrap_or(0) as u64;
                let verified = body.breakpoint.verified;
                Some(DapEvent::BreakpointValidated { id, verified })
            }
            Event::Module(body) => {
                let id = module_id_of(&body.module.id);
                let name = body.module.name.clone();
                Some(DapEvent::Module { id, name })
            }
            Event::Process(body) => {
                let name = body.name;
                let pid = body.system_process_id.unwrap_or(0) as u64;
                Some(DapEvent::Process { name, pid })
            }
            // The adapter has finished initialising and is waiting to be told
            // configuration is complete. This is the handshake step that
            // actually starts the program; it is not surfaced to the UI.
            Event::Initialized => {
                self.send_configuration_done().ok();
                None
            }
            _ => None,
        }
    }
}

/// An editor row (0-based) as a DAP line number.
///
/// `initialize` advertises `linesStartAt1`, so the adapter reads what we send
/// as 1-based. Passing the row straight through set every breakpoint one line
/// above the one the user clicked.
fn to_dap_line(row: usize) -> i64 {
    row as i64 + 1
}

/// A DAP module id as a number.
///
/// The `dap` crate models `ModuleId` as either `Number` — a *unit* variant that
/// carries no payload — or an untagged `String`. So a numeric id only ever
/// reaches us in the string form; there is genuinely nothing to read out of
/// `Number`, and ids that aren't numeric fall back to 0.
fn module_id_of(id: &dap::types::ModuleId) -> u64 {
    match id {
        dap::types::ModuleId::String(s) => s.parse().unwrap_or(0),
        dap::types::ModuleId::Number => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dap::types::ModuleId;

    /// Regression: this previously read `ModuleId::Number | _ => 0`, so the
    /// wildcard swallowed every case and *every* module reported id 0.
    #[test]
    fn module_id_reads_the_string_form() {
        assert_eq!(module_id_of(&ModuleId::String("42".into())), 42);
        assert_eq!(module_id_of(&ModuleId::String("0".into())), 0);
    }

    #[test]
    fn module_id_falls_back_to_zero_when_unreadable() {
        // Non-numeric ids (adapters may send opaque handles) and the payload-less
        // Number variant both have no number to report.
        assert_eq!(module_id_of(&ModuleId::String("libc.so.6".into())), 0);
        assert_eq!(module_id_of(&ModuleId::String(String::new())), 0);
        assert_eq!(module_id_of(&ModuleId::Number), 0);
    }
}
