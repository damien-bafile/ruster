//! Saving the shape of a session and putting it back on the next boot.
//!
//! What is saved is the *layout*: for each of the nine workspaces, the
//! container tree with its ratios and the floating windows with their
//! rectangles, plus which workspace was on screen. What cannot be saved is a
//! window — a leaf is a live Wayland client, and a [`WindowId`] means nothing to
//! the next boot. So each leaf is saved as an entry in an app table holding the
//! command line that produced the window, when the compositor knows it, and the
//! title the window went by, which is all it knows otherwise.
//!
//! That distinction is the whole design. A window the compositor launched
//! itself can be launched again; one that connected from outside, or that a
//! pre-existing process opened, cannot be, and no amount of title matching would
//! make it so. Those entries are saved anyway — the file is then an honest
//! record of what was on screen — and [`Session::restore_into`] simply drops
//! them, which is why restoring is defined as "the saved layout over the windows
//! that actually exist" rather than "the saved layout".
//!
//! # Why a hand-written format
//!
//! The same reason `ruster-core`'s session format is hand-written: this crate
//! has no dependencies at all and one file is not worth serde. The format is
//! line-based, and each tree is written in **preorder** with the child count on
//! the split line:
//!
//! ```text
//! ruster-workspaces 1
//! active 1
//! app foot
//! title ~/src
//! app
//! title Firefox
//! workspace 1
//! split h 2 0.6 0.4
//! leaf 0
//! leaf 1
//! ```
//!
//! `ruster-core`'s splits are binary and need no count; these are n-ary, so
//! without one the parser could not tell where a split's children end. Writing
//! brackets instead would need a second token type and a stack; the count is one
//! number and keeps the parse a single cursor over the lines.
//!
//! Anything unexpected — an unknown keyword, a leaf naming an app that is not in
//! the table, ratios that do not sum to 1, two leaves claiming the same app —
//! makes the whole file invalid rather than restoring part of a layout. A
//! half-restored session is worse than an empty one: the windows come back in
//! places nobody arranged, and there is no way to tell that from a bug.

use std::path::{Path, PathBuf};

use crate::tree::{normalise_ratios, Layout, Node, NodeId, Rect, Tree};
use crate::window::WindowId;
use crate::workspace::{Workspaces, WORKSPACE_COUNT};

/// Bumped only on a breaking change; an older or newer file is refused rather
/// than guessed at.
const VERSION: u32 = 1;
const MAGIC: &str = "ruster-workspaces";

const SLOTS: usize = WORKSPACE_COUNT as usize;

/// What produced one window, as far as the next boot is concerned.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct App {
    /// The command line that would launch this window again, or `None` when the
    /// compositor did not launch it and so cannot say.
    pub command: Option<String>,
    /// What the window called itself. Not used to restore anything — a title is
    /// not a program — but it is what makes the saved file legible, and it is
    /// the only trace left of a window that cannot be relaunched.
    pub title: String,
    /// True when this leaf was an editor pane rather than a client window.
    ///
    /// A pane needs no command: it is recreated rather than relaunched. Without
    /// this it would save as a window with no command, which is the shape of
    /// "we could not identify this program" — and `rebuild` drops those, so a
    /// restored layout would silently lose every pane in it.
    pub pane: bool,
    /// The file an editor pane was showing, when it had one.
    ///
    /// Carried as the `pane` keyword's argument rather than in `command`: a
    /// path is opened, not executed, and putting it where a command line goes
    /// would eventually get one of them run.
    pub pane_path: Option<String>,
}

impl App {
    /// An editor pane, which has no command line to record.
    pub fn pane(title: impl Into<String>) -> Self {
        App {
            command: None,
            title: title.into(),
            pane: true,
            pane_path: None,
        }
    }

    /// An editor pane showing a file, which is reopened rather than relaunched.
    pub fn pane_with_path(title: impl Into<String>, path: impl Into<String>) -> Self {
        App {
            pane_path: Some(path.into()),
            ..App::pane(title)
        }
    }
}

/// One node of a saved container tree. The n-ary mirror of [`Node`], with app
/// table indices where the live tree has window handles.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeSnapshot {
    Leaf(usize),
    Split {
        layout: Layout,
        /// One per child, summing to 1.0, as in [`Node::Split`].
        ratios: Vec<f32>,
        children: Vec<NodeSnapshot>,
    },
}

/// One workspace: its tiling, and its floating windows bottom of the stack
/// first — the order [`Workspaces::layout`] depends on.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorkspaceSnapshot {
    pub tiled: Option<NodeSnapshot>,
    pub floating: Vec<(usize, Rect)>,
}

/// One saved session: every workspace's layout, and the app behind each leaf.
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    /// Referenced exactly once each, across all nine workspaces. See
    /// [`Session::decode`] for why that is enforced rather than assumed.
    pub apps: Vec<App>,
    pub workspaces: [WorkspaceSnapshot; SLOTS],
    /// The workspace that was on screen, 1-based.
    pub active: u32,
}

