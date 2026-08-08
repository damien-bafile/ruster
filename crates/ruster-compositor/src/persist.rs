//! Bringing a session back: what the layout was when the compositor exited, and
//! which program was in each leaf.
//!
//! [`ruster_shell::persist`] does the layout half — it is pure, and testable
//! without a display. This is the half only the compositor can do: deciding
//! *what a leaf was*, and getting it back.
//!
//! # Knowing what a window is
//!
//! A saved leaf is only worth anything if the next boot can produce the same
//! window, and a Wayland compositor is told remarkably little about its clients:
//! a title and an app id, both of which the client chooses and neither of which
//! is a program. There is no protocol by which a client says how it was
//! launched.
//!
//! What the compositor does know is what *it* launched. Every spawn goes through
//! [`crate::lua::spawn_command`], which returns the child's pid, and a Wayland
//! client's pid comes back from its socket credentials — so a window can be tied
//! to the command that produced it whenever that command was ours. On a DRM boot
//! that is nearly all of them: the compositor is the display server, so the only
//! clients are the ones it started (see [`crate::lua::Action::Spawn`]).
//!
//! The match is against the whole ancestor chain, not just the pid we spawned,
//! because a wrapper script or a `sh -c` leaves the real client a few processes
//! below. It still fails, honestly and by design, in two cases:
//!
//! - a client that connects from outside the session (anything launched from a
//!   host terminal in the nested backend), and
//! - a window opened by a process that was already running — `footclient`, or a
//!   second Firefox window, where the new toplevel comes from a daemon that is
//!   nobody's descendant of ours.
//!
//! Those windows are saved with their title and no command. They are not
//! relaunched, because there is nothing to launch; [`Session::restore_into`]
//! drops their leaves and the rest of the layout closes over the space. Guessing
//! a command from a title would produce a session that comes back subtly wrong,
//! which is worse than one that comes back smaller.

use std::collections::HashMap;
use std::path::PathBuf;

use smithay::reexports::wayland_server::{DisplayHandle, Resource};
use smithay::wayland::shell::xdg::ToplevelSurface;

use ruster_shell::persist::{self, App, Session};
use ruster_shell::WindowId;

use crate::backend::Backend;
use crate::compositor::CompositorState;

/// How far up the process tree a client is followed before giving up. A chain
/// that long is not a wrapper script, and `/proc` is being read at every step.
const MAX_ANCESTRY: usize = 16;

/// What the compositor knows about where its windows came from, and what it is
/// still waiting for.
#[derive(Debug, Default)]
pub struct Persistence {
    /// The command line behind every process this compositor has launched.
    ///
    /// Never pruned. A pid is only ever *looked up*, and keeping a dead one
    /// costs a hash entry, while dropping it early would lose the provenance of
    /// a window whose launcher has already exited — which is exactly what a
    /// wrapper script does.
    spawned: HashMap<u32, String>,
    /// The command behind each window, for the windows we can say that about.
    commands: HashMap<WindowId, String>,
    /// The session being restored, until every client it relaunched has
    /// appeared.
    restore: Option<Restore>,
}

#[derive(Debug)]
struct Restore {
    session: Session,
    /// The app entry each relaunched process is meant to fill.
    pending: HashMap<u32, usize>,
    /// The window that turned up for an app entry.
    claimed: HashMap<usize, WindowId>,
}

impl Restore {
    /// Hand `window` to the app entry the session relaunched `pid` for,
    /// reporting whether it was waiting for one at all.
    ///
    /// `parent_of` is a parameter rather than `/proc` so this — the part that
    /// decides where every window in a restored session goes — can be tested
    /// against a process tree that is a literal.
    fn claim(
        &mut self,
        pid: u32,
        window: WindowId,
        parent_of: impl Fn(u32) -> Option<u32>,
    ) -> bool {
        let Some(app) = ancestor_in(pid, &self.pending, parent_of).copied() else {
            return false;
        };
        // The second window from the same process — a dialog, or a terminal
        // opening another of itself — is not the one the session was waiting
        // for. It belongs wherever any new window belongs, and taking the slot
        // would leave the real one homeless.
        if self.claimed.contains_key(&app) {
            return false;
        }
        self.claimed.insert(app, window);
        true
    }

    /// Every relaunched client has appeared; there is nothing left to wait for.
    fn is_complete(&self) -> bool {
        self.claimed.len() >= self.pending.len()
    }
}

