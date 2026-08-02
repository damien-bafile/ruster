# Phase 8 — Finetuning

**Status:** planning, 2026-08-02.

Phases 0–7 built features. This one pays down what building them left behind:
things that work but are wrong in a way a user can see, or that are quietly
getting worse.

Nothing here is a new capability. Every item is something already shipped that
should be better, and every one was found by *using* the editor rather than by
reading it — which is why they were missed at the time.

## Ordering principle

Do the items a user can see before the ones only a maintainer can. A theme that
cannot recolour the git gutter is a visible defect; `App` having 189 methods is
not, however much it matters later.

---

## Stage 1 — Theming: colours no theme can reach

Eight sites draw fixed RGB values, so the active theme and
`ruster.config.colors.*` cannot touch them. Audited 2026-08-02; the rest of the
hardcoded colours in the tree are legitimate (`Theme::default()` itself, syntax
group defaults, ANSI conversion in `ruster-terminal`, colour maths for toast
fades, and `c(|t| t.field, fallback)` patterns where the theme wins).

**The approach is settled.** `:GitStaged` already solved this: `diff` became a
*pseudo-language* in `ruster-syntax`, so it inherits the per-language override
machinery, appears in the Settings syntax editor, and honours
`ruster.config.syntax.diff.*` — without a second theming system. Each item below
follows that pattern.

### Tasks 1-5 ✅

Three new pseudo-languages beside `diff`, and one parity fix.

- [x] **`signs`** — the whole gutter in one group, because the glyphs share a
      column and a theme wants to pick them together: `added`, `modified`,
      `removed` (git hunks), `breakpoint`, `error`, `warning`, `info`, `hint`
      (diagnostics, with `error` doubling as the failing-test sign), and `todo`.
- [x] **`dired`** — `directory`, `executable`, `symlink`.
- [x] **`flash`** — `label` and `pending`, their own group rather than part of
      `signs`: they are transient overlays on the text, not gutter glyphs, and
      a theme wants them loud in a way it never wants a sign column to be.
- [x] The TUI toast background. The GUI paints it `theme.whichkey_bg`; the TUI
      hardcoded `Rgb(30, 30, 50)` against a default of `Rgb(30, 30, 46)`, so the
      two backends disagreed *before* anyone changed a theme. Same source now.

**The audit missed one, and it was the most visible.** `severity_sign` in
`app.rs` hardcoded all four diagnostic severity colours — the E/W/I/H glyphs in
the gutter, which are on screen far more often than a breakpoint. The plan
listed the failing-test sign but not these. Found by scraping for literals
rather than by re-reading the list, which is the argument for the guard below.

**A footgun removed on the way.** `diff_style` read the override map through a
thread-local that the caller had to set with `set_current_lang("diff")` first —
correct only by convention, and the convention was one call site deep. The
accessors now name their language at the lookup. The thread-local stays for the
highlight pass, which sets it once and resolves thousands of captures.

**Tests:** `ruster-syntax/tests/pseudo_languages.rs` (6) and
`ruster-tui/tests/colors_are_themeable.rs` (6).

The first pair are the interesting ones: every group the Settings editor lists
must resolve to a real colour, *and* the style function the drawing code calls
must know every group the editor offers. Either direction failing means a knob
that exists and does nothing. Mutation-tested — adding a group to the Settings
list without a match arm, making a style function ignore overrides, and keying
overrides globally instead of per language are each caught by two tests.

The second is a source scrape, in the shape of `commands_discoverable.rs`: a
colour literal at a draw site fails the build unless the file is allow-listed
with a reason. Most literals in the tree are legitimate — the `unwrap_or` arm of
`c(fallback, |t| t.field)`, where the theme wins whenever there is one — so the
list separates those from real hardcoding.

That guard was **vacuous when first written**, and mutation testing is the only
reason that was found. It truncated each file at the first `#[cfg(test)]` to
skip test modules; `dired.rs` has a test-only accessor at line 156 and its test
module at 470, so the scrape never saw the 300 lines in between — including
every colour it was written to watch. It passed with a hardcoded colour put back
by hand. It now looks for the test *module* specifically, and a test pins that.