impl Default for Session {
    fn default() -> Self {
        Session {
            apps: Vec::new(),
            workspaces: Default::default(),
            active: 1,
        }
    }
}

impl Session {
    /// Take a snapshot of every workspace. `app_of` says what each window was,
    /// which only the compositor can answer.
    pub fn capture(workspaces: &Workspaces, app_of: impl Fn(WindowId) -> App) -> Session {
        let mut apps = Vec::new();
        let mut slots: [WorkspaceSnapshot; SLOTS] = Default::default();
        for (i, slot) in slots.iter_mut().enumerate() {
            let number = i as u32 + 1;
            let Some(tree) = workspaces.tree_on(number) else {
                continue;
            };
            slot.tiled = tree
                .root()
                .and_then(|root| capture_node(tree, root, &mut apps, &app_of));
            for (window, rect) in workspaces.floating_on(number) {
                apps.push(app_of(*window));
                slot.floating.push((apps.len() - 1, *rect));
            }
        }
        Session {
            apps,
            workspaces: slots,
            active: workspaces.active(),
        }
    }

    /// Rebuild the saved layout over the windows that exist, in `workspaces`.
    ///
    /// `window_for` answers which live window an app entry ended up as, or
    /// `None` for one that has not turned up — a client still starting, or one
    /// that could never be relaunched at all. Those leaves are dropped and their
    /// siblings share out the space, so the layout is always the saved one
    /// restricted to what is really there, never a hole held open for something
    /// that may never come.
    ///
    /// Windows already in `workspaces` that the session says nothing about keep
    /// the workspace they were on. They are the ones that opened while the
    /// restore was still filling in — the alternative, dropping them, would lose
    /// a window the user is looking at.
    ///
    /// The active workspace is left alone: this runs again on every arriving
    /// client, and yanking the screen back each time would make the first
    /// seconds of a session unusable.
    pub fn restore_into(
        &self,
        workspaces: &mut Workspaces,
        window_for: impl Fn(usize) -> Option<WindowId>,
    ) {
        let mut trees: [Tree; SLOTS] = std::array::from_fn(|_| Tree::new());
        let mut floating: [Vec<(WindowId, Rect)>; SLOTS] = std::array::from_fn(|_| Vec::new());
        let mut placed: Vec<WindowId> = Vec::new();

        for (i, snapshot) in self.workspaces.iter().enumerate() {
            if let Some(node) = &snapshot.tiled {
                trees[i] = Tree::rebuild(node, &window_for);
                placed.extend(trees[i].windows());
            }
            for (app, rect) in &snapshot.floating {
                if let Some(window) = window_for(*app) {
                    floating[i].push((window, *rect));
                    placed.push(window);
                }
            }
        }

        for number in 1..=WORKSPACE_COUNT {
            let i = (number - 1) as usize;
            let carried = workspaces
                .tree_on(number)
                .map(Tree::windows)
                .unwrap_or_default();
            for window in carried {
                if !placed.contains(&window) {
                    trees[i].insert(window, None, Layout::Horizontal);
                }
            }
            for (window, rect) in workspaces.floating_on(number) {
                if !placed.contains(window) {
                    floating[i].push((*window, *rect));
                }
            }
        }

        workspaces.replace_layout(trees, floating);
    }

    pub fn encode(&self) -> String {
        let mut out = format!("{MAGIC} {VERSION}\n");
        out.push_str(&format!("active {}\n", self.active));
        for app in &self.apps {
            // A distinct keyword, so a pane is never mistaken for a window
            // whose command we failed to identify.
            if app.pane {
                // The path as the keyword's argument, so a pane with no file
                // stays a bare `pane` and older session files still parse.
                match &app.pane_path {
                    Some(path) => out.push_str(&format!("pane {}\n", one_line(path))),
                    None => out.push_str("pane\n"),
                }
            } else {
                out.push_str(&format!(
                    "app {}\n",
                    one_line(app.command.as_deref().unwrap_or(""))
                ));
            }
            let title = one_line(&app.title);
            if !title.is_empty() {
                out.push_str(&format!("title {title}\n"));
            }
        }
        for (i, workspace) in self.workspaces.iter().enumerate() {
            if workspace.tiled.is_none() && workspace.floating.is_empty() {
                continue;
            }
            out.push_str(&format!("workspace {}\n", i + 1));
            if let Some(node) = &workspace.tiled {
                write_node(node, &mut out);
            }
            for (app, r) in &workspace.floating {
                out.push_str(&format!("float {app} {} {} {} {}\n", r.x, r.y, r.w, r.h));
            }
        }
        out
    }