impl Persistence {
    /// Launch `command` and remember the process it produced, so a window from
    /// that process can be saved as something relaunchable.
    ///
    /// The single spawn path is still [`crate::lua::spawn_command`]; this only
    /// writes down what it did. A second `Command` builder here would be a
    /// second set of rules about stdio and `WAYLAND_DISPLAY`, and the one that
    /// went wrong would be the one nobody was looking at.
    pub fn spawn(&mut self, command: &str, socket_name: Option<&str>) -> Option<u32> {
        let pid = crate::lua::spawn_command(command, socket_name)?;
        self.spawned.insert(pid, command.to_string());
        Some(pid)
    }
}

/// The first of `pid` and its ancestors that `known` has an entry for.
///
/// Split from `/proc` so the walk itself — the part with an off-by-one or an
/// infinite loop in it — can be tested against a process tree that is a literal.
fn ancestor_in<T>(
    pid: u32,
    known: &HashMap<u32, T>,
    parent_of: impl Fn(u32) -> Option<u32>,
) -> Option<&T> {
    let mut current = Some(pid);
    for _ in 0..MAX_ANCESTRY {
        let pid = current?;
        if let Some(found) = known.get(&pid) {
            return Some(found);
        }
        current = parent_of(pid);
    }
    None
}

/// The parent of `pid`, per `/proc`.
fn parent_pid(pid: u32) -> Option<u32> {
    parse_ppid(&std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
}

/// The ppid field of a `/proc/<pid>/stat` line.
///
/// The second field is the executable's name in parentheses, and a program is
/// free to be called `foo (bar) baz`, so the fields are counted from the *last*
/// closing parenthesis rather than split from the front. Splitting on whitespace
/// reads the state letter as the ppid for any such program, which is a wrong
/// answer rather than no answer — it would attribute a window to whatever
/// process happens to have that pid.
fn parse_ppid(stat: &str) -> Option<u32> {
    let after_comm = stat.rsplit_once(')')?.1;
    after_comm.split_whitespace().nth(1)?.parse().ok()
}

/// The pid of the process on the other end of a toplevel's connection.
///
/// `None` for a client that has already gone away, and for the socket
/// credentials being unavailable at all — neither is worth a log line, since the
/// only consequence is a window that cannot be relaunched.
pub fn client_pid(surface: &ToplevelSurface, display: &DisplayHandle) -> Option<u32> {
    let client = surface.wl_surface().client()?;
    u32::try_from(client.get_credentials(display).ok()?.pid).ok()
}

/// Where the compositor keeps its state: the same `~/.config/ruster` the config
/// is read from and the editor writes its sessions under. A second directory for
/// one file would be one more place to look when a session does not come back.
pub fn ruster_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|p| p.join("ruster"))
}