**Verified in the editor, not only by test:** with
`dired = { directory = "#ff0000", executable = "#00ff00" }` a listing renders
`255;0;0` and `0;255;0`; with `signs = { added = "#ff00ff", modified = "#00ffff" }`
the gutter renders `+` magenta and `~` cyan.

---

## Stage 2 — Small things that are simply missing

### Task 6: `:16` — goto line ✅

- [x] `:16` jumps and scrolls into view; `:$` goes to the last line. Clamped
      rather than rejected — `:9999` in a short file goes to the end, which is
      what vim does and what the typist meant.
- [x] The parse arm is *all digits*, not *starts with a digit*, or `:16x` and
      `:2vsplit` would become jumps. Tested both ways.
- [x] No explicit scroll: `render` already pulls the window to the cursor, the
      same path `G` and a quickfix jump take.

### Task 7: `:hover` ✅

- [x] Added, dispatching to the same `lsp_hover` as `K` and `SPC c k`.
- [x] **Verified against a live rust-analyzer**, which is the first time a float
      has been seen on screen with its border: hovering `add` renders the
      signature and its doc comment inside the box.
- [ ] Still unverified in the **GUI**. The screenshot fires a couple of frames
      in, long before the LSP replies, and `init.lua` has no way to delay it —
      every queued command is applied before the first render. Stage 5's
      `ruster.defer` is what unblocks this; capture it then.

### Task 8: Two stale things in the render path ✅

- [x] `App::floats` deleted. Nothing ever wrote to it; the hover popup was
      always pushed to a *local* vector in `render`, so the field was an empty
      `Vec` cloned into every `FrameState`. There is no Lua float API to give it
      a writer for, so deleting was the honest option.
- [x] The draw order now matches the comment rather than the other way round.
      **Both** backends drew floats after the dialog while both of their
      comments claimed the opposite — consistent with each other, so parity was
      never at risk, but code and comment disagreed and one had to be wrong. A
      modal is the surface with focus, so it draws last.
- [x] `tests/draw_order_parity.rs` pins it in both backends and, separately,
      pins that they *agree* — even a reversed intent would be a bug if only one
      followed it. Mutation-tested by putting the raylib order back.

---

### Task 8b: Which-key coverage ✅

Audited 2026-08-02: 41 of 65 `CmdAction` variants had no leader route. The tree
was built for Phase 5 and never grew, so the entire Phase 7 git porcelain —
status, commit, push, pull, staged diff, hunk staging, diffview — was reachable
only by typing, along with Mason, help, themes, sessions and the notification
panel.

- [x] A `SPC g` git group, `SPC S` for sessions, and the missing entries under
      `SPC o` and `SPC x`.
- [x] `tests/commands_discoverable.rs` — every command must be bound **or**
      declared typed-only *with a reason*, so adding one forces a deliberate
      choice instead of silent omission. The same shape as the docs guard.

---

## Stage 3 — Performance, where it is actually spent

### Task 9: Incremental parsing — **measured; the guard landed first**

The single largest win available, and currently switched off.

`SyntaxEngine::reparse` runs from `render`, so **every frame**:

```rust
let mut parser = tree_sitter::Parser::new();   // fresh parser, every frame
parser.parse(text, None)                        // full reparse — no old tree
```

**Measured 2026-08-02** on `crates/ruster-tui/src/app.rs` (10,294 lines), via
`cargo run --release -p ruster-syntax --example parse_bench`:

| | per call |
| --- | --- |
| `reparse` | **106 ms** |
| TODO overlay | **21 ms** |
| rope `to_string` | 0.03 ms |

Against a 16.7 ms frame budget that is roughly **7 fps** for a buffer nobody had
touched. The `to_string` worry in the original bullet was wrong — it is
negligible, and the plan said so before anyone measured.