    /// Parse a session file, or `None` if it is not one we understand.
    pub fn decode(text: &str) -> Option<Session> {
        let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
        let header = lines.next()?;
        let (magic, version) = header.split_once(' ')?;
        if magic != MAGIC || version.parse::<u32>().ok()? != VERSION {
            return None;
        }

        let rest: Vec<&str> = lines.collect();
        let mut at = 0;
        let active: u32 = value(rest.first()?, "active")?.parse().ok()?;
        if !(1..=WORKSPACE_COUNT).contains(&active) {
            return None;
        }
        at += 1;

        let mut apps: Vec<App> = Vec::new();
        while let Some(line) = rest.get(at) {
            if line.trim() == "pane" || line.starts_with("pane ") {
                let path = line
                    .strip_prefix("pane ")
                    .map(str::trim)
                    .filter(|p| !p.is_empty());
                apps.push(App {
                    command: None,
                    title: String::new(),
                    pane: true,
                    pane_path: path.map(str::to_string),
                });
            } else if let Some(command) = value(line, "app") {
                apps.push(App {
                    command: (!command.is_empty()).then(|| command.to_string()),
                    title: String::new(),
                    pane: false,
                    pane_path: None,
                });
            } else if let Some(title) = value(line, "title") {
                // A title with no app in front of it is not a file we wrote.
                apps.last_mut()?.title = title.to_string();
            } else {
                break;
            }
            at += 1;
        }

        let mut workspaces: [WorkspaceSnapshot; SLOTS] = Default::default();
        let mut seen = [false; SLOTS];
        while let Some(line) = rest.get(at) {
            // Anything that is not a workspace header here is junk, and junk
            // means the rest of the file cannot be trusted either.
            let number: u32 = value(line, "workspace")?.parse().ok()?;
            let i = (1..=WORKSPACE_COUNT)
                .contains(&number)
                .then(|| (number - 1) as usize)?;
            if seen[i] {
                return None;
            }
            seen[i] = true;
            at += 1;

            if matches!(keyword(rest.get(at)?), "split" | "leaf") {
                workspaces[i].tiled = Some(read_node(&rest, &mut at)?);
            }
            while let Some(rect) = rest.get(at).and_then(|l| value(l, "float")) {
                workspaces[i].floating.push(read_float(rect)?);
                at += 1;
            }
        }

        let session = Session {
            apps,
            workspaces,
            active,
        };
        // Every app placed once and only once. A leaf pointing past the table is
        // the obvious corruption; two leaves pointing at the same app is the
        // subtle one — it would restore one window into two places, where the
        // tree finds only the first and the second half of the layout silently
        // vanishes. And an app nothing points at would be relaunched into a
        // window with nowhere to go, which is worse: it exists and is invisible.
        (session.references() == (0..session.apps.len()).collect::<Vec<_>>()).then_some(session)
    }

    /// Every app index the workspaces refer to, sorted. Equal to `0..apps.len()`
    /// exactly when each app is placed once.
    fn references(&self) -> Vec<usize> {
        let mut out = Vec::new();
        for workspace in &self.workspaces {
            if let Some(node) = &workspace.tiled {
                collect_leaves(node, &mut out);
            }
            out.extend(workspace.floating.iter().map(|(app, _)| *app));
        }
        out.sort_unstable();
        out
    }
}

/// The rest of `line` after `key`, or `None` if it starts with something else.
/// The value runs to the end of the line, so commands and titles keep their
/// spaces without quoting.
fn value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    match line.strip_prefix(key)? {
        "" => Some(""),
        rest => rest.strip_prefix(' ').map(str::trim),
    }
}

fn keyword(line: &str) -> &str {
    line.split_whitespace().next().unwrap_or_default()
}