impl<B: Backend + 'static> CompositorState<B> {
    /// Write the current layout out. Called once, on the way out of the event
    /// loop, by both backends.
    ///
    /// A window the compositor cannot relaunch is still recorded, with its title
    /// and no command, so the file says what was actually on screen. An app that
    /// was relaunched at this boot and never appeared is *not* recorded: the
    /// session is what was on screen, not a list of things that were meant to
    /// be.
    pub fn save_session(&self) {
        let Some(dir) = ruster_dir() else {
            tracing::warn!("no config directory; the session was not saved");
            return;
        };
        let panes = &self.panes;
        let session = Session::capture(&self.workspaces, |id| {
            // A pane is recreated, not relaunched, so it saves as its own shape
            // rather than as a window whose command we could not identify —
            // which `rebuild` drops, silently losing it from the layout.
            if let Some(pane) = panes.get(&id) {
                return App::pane(pane.title.clone());
            }
            App {
                command: self.persist.commands.get(&id).cloned(),
                title: self
                    .shell
                    .windows()
                    .find(|w| w.id == id)
                    .map(|w| w.title.clone())
                    .unwrap_or_default(),
                pane: false,
            }
        });
        let relaunchable = session.apps.iter().filter(|a| a.command.is_some()).count();
        match persist::save(&dir, &self.seat_name(), &session) {
            Ok(()) => tracing::info!(
                windows = session.apps.len(),
                relaunchable,
                workspace = session.active,
                "session saved"
            ),
            Err(err) => tracing::warn!(%err, "failed to save the session"),
        }
    }

    /// Relaunch the saved session, reporting whether there was one to relaunch.
    ///
    /// Only the spawning happens here. Nothing is laid out until the clients
    /// connect, because there is nothing to lay out — a leaf needs a window —
    /// and each one that arrives rebuilds the layout over everything that has
    /// arrived so far (see [`Self::place_restored_window`]). So a session comes
    /// back progressively, and one client that never starts costs its own leaf
    /// and nothing else.
    pub fn restore_session(&mut self, socket_name: &str) -> bool {
        let Some(session) = ruster_dir().and_then(|dir| persist::load(&dir, &self.seat_name()))
        else {
            return false;
        };
        let mut pending = HashMap::new();
        // Panes come back immediately: there is no process to wait for, so the
        // leaf can be filled in this pass rather than when some client appears.
        let mut restored_panes: Vec<(usize, WindowId)> = Vec::new();
        for (app, entry) in session.apps.iter().enumerate() {
            if entry.pane {
                let area = self.output_rect();
                let id = self.shell.add_window(entry.title.clone(), area.w, area.h);
                self.panes
                    .insert(id, crate::pane::EditorPane::new(entry.title.clone()));
                restored_panes.push((app, id));
                continue;
            }
            let Some(command) = &entry.command else {
                tracing::info!(
                    title = %entry.title,
                    "not relaunching a window the compositor did not start"
                );
                continue;
            };
            if let Some(pid) = self.persist.spawn(command, Some(socket_name)) {
                pending.insert(pid, app);
            }
        }
        tracing::info!(
            windows = session.apps.len(),
            relaunched = pending.len(),
            panes = restored_panes.len(),
            workspace = session.active,
            "restoring session"
        );
        // The workspace that was on screen, once, here — and never again while
        // the clients trickle in, which would move the screen under whoever is
        // already typing.
        self.switch_workspace(session.active);

        // Panes are already in hand, so seed them into the claim table. When
        // nothing was relaunched they are the whole restore and the tree is
        // rebuilt here; otherwise they are simply filled in before the first
        // client arrives, and `place_restored_window` completes the picture.
        let claimed: HashMap<usize, WindowId> = restored_panes.into_iter().collect();
        if pending.is_empty() {
            if !claimed.is_empty() {
                session.restore_into(&mut self.workspaces, |app| claimed.get(&app).copied());
                self.reconfigure_tiles();
                self.shell.focus = self.workspaces.focus_for_active(None);
                tracing::info!(panes = claimed.len(), "restored a session of panes");
            }
            return false;
        }
        self.persist.restore = Some(Restore {
            session,
            pending,
            claimed,
        });
        true
    }

    /// Record where `window` came from and, if the session was waiting for it,
    /// put the layout back around it. Reports whether the session placed the
    /// window, in which case the caller must not insert it itself.
    ///
    /// The layout is rebuilt from the file on every arrival rather than the
    /// window being threaded into the tree at the right point: "the saved layout
    /// over the windows that exist" is one operation with one meaning, whereas
    /// inserting into a partly-restored tree would have to invent an answer for
    /// every leaf that has not arrived yet.
    pub fn place_restored_window(&mut self, window: WindowId, pid: Option<u32>) -> bool {
        let Some(pid) = pid else {
            return false;
        };
        if let Some(command) = ancestor_in(pid, &self.persist.spawned, parent_pid).cloned() {
            self.persist.commands.insert(window, command);
        }
        let Some(mut restore) = self.persist.restore.take() else {
            return false;
        };
        let placed = restore.claim(pid, window, parent_pid);
        if placed {
            restore.session.restore_into(&mut self.workspaces, |app| {
                restore.claimed.get(&app).copied()
            });
            tracing::info!(
                ?window,
                restored = restore.claimed.len(),
                of = restore.pending.len(),
                "put a window back where the session had it"
            );
        }
        // Once everything that was relaunched has appeared there is nothing left
        // to wait for, and every later window is an ordinary one.
        if !restore.is_complete() {
            self.persist.restore = Some(restore);
        }
        placed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `pid (comm) state ppid ...` — the first four fields are all this reads.
    fn stat(pid: u32, comm: &str, ppid: u32) -> String {
        format!("{pid} ({comm}) S {ppid} {pid} {pid} 0 -1 4194304 1234 0 0 0\n")
    }

    #[test]
    fn the_parent_is_read_from_the_field_after_the_command_name() {
        assert_eq!(parse_ppid(&stat(1234, "foot", 42)), Some(42));
        assert_eq!(parse_ppid(&stat(1, "systemd", 0)), Some(0));
    }

    #[test]
    fn a_command_name_with_spaces_and_brackets_in_it_still_reads() {
        // A process may call itself anything, and it lands in this file
        // unescaped. Counting fields from the front reads the state letter `S`
        // as the parent pid — which parses as nothing, or worse, as a pid that
        // belongs to somebody else.
        assert_eq!(parse_ppid(&stat(7, "Web Content", 3)), Some(3));
        assert_eq!(parse_ppid(&stat(7, "a (b) c", 3)), Some(3));
        assert_eq!(parse_ppid("garbage with no brackets"), None);
        assert_eq!(parse_ppid("7 (foot) S"), None, "truncated");
    }

    /// 100 spawned the shell 200, which exec'd the client 300; 900 is unrelated.
    fn tree() -> HashMap<u32, u32> {
        HashMap::from([(300, 200), (200, 100), (100, 1), (900, 1)])
    }

    fn known() -> HashMap<u32, &'static str> {
        HashMap::from([(100, "sh -c foot")])
    }

    #[test]
    fn a_client_below_the_process_we_spawned_is_still_ours() {
        // The case this exists for: a wrapper script, or `sh -c`, between the
        // spawn and the program that actually connects. Matching only the pid we
        // spawned would call every such window unrelaunchable.
        let (parents, known) = (tree(), known());
        let found = ancestor_in(300, &known, |pid| parents.get(&pid).copied());
        assert_eq!(found, Some(&"sh -c foot"));
    }

    #[test]
    fn a_client_we_did_not_spawn_is_nobodys() {
        // Something launched from a host terminal, or a window handed over by a
        // daemon that was already running. Attributing it to a command would put
        // a wrong entry in the session file, which is worse than no entry.
        let (parents, known) = (tree(), known());
        assert_eq!(
            ancestor_in(900, &known, |pid| parents.get(&pid).copied()),
            None
        );
        assert_eq!(
            ancestor_in(300, &known, |_| None),
            None,
            "and a chain that cannot be walked at all"
        );
    }

    /// A session that relaunched two clients, as pids 100 and 900.
    fn restore() -> Restore {
        Restore {
            session: Session::default(),
            pending: HashMap::from([(100, 0), (900, 1)]),
            claimed: HashMap::new(),
        }
    }

    #[test]
    fn a_relaunched_client_claims_the_leaf_it_was_launched_for() {
        let parents = tree();
        let mut restore = restore();
        // 300 is two processes below the 100 we spawned.
        assert!(restore.claim(300, WindowId(1), |pid| parents.get(&pid).copied()));
        assert_eq!(restore.claimed.get(&0), Some(&WindowId(1)));
        assert!(!restore.is_complete(), "one of the two is still out");

        assert!(restore.claim(900, WindowId(2), |_| None));
        assert!(restore.is_complete());
    }

    #[test]
    fn a_second_window_from_the_same_client_does_not_take_the_slot() {
        // A terminal that opens a second window, or any client with a dialog.
        // Letting it claim again would put it where the first one belongs and
        // leave the window the session was actually waiting for homeless.
        let mut restore = restore();
        assert!(restore.claim(100, WindowId(1), |_| None));
        assert!(!restore.claim(100, WindowId(2), |_| None));
        assert_eq!(restore.claimed.get(&0), Some(&WindowId(1)));
        assert_eq!(restore.claimed.len(), 1);
    }

    #[test]
    fn a_window_the_session_was_not_waiting_for_claims_nothing() {
        let mut restore = restore();
        assert!(!restore.claim(555, WindowId(1), |_| None));
        assert!(restore.claimed.is_empty());
    }

    #[test]
    fn a_cycle_in_the_process_tree_does_not_hang_the_compositor() {
        // /proc cannot produce one, but this walks whatever it is given, and a
        // compositor that spins here takes the screen with it.
        assert_eq!(ancestor_in(1, &known(), Some), None);
        assert_eq!(ancestor_in(1, &known(), |pid| Some(pid + 1)), None);
    }
}