- [x] **Skip the work entirely when nothing changed.** `Buffer` gains a
      revision counter, and `update_syntax` reparses only when it moves. This
      was not in the original plan and is worth more than everything else in
      this task: idle frames go from 127 ms to nothing.
**Where the time actually went** (same file, `examples/inc_bench` breaks
`reparse` into its stages):

| stage | before | after |
| --- | --- | --- |
| tree-sitter parse | 32 ms | 32 ms |
| bracket depths | 1 ms | 1 ms |
| **`highlight_lines`** | **75 ms** | **20 ms** |
| `reparse` total | 106 ms | **53 ms** |

The highlight pass, not the parse, was the dominant cost — and `byte_to_line`
was scanning `line_starts` linearly *per capture*, making the pass
O(captures x lines): hundreds of millions of comparisons on a 10k-line file.
Binary search plus an ASCII fast path in `byte_to_char_offset` took it from
75 ms to 20 ms. Proven equivalent to the linear version over every byte
position of a real 10k-line file, and pinned by a test that does the same over
edge-case inputs including unicode and empty text.

Two cheaper suspects were measured and rejected: a `String` allocation per
capture and an `RwLock` read per capture cost about 1 ms between them.

- [x] Reuse one `Parser` rather than allocating per frame.
- [x] Track edits as `InputEdit` and pass `Some(&old_tree)`. `Buffer` records
      each edit at the point of mutation — the byte offsets and points describe
      the buffer *as it was*, and afterwards that information is gone.
- [x] `buffer.to_string()` — measured at 0.03 ms. The original bullet's worry
      was wrong; left alone.
- [x] Measure first. **Do not** reach for threads before this: the work being
      parallelised should not exist.

**Done.** Where a keystroke in a 10k-line file stood at each step:

| | per keystroke |
| --- | --- |
| originally | 127 ms |
| after the highlight-pass fix | 74 ms |
| after incremental parsing | 43 ms |
| after fixing the TODO overlay | 21 ms |
| after bounding the highlight to the viewport | **2.7 ms** |

**Down from 127 ms to 2.7 ms — this task is done.** Roughly 47x, and comfortably
inside a 16.7 ms frame with room for everything else the frame has to do.

### Task 9b: Only highlight what is visible

- [x] `highlight_lines` built a `StyledLine` for every line in the file; the
      renderer reads roughly fifty.

      Split the 20 ms first, because the obvious suspect was wrong. The
      per-line build loop — a `String` per line, a character scan for rainbow
      brackets — is only **1.2 ms**. Running the query is **15.7 ms**, and the
      incremental tree-sitter parse underneath it is **0.30 ms**, exactly what
      it should be. So the query was the whole problem and cloning styled lines
      (0.70 ms, measured earlier) never was.

      `SyntaxEngine` now carries a viewport, set from the window's scroll in
      `render` — the only place the offset is settled — and bounds the query to
      it with `QueryCursor::set_byte_range`. **20.6 ms -> 2.7 ms per keystroke.**

      The cache is *not* partial: it stays one entry per line, with off-screen
      rows holding their text and no highlights. No caller handles a miss, and
      `styled_lines()` is unchanged. The API change the plan feared was
      avoidable.

      A 200-line margin either side means scrolling re-highlights only when it
      leaves the margin; the worst case a user can provoke, holding a movement
      key past it, is **1.5 ms**.
- [x] Bound the rainbow-bracket pass too. It walked every character of every
      line regardless, which both cost time and left off-screen rows
      half-styled — brackets coloured, nothing else. Found by a test asserting
      those rows were plain and getting colours back.
- [x] Keep the TODO panel honest. `todo_markers` reads comment ranges from the
      last highlight pass, so it now sees only the visible rows — correct for
      drawing the overlay, wrong for a panel listing a file's markers. Added
      `all_todo_markers`, a full-tree scan, and pointed the panel at it. Paid
      once per invocation, never per frame.