/// A command or title with anything that would break the line-based format
/// taken out. Clients choose their own titles and are free to put a newline in
/// one; that would otherwise write a line the parser reads as a keyword, and
/// silently invalidate the whole file at the next boot.
fn one_line(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

fn write_node(node: &NodeSnapshot, out: &mut String) {
    match node {
        NodeSnapshot::Leaf(app) => out.push_str(&format!("leaf {app}\n")),
        NodeSnapshot::Split {
            layout,
            ratios,
            children,
        } => {
            let axis = if *layout == Layout::Vertical {
                'v'
            } else {
                'h'
            };
            out.push_str(&format!("split {axis} {}", children.len()));
            for r in ratios {
                // Full precision, not the four decimals `ruster-core` writes for
                // its one ratio. Rust prints the shortest string that parses
                // back to the same `f32`, so a saved tree is the tree that comes
                // back — and the sum check in `read_node` cannot then trip over
                // rounding that this line introduced.
                out.push_str(&format!(" {r}"));
            }
            out.push('\n');
            // Preorder: every child follows, in order, and the count above says
            // how many of them to read.
            for child in children {
                write_node(child, out);
            }
        }
    }
}

fn read_node(lines: &[&str], at: &mut usize) -> Option<NodeSnapshot> {
    let line = lines.get(*at)?;
    *at += 1;
    let mut fields = line.split_whitespace();
    match fields.next()? {
        "leaf" => {
            let app = fields.next()?.parse().ok()?;
            fields.next().is_none().then_some(NodeSnapshot::Leaf(app))
        }
        "split" => {
            let layout = match fields.next()? {
                "h" => Layout::Horizontal,
                "v" => Layout::Vertical,
                _ => return None,
            };
            // A split with fewer than two children is not a split; the tree
            // collapses those rather than writing them.
            let count: usize = fields.next()?.parse().ok()?;
            if count < 2 {
                return None;
            }
            // Not preallocated: `count` comes from the file, and reserving it up
            // front would let a corrupt one ask for gigabytes before the first
            // missing ratio proves it wrong.
            let mut ratios = Vec::new();
            for _ in 0..count {
                let ratio: f32 = fields.next()?.parse().ok()?;
                if !(ratio > 0.0 && ratio <= 1.0) {
                    return None;
                }
                ratios.push(ratio);
            }
            if fields.next().is_some() {
                return None;
            }
            // The invariant `Tree::layout` divides by. The tolerance is for a
            // file someone has edited by hand — `Tree::rebuild` renormalises
            // anyway — while still refusing one whose numbers say the layout
            // tiles with a gap or an overlap.
            let sum: f32 = ratios.iter().sum();
            if (sum - 1.0).abs() > 1e-3 {
                return None;
            }
            let mut children = Vec::new();
            for _ in 0..count {
                children.push(read_node(lines, at)?);
            }
            Some(NodeSnapshot::Split {
                layout,
                ratios,
                children,
            })
        }
        _ => None,
    }
}

fn read_float(fields: &str) -> Option<(usize, Rect)> {
    let mut f = fields.split_whitespace();
    let app = f.next()?.parse().ok()?;
    let rect = Rect::new(
        f.next()?.parse().ok()?,
        f.next()?.parse().ok()?,
        f.next()?.parse().ok()?,
        f.next()?.parse().ok()?,
    );
    // A float with no area cannot be seen or grabbed; a file claiming one is
    // corrupt rather than merely odd.
    if rect.w <= 0 || rect.h <= 0 || f.next().is_some() {
        return None;
    }
    Some((app, rect))
}

fn collect_leaves(node: &NodeSnapshot, out: &mut Vec<usize>) {
    match node {
        NodeSnapshot::Leaf(app) => out.push(*app),
        NodeSnapshot::Split { children, .. } => {
            for child in children {
                collect_leaves(child, out);
            }
        }
    }
}

/// Snapshot the subtree at `id`, appending an app entry per leaf.
///
/// `None` for a subtree with nothing in it. That cannot happen for a tree the
/// arena built — it collapses empty splits as they appear — but making the walk
/// total is cheaper than deciding what a save should do when it does.
fn capture_node(
    tree: &Tree,
    id: NodeId,
    apps: &mut Vec<App>,
    app_of: &impl Fn(WindowId) -> App,
) -> Option<NodeSnapshot> {
    match tree.node(id)? {
        Node::Leaf(window) => {
            apps.push(app_of(*window));
            Some(NodeSnapshot::Leaf(apps.len() - 1))
        }
        Node::Split {
            layout,
            children,
            ratios,
        } => {
            let mut kept = Vec::new();
            let mut kept_ratios = Vec::new();
            for (i, child) in children.iter().enumerate() {
                if let Some(node) = capture_node(tree, *child, apps, app_of) {
                    kept.push(node);
                    kept_ratios.push(ratios.get(i).copied().unwrap_or(0.0));
                }
            }
            match kept.len() {
                0 => None,
                1 => kept.pop(),
                _ => Some(NodeSnapshot::Split {
                    layout: *layout,
                    ratios: normalise_ratios(&kept_ratios),
                    children: kept,
                }),
            }
        }
    }
}

/// Where a compositor session lives: one file per seat, named by a hash of the
/// seat name so a second seat keeps its own layout and the name is a valid
/// filename whatever the seat is called.
///
/// The hash is spelled out here rather than borrowed from
/// `ruster_core::session::session_path`, which does the same thing for editor
/// sessions. Reusing it would mean this crate — which has no dependencies, and
/// is therefore testable without a display or an editor — depending on the whole
/// editor core for five lines of FNV, and would put compositor state in the
/// editor's `sessions/` directory, where a project whose path hashed the same
/// would collide with it.
pub fn session_path(state_dir: &Path, seat: &str) -> PathBuf {
    // FNV-1a: stable across runs and platforms, which `DefaultHasher` is not
    // guaranteed to be — a session must still be found tomorrow.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in seat.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    state_dir
        .join("workspaces")
        .join(format!("{hash:016x}.workspaces"))
}

/// Write a session, creating the directory if needed. Errors are returned so the
/// caller can report them; losing a layout is not worth failing an exit over.
pub fn save(state_dir: &Path, seat: &str, session: &Session) -> std::io::Result<()> {
    let path = session_path(state_dir, seat);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, session.encode())
}

