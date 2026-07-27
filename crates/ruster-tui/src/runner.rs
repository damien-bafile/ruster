//! Background command runner for the build/test/task runners (Phase 5). A
//! command runs on its own thread; its merged stdout+stderr streams back as
//! [`RunnerMsg`] lines over an `mpsc` channel, drained per frame like the LSP
//! and picker streams. No tokio — plain threads + channels.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;

use crate::quickfix::QuickfixItem;

/// A message from a running command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunnerMsg {
    /// One line of output (stdout or stderr), without the trailing newline.
    Line(String),
    /// The command finished with this exit code (`None` if it couldn't run).
    Done(Option<i32>),
}

/// Spawn `cmd` through the platform shell in `cwd`, returning a receiver of its
/// output lines followed by a final [`RunnerMsg::Done`]. Never blocks the caller.
pub fn spawn_shell_command(cmd: &str, cwd: &Path) -> Receiver<RunnerMsg> {
    let (tx, rx) = mpsc::channel();
    let cmd = cmd.to_string();
    let cwd = cwd.to_path_buf();
    thread::spawn(move || {
        let (program, first_arg) = if cfg!(windows) { ("cmd", "/C") } else { ("sh", "-c") };
        let child = Command::new(program)
            .arg(first_arg)
            .arg(&cmd)
            .current_dir(&cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(RunnerMsg::Line(format!("failed to start `{cmd}`: {e}")));
                let _ = tx.send(RunnerMsg::Done(None));
                return;
            }
        };
        // Read stdout and stderr concurrently into the same channel.
        let mut handles = Vec::new();
        let pipes = [child.stdout.take().map(Pipe::Out), child.stderr.take().map(Pipe::Err)];
        for pipe in pipes.into_iter().flatten() {
            let tx = tx.clone();
            handles.push(thread::spawn(move || pipe.forward_lines(&tx)));
        }
        for h in handles {
            let _ = h.join();
        }
        let code = child.wait().ok().and_then(|s| s.code());
        let _ = tx.send(RunnerMsg::Done(code));
    });
    rx
}

/// A child pipe tagged by which stream it is (both forward lines the same way;
/// the tag keeps the reader threads' closures monomorphic).
enum Pipe {
    Out(std::process::ChildStdout),
    Err(std::process::ChildStderr),
}

impl Pipe {
    fn forward_lines(self, tx: &mpsc::Sender<RunnerMsg>) {
        match self {
            Pipe::Out(o) => forward(BufReader::new(o), tx),
            Pipe::Err(e) => forward(BufReader::new(e), tx),
        }
    }
}

fn forward<R: BufRead>(reader: R, tx: &mpsc::Sender<RunnerMsg>) {
    for line in reader.lines().map_while(Result::ok) {
        if tx.send(RunnerMsg::Line(line)).is_err() {
            break; // receiver dropped
        }
    }
}

/// Parse compiler/tool output into quickfix items. Handles the rustc/cargo
/// textual form (a `error[..]:`/`warning:` header followed by a ` --> file:line:col`
/// location) and the generic `file:line:col: message` form (gcc/clang/eslint …).
/// Relative paths are resolved against `root`.
pub fn parse_build_diagnostics(output: &str, root: &Path) -> Vec<QuickfixItem> {
    let mut items = Vec::new();
    // The most recent rustc header, carried down to its `-->` location line.
    let mut pending: Option<(u8, String)> = None;

    for raw in output.lines() {
        let line = raw.trim_end();
        let trimmed = line.trim_start();

        // rustc header: `error[E0433]: msg` / `warning: msg`.
        if let Some((sev, msg)) = rustc_header(trimmed) {
            pending = Some((sev, msg));
            continue;
        }

        // rustc location: `--> path:line:col`.
        if let Some(rest) = trimmed.strip_prefix("-->") {
            if let Some((path, l, c)) = split_path_line_col(rest.trim()) {
                let (sev, msg) = pending.take().unwrap_or((1, String::new()));
                items.push(make_item(root, path, l, c, msg, sev));
            }
            continue;
        }

        // Generic `path:line:col: message`.
        if let Some((path, l, c, msg)) = generic_diagnostic(line) {
            let sev = severity_from_message(&msg);
            items.push(make_item(root, path, l, c, msg, sev));
        }
    }
    items
}

fn make_item(root: &Path, path: &str, line: usize, col: usize, message: String, severity: u8) -> QuickfixItem {
    let p = Path::new(path);
    let path = if p.is_absolute() { p.to_path_buf() } else { root.join(p) };
    QuickfixItem { path, line, col, message, severity }
}