- [x] The TODO overlay — fixed differently and more cheaply than range-limiting.
      It was *recompiling the highlight query from source* and then re-running it
      over the whole tree, to find comments the highlight pass had just walked
      past. It now reads the ranges that pass recorded: **21.7 ms -> 0.26 ms**.
- [x] Tests: `tests/viewport_equivalence.rs`, 11 of them. A 20-line window
      walks the whole of a 1,200-line fixture and every covered row must equal
      the unbounded result, so the suite stays honest if the grammar changes.

      The load-bearing claim is that `set_byte_range` matches nodes which
      *overlap* the range rather than ones contained in it — otherwise a comment
      or string opening above the viewport would render as plain text. Verified
      against tree-sitter directly before relying on it, then guarded.

      Mutation-tested, which was necessary: deleting the bound entirely left
      nine of the eleven passing, because the off-screen skip in the build loop
      hides it. The test named for the bound was testing the output, not the
      query. Split into two, and now both die when the bound goes.

      One incidental finding, left alone as out of scope: the Rust highlight
      query captures nothing inside `raw_string_literal` bodies, so raw strings
      render unhighlighted in a full pass too. It cost a test premise before it
      was noticed.

---

## Stage 4 — The thing that is quietly getting worse

### Task 10: `App`

Measured, not asserted:

| | PR #24 | 2026-08-01 | 2026-08-02 |
| --- | --- | --- | --- |
| methods in `impl App` | 127 | 151 | **189** |
| `app.rs` non-test lines | 5,766 | 6,439 | **7,583** |

Phase 7 added roughly a third again on top of a file that was already the
largest thing in the tree.

**Do not chase betweenness.** The 2026-08-01 graphify run measured it at 0.51,
and simulating extractions showed moving all 151 methods into modules changed it
by −4.3%, while moving `App`'s cross-crate *field references* changed it by
−91%. That number measures being the composition root, which every application
has. Method count and line count are the metrics that track the real problem.

- [ ] Extract the LSP glue (14 methods) and the DAP glue (9) — both are already
      separate crates, so these are the least entangled.
- [ ] Extract the git surface, which Phase 7 added wholesale.
- [ ] Follow the `sidebar`/`dired`/`trouble` shape: state in its own module, one
      field on `App`, a thin adapter.

---

## Stage 5 — Lua: let plugins react, not just run

`ruster.cmd(":Whatever")` already makes **every `:` command a Lua API**, so the
command surface is not the limitation — adding a command extends the cmdline,
the keymap system and Lua at once. The limitation is that a plugin can be
*invoked* but can barely *react*, which makes plugins closer to startup scripts
than extensions.

The decision rule this implies, worth writing down:

- needs to **do** something → add a `:` command; Lua gets it free
- needs to **return** a value → `ruster.cmd` is fire-and-forget, so `ruster.api.*`
- needs to **react** to something → an event, which is the gap below

### Tasks 11-13 ✅

- [x] **Events**: `BufEnter`/`BufLeave`, `WinEnter`, `FileType`, `CursorMoved`,
      `InsertEnter`/`InsertLeave`, joining the four that existed.

      Fired by diffing watched state once per frame, not from each mutation
      site. Far too many places change the active buffer — every open, close,
      split, pick, jump and `:bd` — and an event firing from *most* of them is
      worse than one firing from all, because a plugin cannot tell what it
      missed.

      `CursorMoved` is debounced for free by that design: a held `j` moves the
      cursor many times between frames and fires once. The plan asked for a
      debounce and this needed no timer to get one.

      `BufLeave` names the buffer being *left*. It fires after the switch has
      happened, so the obvious version reports the new path for both and a
      handler saving per-file state writes it against the wrong file. Mutation
      testing showed nothing caught that; the test came second.

      The first pass records a baseline without firing, so a plugin loading into
      an editor with a buffer already open gets no BufEnter storm.