/// Read a seat's session, or `None` when there is none or it is unreadable.
pub fn load(state_dir: &Path, seat: &str) -> Option<Session> {
    Session::decode(&std::fs::read_to_string(session_path(state_dir, seat)).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect {
        x: 0,
        y: 0,
        w: 1000,
        h: 800,
    };

    fn w(n: u32) -> WindowId {
        WindowId(n)
    }

    /// An app table entry named after the window, so a restore can be checked
    #[test]
    fn a_pane_remembers_the_file_it_was_showing() {
        // Without the path a restored pane comes back empty wearing its old
        // name, which looks like the file failed to load rather than like it
        // was never recorded.
        let mut ws = Workspaces::new();
        ws.insert(w(1), None, Layout::Horizontal);
        let session = Session::capture(&ws, |_| App::pane_with_path("main.rs", "/src/main.rs"));
        let text = session.encode();
        assert!(text.contains("pane /src/main.rs"), "got:\n{text}");

        let back = Session::decode(&text).unwrap();
        assert!(back.apps[0].pane);
        assert_eq!(back.apps[0].pane_path.as_deref(), Some("/src/main.rs"));
        assert_eq!(back.apps[0].title, "main.rs");
    }

    #[test]
    fn a_scratch_pane_stays_a_bare_keyword() {
        // Both so the file stays readable and so a session written before paths
        // existed still parses — a format that could not read its own history
        // would lose the layout it was meant to preserve.
        let mut ws = Workspaces::new();
        ws.insert(w(1), None, Layout::Horizontal);
        let session = Session::capture(&ws, |_| App::pane("scratch"));
        assert!(session.encode().contains("\npane\n"));

        let old = "ruster-workspaces 1\nactive 1\npane\ntitle scratch\nworkspace 1\nleaf 0\n";
        let back = Session::decode(old).expect("an older session file must still parse");
        assert!(back.apps[0].pane);
        assert_eq!(back.apps[0].pane_path, None);
    }

    #[test]
    fn a_path_with_spaces_survives_the_round_trip() {
        // The format is line-oriented and the path is the keyword's argument,
        // so a space in it is not a separator.
        let mut ws = Workspaces::new();
        ws.insert(w(1), None, Layout::Horizontal);
        let session = Session::capture(&ws, |_| {
            App::pane_with_path("notes", "/home/a b/my notes.md")
        });
        let back = Session::decode(&session.encode()).unwrap();
        assert_eq!(
            back.apps[0].pane_path.as_deref(),
            Some("/home/a b/my notes.md")
        );
    }

    #[test]
    fn a_pane_survives_the_round_trip_as_a_pane() {
        // A pane saved as "a window with no command" is indistinguishable from
        // one whose program we failed to identify — and `rebuild` drops those,
        // so every pane would vanish from a restored layout without a word.
        let mut ws = Workspaces::new();
        ws.insert(w(1), None, Layout::Horizontal);
        ws.insert(w(2), Some(w(1)), Layout::Horizontal);
        let session = Session::capture(&ws, |id| {
            if id == w(2) {
                App::pane("scratch")
            } else {
                App {
                    command: Some("foot".into()),
                    title: "term".into(),
                    pane: false,
                    pane_path: None,
                }
            }
        });
        let text = session.encode();
        assert!(
            text.contains("\npane\n"),
            "expected a pane keyword in:\n{text}"
        );

        let back = Session::decode(&text).expect("a session we wrote must parse");
        assert_eq!(back.apps.len(), 2);
        assert!(back.apps[1].pane, "the pane must come back a pane");
        assert_eq!(back.apps[1].title, "scratch");
        assert!(!back.apps[0].pane, "the client must not");
        assert_eq!(back.apps[0].command.as_deref(), Some("foot"));
    }

    #[test]
    fn a_pane_is_not_confused_with_a_window_we_could_not_identify() {
        // Both have no command. Only one should come back as a pane.
        let mut ws = Workspaces::new();
        ws.insert(w(1), None, Layout::Horizontal);
        let session = Session::capture(&ws, |_| App {
            command: None,
            title: "mystery".into(),
            pane: false,
            pane_path: None,
        });
        let back = Session::decode(&session.encode()).unwrap();
        assert!(!back.apps[0].pane);
        assert_eq!(back.apps[0].command, None);
    }

    /// window by window.
    fn app_of(window: WindowId) -> App {
        App {
            command: Some(format!("foot -e app{}", window.0)),
            title: format!("window {}", window.0),
            pane: false,
            pane_path: None,
        }
    }

    /// Two windows side by side on workspace 1, three stacked on workspace 4
    /// with one of them floating, showing workspace 4.
    fn live() -> Workspaces {
        let mut ws = Workspaces::new();
        ws.insert(w(1), None, Layout::Horizontal);
        ws.insert(w(2), Some(w(1)), Layout::Horizontal);
        ws.tree_mut().resize(w(1), Direction::Right, 0.1);

        ws.switch_to(4);
        ws.insert(w(3), None, Layout::Vertical);
        ws.insert(w(4), Some(w(3)), Layout::Vertical);
        ws.insert(w(5), Some(w(4)), Layout::Vertical);
        ws.toggle_floating(w(5), AREA);
        ws
    }

    use crate::tree::Direction;

    /// Restore a capture into empty workspaces, with every app resolving to the
    /// window it was captured from.
    fn round_trip(source: &Workspaces) -> Workspaces {
        let session = Session::capture(source, app_of);
        let decoded = Session::decode(&session.encode()).expect("a capture is decodable");
        assert_eq!(decoded, session, "encoding is lossless");
        let mut out = Workspaces::new();
        out.switch_to(decoded.active);
        decoded.restore_into(&mut out, |app| window_named(&decoded.apps[app]));
        out
    }

    /// The window an app entry was captured from, per [`app_of`].
    fn window_named(app: &App) -> Option<WindowId> {
        app.title
            .strip_prefix("window ")?
            .parse()
            .ok()
            .map(WindowId)
    }

    fn layouts(ws: &Workspaces) -> Vec<Vec<(WindowId, Rect)>> {
        (1..=WORKSPACE_COUNT)
            .map(|n| {
                let mut all = ws.tree_on(n).map(|t| t.layout(AREA)).unwrap_or_default();
                all.extend(ws.floating_on(n).iter().copied());
                all
            })
            .collect()
    }

    #[test]
    fn every_workspace_comes_back_where_it_was() {
        // The whole point: the same windows, in the same rectangles, on the same
        // workspaces, with the same one on screen.
        let before = live();
        let after = round_trip(&before);
        assert_eq!(layouts(&after), layouts(&before));
        assert_eq!(after.active(), before.active());
        assert!(after.is_floating(w(5)));
    }

    #[test]
    fn a_resized_split_keeps_its_ratios() {
        // Ratios are the part a shape-only format would lose: the windows would
        // come back in the right places and the wrong sizes, which looks like
        // the restore worked.
        let before = live();
        let after = round_trip(&before);
        let widths: Vec<i32> = after
            .tree_on(1)
            .unwrap()
            .layout(AREA)
            .iter()
            .map(|(_, r)| r.w)
            .collect();
        assert_eq!(widths, vec![600, 400]);
    }

    #[test]
    fn an_empty_session_restores_to_nothing() {
        let empty = Workspaces::new();
        let after = round_trip(&empty);
        assert_eq!(layouts(&after), layouts(&empty));
        assert!(Session::capture(&empty, app_of).apps.is_empty());
    }

    #[test]
    fn a_window_that_never_comes_back_leaves_its_siblings_the_space() {
        // A leaf held open for a client that may never connect would be a hole
        // in the layout with no way to close it.
        let session = Session::capture(&live(), app_of);
        let mut out = Workspaces::new();
        session.restore_into(&mut out, |app| {
            window_named(&session.apps[app]).filter(|id| *id != w(2))
        });
        assert_eq!(out.tree_on(1).unwrap().layout(AREA), vec![(w(1), AREA)]);
        // And the workspace nothing was dropped from is untouched.
        assert_eq!(out.tree_on(4).unwrap().windows(), vec![w(3), w(4)]);
    }

    #[test]
    fn the_survivors_of_a_dropped_leaf_keep_their_proportions() {
        // A row of three, resized to 0.5 / 0.1667 / 0.3333. Losing the last one
        // should leave the other two in the 3:1 they were in, not an even split
        // — and, whatever it leaves, the row still has to add up to the output
        // or there is a seam down the screen.
        let mut ws = Workspaces::new();
        ws.insert(w(1), None, Layout::Horizontal);
        ws.insert(w(2), Some(w(1)), Layout::Horizontal);
        ws.insert(w(3), Some(w(2)), Layout::Horizontal);
        ws.tree_mut().resize(w(1), Direction::Right, 0.1666);

        let session = Session::capture(&ws, app_of);
        let mut out = Workspaces::new();
        session.restore_into(&mut out, |app| {
            window_named(&session.apps[app]).filter(|id| *id != w(3))
        });
        let rects = out.tree_on(1).unwrap().layout(AREA);
        assert_eq!(rects.len(), 2);
        assert_eq!(rects.iter().map(|(_, r)| r.w).sum::<i32>(), AREA.w);
        assert_eq!(
            rects.iter().map(|(_, r)| r.w).collect::<Vec<_>>(),
            vec![750, 250]
        );
    }

    #[test]
    fn a_window_the_session_never_heard_of_keeps_its_workspace() {
        // It opened while the restore was still filling in. Dropping it would
        // make a window the user is looking at disappear.
        let session = Session::capture(&live(), app_of);
        let mut out = Workspaces::new();
        out.switch_to(7);
        out.insert(w(9), None, Layout::Horizontal);
        session.restore_into(&mut out, |app| window_named(&session.apps[app]));
        assert_eq!(out.tree_on(7).unwrap().windows(), vec![w(9)]);
        assert_eq!(out.tree_on(1).unwrap().windows(), vec![w(1), w(2)]);
    }

    #[test]
    fn restoring_twice_lands_in_the_same_place() {
        // Each client that turns up triggers another restore, so this runs once
        // per window; the second pass must not shuffle what the first placed.
        let session = Session::capture(&live(), app_of);
        let mut out = Workspaces::new();
        session.restore_into(&mut out, |app| window_named(&session.apps[app]));
        let once = layouts(&out);
        session.restore_into(&mut out, |app| window_named(&session.apps[app]));
        assert_eq!(layouts(&out), once);
    }

    #[test]
    fn a_restore_leaves_the_workspace_on_screen_alone() {
        // It runs again on every arriving client; changing the active workspace
        // each time would yank the screen away while the session starts.
        let session = Session::capture(&live(), app_of);
        assert_eq!(session.active, 4);
        let mut out = Workspaces::new();
        out.switch_to(2);
        session.restore_into(&mut out, |app| window_named(&session.apps[app]));
        assert_eq!(out.active(), 2);
    }

    #[test]
    fn a_window_with_no_command_is_saved_but_not_restored() {
        // The honest half of the design: a client the compositor did not launch
        // is recorded, so the file says what was there, and dropped on the way
        // back, because there is nothing to launch.
        let mut ws = Workspaces::new();
        ws.insert(w(1), None, Layout::Horizontal);
        ws.insert(w(2), Some(w(1)), Layout::Horizontal);
        let session = Session::capture(&ws, |id| App {
            command: (id == w(1)).then(|| "foot".to_string()),
            title: format!("window {}", id.0),
            pane: false,
            pane_path: None,
        });
        assert_eq!(session.apps[1].command, None);
        assert_eq!(session.apps[1].title, "window 2");

        let decoded = Session::decode(&session.encode()).expect("decodable");
        assert_eq!(decoded, session, "a commandless app survives the file");
        let mut out = Workspaces::new();
        decoded.restore_into(&mut out, |app| {
            decoded.apps[app]
                .command
                .as_ref()
                .and_then(|_| window_named(&decoded.apps[app]))
        });
        assert_eq!(out.tree_on(1).unwrap().layout(AREA), vec![(w(1), AREA)]);
    }

    // ---- the file format -------------------------------------------------

    #[test]
    fn a_file_that_is_not_a_session_is_refused() {
        assert_eq!(Session::decode(""), None);
        assert_eq!(Session::decode("hello\nworld\n"), None);
        assert_eq!(
            Session::decode("ruster-workspaces 99\nactive 1\n"),
            None,
            "version"
        );
        assert_eq!(Session::decode("ruster-workspaces\n"), None, "no version");
        assert_eq!(
            Session::decode("ruster-workspaces 1\n"),
            None,
            "no active workspace"
        );
        assert_eq!(
            Session::decode("ruster-workspaces 1\nactive 0\n"),
            None,
            "there is no workspace 0"
        );
        assert_eq!(
            Session::decode("ruster-workspaces 1\nactive 10\n"),
            None,
            "nor a tenth"
        );
    }

    #[test]
    fn a_corrupt_layout_restores_nothing_rather_than_half_of_one() {
        let good = "ruster-workspaces 1\nactive 1\napp foot\napp foot\nworkspace 1\nsplit h 2 0.5000 0.5000\nleaf 0\nleaf 1\n";
        assert!(Session::decode(good).is_some(), "the shape under test");

        for (why, text) in [
            (
                "a split promising two children and supplying one",
                "ruster-workspaces 1\nactive 1\napp foot\nworkspace 1\nsplit h 2 0.5000 0.5000\nleaf 0\n",
            ),
            (
                "a leaf pointing past the app table",
                "ruster-workspaces 1\nactive 1\napp foot\nworkspace 1\nleaf 7\n",
            ),
            (
                "two leaves claiming the same app",
                "ruster-workspaces 1\nactive 1\napp foot\napp foot\nworkspace 1\nsplit h 2 0.5000 0.5000\nleaf 0\nleaf 0\n",
            ),
            (
                "an app nothing places",
                "ruster-workspaces 1\nactive 1\napp foot\napp foot\nworkspace 1\nleaf 0\n",
            ),
            (
                "ratios that do not add up",
                "ruster-workspaces 1\nactive 1\napp foot\napp foot\nworkspace 1\nsplit h 2 0.5000 0.9000\nleaf 0\nleaf 1\n",
            ),
            (
                "one ratio short",
                "ruster-workspaces 1\nactive 1\napp foot\napp foot\nworkspace 1\nsplit h 2 1.0000\nleaf 0\nleaf 1\n",
            ),
            (
                "a split of one",
                "ruster-workspaces 1\nactive 1\napp foot\nworkspace 1\nsplit h 1 1.0000\nleaf 0\n",
            ),
            (
                "an unknown node keyword",
                "ruster-workspaces 1\nactive 1\napp foot\nworkspace 1\nfrob 0\n",
            ),
            (
                "junk after a complete tree",
                "ruster-workspaces 1\nactive 1\napp foot\nworkspace 1\nleaf 0\nwat\n",
            ),
            (
                "a title with no app in front of it",
                "ruster-workspaces 1\nactive 1\ntitle stray\napp foot\nworkspace 1\nleaf 0\n",
            ),
            (
                "a workspace that does not exist",
                "ruster-workspaces 1\nactive 1\napp foot\nworkspace 12\nleaf 0\n",
            ),
            (
                "the same workspace twice",
                "ruster-workspaces 1\nactive 1\napp foot\napp foot\nworkspace 1\nleaf 0\nworkspace 1\nleaf 1\n",
            ),
            (
                "a float with no area",
                "ruster-workspaces 1\nactive 1\napp foot\nworkspace 1\nfloat 0 10 10 0 100\n",
            ),
            (
                "a float missing a coordinate",
                "ruster-workspaces 1\nactive 1\napp foot\nworkspace 1\nfloat 0 10 10 100\n",
            ),
        ] {
            assert_eq!(Session::decode(text), None, "{why}");
        }
    }

    #[test]
    fn preorder_with_a_child_count_is_unambiguous() {
        // Three leaves in a row and the same three nested one deep have the same
        // leaves in the same order; only the split lines tell them apart.
        let flat = NodeSnapshot::Split {
            layout: Layout::Horizontal,
            ratios: vec![1.0 / 3.0; 3],
            children: vec![
                NodeSnapshot::Leaf(0),
                NodeSnapshot::Leaf(1),
                NodeSnapshot::Leaf(2),
            ],
        };
        let nested = NodeSnapshot::Split {
            layout: Layout::Horizontal,
            ratios: vec![0.5, 0.5],
            children: vec![
                NodeSnapshot::Leaf(0),
                NodeSnapshot::Split {
                    layout: Layout::Horizontal,
                    ratios: vec![0.5, 0.5],
                    children: vec![NodeSnapshot::Leaf(1), NodeSnapshot::Leaf(2)],
                },
            ],
        };
        let session = |tiled| {
            let mut workspaces: [WorkspaceSnapshot; SLOTS] = Default::default();
            workspaces[0].tiled = Some(tiled);
            Session {
                apps: vec![App::default(), App::default(), App::default()],
                workspaces,
                active: 1,
            }
        };
        let a = session(flat);
        let b = session(nested);
        assert_ne!(a.encode(), b.encode());
        assert_eq!(Session::decode(&a.encode()), Some(a));
        assert_eq!(Session::decode(&b.encode()), Some(b));
    }

    #[test]
    fn commands_and_titles_keep_their_spaces() {
        let session = Session {
            apps: vec![App {
                command: Some("sh -c 'foot -e htop'".into()),
                title: "htop — 3 running".into(),
                pane: false,
                pane_path: None,
            }],
            workspaces: {
                let mut ws: [WorkspaceSnapshot; SLOTS] = Default::default();
                ws[8].tiled = Some(NodeSnapshot::Leaf(0));
                ws
            },
            active: 9,
        };
        assert_eq!(Session::decode(&session.encode()), Some(session));
    }

    #[test]
    fn a_title_with_a_newline_in_it_cannot_corrupt_the_file() {
        // Clients pick their own titles, and nothing stops one containing a
        // newline — which would otherwise write a line the parser reads as a
        // keyword and throw the whole session away at the next boot.
        let mut workspaces: [WorkspaceSnapshot; SLOTS] = Default::default();
        workspaces[0].tiled = Some(NodeSnapshot::Leaf(0));
        let session = Session {
            apps: vec![App {
                command: Some("foot".into()),
                title: "evil\nworkspace 2\nleaf 0".into(),
                pane: false,
                pane_path: None,
            }],
            workspaces,
            active: 1,
        };
        let decoded = Session::decode(&session.encode()).expect("still a valid file");
        assert_eq!(decoded.apps[0].command.as_deref(), Some("foot"));
        assert_eq!(decoded.apps[0].title, "evilworkspace 2leaf 0");
        assert_eq!(decoded.workspaces[1], WorkspaceSnapshot::default());
    }

    #[test]
    fn each_seat_gets_its_own_file_and_the_name_is_stable() {
        let dir = Path::new("/state");
        let a = session_path(dir, "seat0");
        assert_ne!(a, session_path(dir, "seat1"), "seats do not collide");
        assert_eq!(a, session_path(dir, "seat0"), "and the name is stable");
        assert!(a.to_string_lossy().ends_with(".workspaces"));
        assert!(a.starts_with("/state/workspaces"));
    }

    #[test]
    fn saving_then_loading_returns_the_same_session() {
        let dir = std::env::temp_dir().join("ruster_workspaces_rt");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(load(&dir, "seat0"), None, "nothing saved yet");
        let session = Session::capture(&live(), app_of);
        save(&dir, "seat0", &session).unwrap();
        assert_eq!(load(&dir, "seat0"), Some(session));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