/// `error[E0433]: msg` / `error: msg` → (1, msg); `warning: msg` → (2, msg).
fn rustc_header(s: &str) -> Option<(u8, String)> {
    let (kind, rest) = if let Some(r) = s.strip_prefix("error") {
        (1u8, r)
    } else if let Some(r) = s.strip_prefix("warning") {
        (2u8, r)
    } else {
        return None;
    };
    // Skip an optional `[CODE]`, then require `: `.
    let rest = if let Some(open) = rest.strip_prefix('[') {
        open.split_once(']').map(|(_, after)| after)?
    } else {
        rest
    };
    let msg = rest.strip_prefix(':')?.trim();
    // Guard against matching `error:` inside a `file:line:col:` line.
    if msg.is_empty() {
        return None;
    }
    Some((kind, msg.to_string()))
}

/// Split `path:line:col` (trailing text ignored) into its parts.
fn split_path_line_col(s: &str) -> Option<(&str, usize, usize)> {
    // Take the first whitespace-delimited token, then peel `:line:col` off the
    // right so paths may contain colons (Windows drive letters) elsewhere.
    let token = s.split_whitespace().next()?;
    let (rest, col) = token.rsplit_once(':')?;
    let (path, line) = rest.rsplit_once(':')?;
    let col: usize = col.parse().ok()?;
    let line: usize = line.parse().ok()?;
    if path.is_empty() {
        return None;
    }
    Some((path, line, col))
}

/// `path:line:col: message` (a whole diagnostic line, not a rustc `-->`).
fn generic_diagnostic(line: &str) -> Option<(&str, usize, usize, String)> {
    let (loc, msg) = line.split_once(": ")?;
    let (path, l, c) = split_path_line_col(loc)?;
    // Require a path-like first field to avoid matching prose with numbers.
    if !path.contains('/') && !path.contains('\\') && !path.contains('.') {
        return None;
    }
    Some((path, l, c, msg.trim().to_string()))
}

/// Severity from a generic message prefix: `error…` → 1, `warning…` → 2, else 3.
fn severity_from_message(msg: &str) -> u8 {
    let m = msg.to_ascii_lowercase();
    if m.starts_with("error") {
        1
    } else if m.starts_with("warning") || m.starts_with("warn") {
        2
    } else {
        3
    }
}

/// Whether a test passed, failed, or was ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestOutcome {
    Pass,
    Fail,
    Ignored,
}

/// One test's result, with the failure location when the harness reported a panic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestResult {
    pub name: String,
    pub outcome: TestOutcome,
    /// `(path, line, col)` of the panic for a failed test, resolved against root.
    pub location: Option<(PathBuf, usize, usize)>,
}

/// The parsed outcome of a test run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TestRun {
    pub results: Vec<TestResult>,
    pub passed: usize,
    pub failed: usize,
}

/// Parse `cargo test` / libtest textual output into per-test results. Reads the
/// `test NAME ... ok|FAILED|ignored` lines and attaches failure locations from
/// the `thread 'NAME' panicked at file:line:col` lines (new and old formats).
pub fn parse_test_results(output: &str, root: &Path) -> TestRun {
    use std::collections::HashMap;
    let mut results: Vec<TestResult> = Vec::new();
    let mut locs: HashMap<String, (PathBuf, usize, usize)> = HashMap::new();

    for raw in output.lines() {
        let line = raw.trim();
        // `test NAME ... ok` / `FAILED` / `ignored`.
        if let Some(rest) = line.strip_prefix("test ") {
            if let Some((name, outcome)) = rest.rsplit_once(" ... ") {
                let outcome = match outcome.trim() {
                    "ok" => Some(TestOutcome::Pass),
                    "FAILED" => Some(TestOutcome::Fail),
                    s if s.starts_with("ignored") => Some(TestOutcome::Ignored),
                    _ => None,
                };
                if let Some(o) = outcome {
                    results.push(TestResult { name: name.trim().to_string(), outcome: o, location: None });
                    continue;
                }
            }
        }
        // `thread 'NAME' panicked at file:line:col` → failure location.
        if let Some((name, path, l, c)) = parse_panic(line) {
            let p = Path::new(path);
            let path = if p.is_absolute() { p.to_path_buf() } else { root.join(p) };
            locs.insert(name, (path, l, c));
        }
    }

    for r in &mut results {
        if r.outcome == TestOutcome::Fail {
            r.location = locs.get(&r.name).cloned();
        }
    }
    let passed = results.iter().filter(|r| r.outcome == TestOutcome::Pass).count();
    let failed = results.iter().filter(|r| r.outcome == TestOutcome::Fail).count();
    TestRun { results, passed, failed }
}