- [x] **Timers**: `ruster.defer`, `ruster.timer`, `ruster.timer_stop`. On the
      frame drain, not a thread. At most one firing per drain however far
      behind — a slow frame must not become a catch-up burst. Callbacks are
      resolved and the borrow dropped before any is invoked, so a callback can
      reschedule or cancel itself; holding the borrow across the call kills two
      tests.

- [x] **Queries**: `buf_path`, `filetype`, `diagnostics`, `git_status`. Served
      from a snapshot the frame loop refreshes, because the Lua closures are
      installed before `App` exists and cannot hold `&mut self`.

- [x] Verified in the running editor, with a caution about how. Reading the
      result through `:messages` gave empty paths and cost a long detour: the
      *messages buffer itself* is pathless and becomes active when opened, so
      every line I read had been recorded after the thing I was measuring
      changed. Writing to a file from the callback instead showed the API had
      been correct the whole time. The measurement was wrong, not the code.

- [x] `git_status()` is now kept current by a background refresh every two
      seconds, rather than staying empty until `:Git` was opened. Two seconds is
      a compromise, not a right answer: fast enough that the branch is not
      visibly stale after a commit, slow enough not to run `git` at frame rate
      on a large repository.

      An in-flight guard stops a slow repository accumulating processes — and
      the worker reports back **even when it finds nothing**, because a guard
      that only clears on success latches on the first failure and silently
      stops polling for the rest of the session. That was in the first version
      of this and would have looked exactly like the bug it replaced.

---

## Stage 6 — The application icon

ruster has no icon anywhere: no `.icns`, no `.ico`, no `.desktop`, and the
raylib window shows the default. It is the first thing anyone sees and the last
thing anyone adds.

GUI-only, so no TUI parity question arises.

### Task 14: The artwork ✅

- [x] Decide the mark. A phosphor-green prompt caret and an amber block cursor
      on CRT black — the palette `docs/config-reference.md` already defines for
      the Starship direction, rather than one invented for the icon. Checked at
      16px, where the two shapes stay distinct; an earlier draft sat too small
      in the canvas and merged into a blob.
- [x] Source: one square master, `assets/icon.png` at 1024×1024, with
      `just icon` deriving everything else.

**Still worth a designer.** This is a competent mark from the existing palette,
not an identity. Replacing it is one PNG and `just icon`.

### Task 15: Wire it up per platform

- [ ] **Runtime window icon** — `RaylibHandle::set_window_icon`, so the running
      window and the dock/taskbar entry stop showing the default. Cheapest win
      and works on all three platforms; do this one first.
- [x] **macOS** — `scripts/bundle-macos.sh` and `just bundle`. Verified: macOS
      reports `bundleID="dev.ruster.editor"`, names the app *ruster*, and the
      bundled binary opens a window. Ad-hoc codesigned, because Apple silicon
      refuses unsigned bundles outright. **No document types**: "Open With"
      needs Apple Event handling, since a bundled app receives files by `odoc`
      rather than argv, and declaring the types without it would advertise
      something broken.
- [ ] **Windows** — `.ico` embedded in the executable via a build script
      (`embed-resource` or `winres`).
- [ ] **Linux** — hicolor PNGs plus a `.desktop` entry.
- [ ] Tests: the asset is present and non-empty, and the build script runs on
      the platform that needs it. Nobody can test "looks right" — that is a
      look, and the `gui-check` skill is how to take it.

---

## Out of scope, deliberately

- **Threading the core.** `Rc<RefCell<Workspace>>` with 292 borrow sites; every
  one becomes a lock, turning a loud `BorrowMutError` into a silent deadlock.
  Revisit only if Task 9 fails to deliver.
- **Rewriting the theme system.** The pseudo-language route works and is already
  proven; five more groups do not justify a new mechanism.
- **New features of any kind.** That is what a finetuning phase is not — with
  the icon as the deliberate exception, since an application without one is
  unfinished rather than unfeatured.
- **A full autocmd system.** Task 11 adds the events plugins actually reach for,
  not Neovim's sixty. Add more when something needs them.
