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

### Task 1: Git gutter signs

- [ ] `app.rs:5384-86` — added / modified / removed sign colours.
- [ ] Groups under a `signs` pseudo-language: `added`, `modified`, `removed`.

### Task 2: Diagnostic and debug signs

- [ ] `app.rs:3733` breakpoint, `app.rs:7313` test failure, `app.rs:389` the
      TODO keyword colour.
- [ ] Extend `signs` with `breakpoint`, `error`, `todo` rather than inventing a
      third group.

### Task 3: Dired entry types

- [ ] `dired.rs:429-33` — directories blue, executables green, symlinks teal.
- [ ] A `dired` pseudo-language with `directory`, `executable`, `symlink`.

### Task 4: Flash-jump labels

- [ ] `app.rs:3759-61` — the two label colours.
- [ ] Group them with the signs or give them their own; decide when doing it,
      not now.

### Task 5: The TUI-only toast background

- [ ] `renderer.rs:152,166` paint the noice toast `Rgb(30, 30, 50)`
      unconditionally, **in the TUI only**. The GUI themes it. So a themed GUI
      and an unthemed TUI disagree, which is a parity break rather than a
      missing setting.
- [ ] Use the theme's `cmdline_bg`/`whichkey_bg` as the GUI already does.

**Tests, per task:** an override reaches the rendered line, and the groups are
visually distinct from each other. Both patterns exist in
`git_status::tests::a_syntax_override_recolours_the_diff`.

---

## Stage 2 — Small things that are simply missing

### Task 6: `:16` — goto line

- [ ] The cmdline does not accept a bare line number. Vim users type it
      constantly; I typed it myself mid-session, watched nothing happen, and
      misread the resulting no-op as a different bug.
- [ ] `:16` moves the cursor to line 16 and scrolls it into view.

### Task 7: `:hover`

- [ ] LSP hover has no `:` command — only `K` and `SPC c k`.
- [ ] Worth having on its own, and it is the only way to put a **float** on
      screen deliberately. Floats are currently unreachable except through
      `K` on a live LSP server, which is why the float border has never been
      verified in a screenshot.

### Task 8: Two stale things in the render path

- [ ] `App::floats` is written nowhere. It is cloned into `FrameState` every
      frame and only ever gains the hover popup, pushed separately. Either give
      it a writer or delete the field.
- [ ] `ruster-render-raylib` comments that "a modal dialog sits above the
      floats", but the float loop runs *after* the dialog. Inert today because
      the only float is the hover popup, which never coexists with a dialog —
      but the comment and the code disagree, and one of them is wrong.

---

## Stage 3 — Performance, where it is actually spent

### Task 9: Incremental parsing

The single largest win available, and currently switched off.

`SyntaxEngine::reparse` runs from `render`, so **every frame**:

```rust
let mut parser = tree_sitter::Parser::new();   // fresh parser, every frame
parser.parse(text, None)                        // full reparse — no old tree
```

- [ ] Reuse one `Parser` rather than allocating per frame.
- [ ] Track edits as `InputEdit` and pass `Some(&old_tree)`, which is the whole
      point of tree-sitter and is what `None` here discards.
- [ ] `buffer.to_string()` also materialises the entire rope each frame to feed
      it; consider a chunk-based callback.
- [ ] Measure first and after. **Do not** reach for threads before this: the
      work being parallelised should not exist.

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

### Task 11: More events

Lua can hook exactly four: `VimEnter`, `ModeChanged`, `BufWritePre`,
`BufWritePost`. Neovim has around sixty.

- [ ] `BufEnter` / `BufLeave` — the most-used autocmd in any editor.
- [ ] `CursorMoved` — debounced, or it fires per keypress and every plugin
      using it becomes a performance problem.
- [ ] `InsertEnter` / `InsertLeave`, `WinEnter`, `FileType`, `VimLeave`.
- [ ] Tests: each fires once, with the right arguments, and firing into a
      handler that errors does not take the editor down.

### Task 12: A timer

- [ ] `ruster.defer(ms, fn)` and a cancellable `ruster.timer`.
- [ ] Without it there is no debounce, no polling, no deferred work — the
      absence is why driving the GUI for screenshots needed a whole
      `init.lua`-and-`:screenshot` dance rather than "wait, then capture".
- [ ] Must run on the frame drain like every other Lua action, not a thread:
      the runtime is not `Send`, and that is deliberate.

### Task 13: Read-only introspection

- [ ] Lua cannot ask for diagnostics, git status, or the current file's path.
- [ ] Add the queries a statusline or a lightweight plugin actually needs, and
      no more. Every getter is API surface that has to keep working.

---

## Stage 6 — The application icon

ruster has no icon anywhere: no `.icns`, no `.ico`, no `.desktop`, and the
raylib window shows the default. It is the first thing anyone sees and the last
thing anyone adds.

GUI-only, so no TUI parity question arises.

### Task 14: The artwork

- [ ] Decide the mark. This wants a designer rather than a generated
      placeholder — an obviously-programmatic icon is worse than none, because
      it looks finished. If a placeholder ships first, make it visibly a
      placeholder and track replacing it.
- [ ] Source: one square master (1024×1024) that everything else derives from.

### Task 15: Wire it up per platform

- [ ] **Runtime window icon** — `RaylibHandle::set_window_icon`, so the running
      window and the dock/taskbar entry stop showing the default. Cheapest win
      and works on all three platforms; do this one first.
- [ ] **macOS** — an `.app` bundle with `Contents/Resources/*.icns`. Without a
      bundle there is no Dock icon however the window is configured, so this is
      packaging work, not icon work.
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