/// Parse a libtest panic line into `(test name, path, line, col)`. Handles the
/// new `thread 'N' panicked at path:line:col:` and old
/// `thread 'N' panicked at 'msg', path:line:col` formats.
fn parse_panic(line: &str) -> Option<(String, &str, usize, usize)> {
    let after = line.strip_prefix("thread '")?;
    let (name, rest) = after.split_once("' panicked at ")?;
    let loc = if let Some(stripped) = rest.strip_prefix('\'') {
        // old format: `'msg', path:line:col`
        let _ = stripped;
        rest.split_once("', ").map(|(_, l)| l)?
    } else {
        rest
    };
    let loc = loc.trim_end_matches(':').trim();
    let (path, l, c) = split_path_line_col(loc)?;
    Some((name.to_string(), path, l, c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rustc_textual_output() {
        let out = "\
Compiling demo v0.1.0
error[E0425]: cannot find value `x` in this scope
 --> src/main.rs:5:13
  |
5 |     let y = x;
  |             ^ not found in this scope
warning: unused variable: `z`
 --> src/lib.rs:3:9
  |
3 |     let z = 1;
error: aborting due to previous error";
        let items = parse_build_diagnostics(out, Path::new("/proj"));
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].severity, 1);
        assert_eq!(items[0].path, Path::new("/proj/src/main.rs"));
        assert_eq!((items[0].line, items[0].col), (5, 13));
        assert!(items[0].message.contains("cannot find value"));
        assert_eq!(items[1].severity, 2);
        assert_eq!(items[1].path, Path::new("/proj/src/lib.rs"));
        assert_eq!((items[1].line, items[1].col), (3, 9));
    }

    #[test]
    fn parses_generic_file_line_col() {
        let out = "src/foo.c:10:5: error: expected ';'\nsrc/bar.c:2:1: warning: unused";
        let items = parse_build_diagnostics(out, Path::new("/p"));
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].path, Path::new("/p/src/foo.c"));
        assert_eq!((items[0].line, items[0].col, items[0].severity), (10, 5, 1));
        assert_eq!(items[1].severity, 2);
    }

    #[test]
    fn absolute_paths_are_kept() {
        let items = parse_build_diagnostics("/abs/x.rs:1:1: error: boom", Path::new("/root"));
        assert_eq!(items[0].path, Path::new("/abs/x.rs"));
    }

    #[test]
    fn prose_with_numbers_is_not_a_diagnostic() {
        // Colons + numbers but no path-like field → ignored.
        let items = parse_build_diagnostics("note: run with 3:2 for details", Path::new("/p"));
        assert!(items.is_empty());
    }

    #[test]
    fn parses_libtest_results_with_failure_location() {
        let out = "\
running 3 tests
test tests::alpha ... ok
test tests::gamma ... ignored
test tests::beta ... FAILED

failures:

---- tests::beta stdout ----
thread 'tests::beta' panicked at src/lib.rs:42:9:
assertion `left == right` failed

test result: FAILED. 1 passed; 1 failed; 1 ignored; 0 measured; 0 filtered out;";
        let run = parse_test_results(out, Path::new("/proj"));
        assert_eq!((run.passed, run.failed), (1, 1));
        assert_eq!(run.results.len(), 3);
        let beta = run.results.iter().find(|r| r.name == "tests::beta").unwrap();
        assert_eq!(beta.outcome, TestOutcome::Fail);
        assert_eq!(beta.location, Some((PathBuf::from("/proj/src/lib.rs"), 42, 9)));
        let alpha = run.results.iter().find(|r| r.name == "tests::alpha").unwrap();
        assert_eq!(alpha.outcome, TestOutcome::Pass);
        assert!(alpha.location.is_none());
    }

    #[test]
    fn parses_old_style_panic_location() {
        let out = "\
test t::x ... FAILED
thread 't::x' panicked at 'boom', src/x.rs:7:1";
        let run = parse_test_results(out, Path::new("/r"));
        let x = &run.results[0];
        assert_eq!(x.location, Some((PathBuf::from("/r/src/x.rs"), 7, 1)));
    }

    #[test]
    fn missing_command_reports_and_finishes() {
        let rx = spawn_shell_command("this-command-does-not-exist-xyz", Path::new("."));
        let msgs: Vec<RunnerMsg> = rx.iter().collect();
        assert!(matches!(msgs.last(), Some(RunnerMsg::Done(_))));
    }

    #[test]
    fn echo_streams_a_line_then_done() {
        let rx = spawn_shell_command("echo hello-runner", Path::new("."));
        let msgs: Vec<RunnerMsg> = rx.iter().collect();
        assert!(msgs.iter().any(|m| matches!(m, RunnerMsg::Line(l) if l.contains("hello-runner"))));
        assert!(matches!(msgs.last(), Some(RunnerMsg::Done(Some(0)))));
    }
}
