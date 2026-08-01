# Phase 7 — The "Emacs Extras" (Application Platform)

**Status:** planning, 2026-08-02. Phase 6 is 16/17 complete; only Task 4 (look at
the raylib GUI) is outstanding, blocked on a locked screen rather than on code.

`AGENTS.md` lists six features for this phase. They are not comparable in value,
risk, or fit, so this plan sequences them by that rather than by the order they
happen to appear in the table — and says plainly which two should probably not
be built at all.

## Global constraints (unchanged from Phase 6)

- **GUI/TUI parity.** `ruster-render-raylib` does not depend on ratatui. Any
  feature that only reaches one backend is a regression, and this is what rules
  out the embedded browser below.
- **Non-blocking.** Background thread → `mpsc` → per-frame drain. No tokio in
  the editor loop.
- **Docs as each task lands.** Enforced now by
  `crates/ruster-tui/tests/docs_in_sync.rs`, which fails CI naming any command
  absent from `docs/keybindings.md`.
- **`App` is already the bottleneck.** The 2026-08-01 graphify run measured its
  betweenness at 0.51, driven by its 53 cross-crate field references — not by
  its method count. Every feature here adds at least one more subsystem field,
  so each should own its state in its own module from the start, the way
  `sidebar`/`dired`/`trouble`/`dialog` do, and expose one field to `App`.

---

## Stage 1 — The two that are clearly worth it

### Task 1: Session management ✅

Restore what you had open. The highest value-to-risk ratio in the phase: no new
dependency, no network, no platform code, and every piece is serialisable state
ruster already holds.

- [x] Persist per project root: open buffers (paths only), the window tree
      (`Layout` + which buffer each leaf shows), the active window, per-window
      cursor offset and `scroll_top`.
- [x] Write to `~/.config/ruster/sessions/<hash-of-root>.json` on exit and on
      `:SessionSave`; restore on `:SessionRestore`, and optionally on startup
      behind a `session.autoload` setting (default **off** — silently reopening
      files is surprising).
- [x] Skip what must not be restored: special buffers (dired, mason, diff,
      terminals, `*messages*`), unsaved scratch buffers, and any path that no
      longer exists. A session that fails to restore cleanly must open an empty
      editor with a warning, never refuse to start.
- [x] Terminal history is listed in `AGENTS.md` but is **out of scope**:
      restoring a PTY's scrollback means replaying output into a shell that no
      longer exists. Record the decision rather than half-doing it.
- [x] Tests: round-trip a nested split layout; a missing file is dropped, not
      fatal; a corrupt session file degrades to an empty session; special
      buffers are excluded.

**Why first:** `Layout` is a private binary tree in `ruster-core::windows`, so
this needs a deliberate serialisable representation rather than `#[derive]` on
internals — worth doing carefully once, since Task 2 and any future workspace
feature will want the same thing.

### Task 2: Help menu (`:help`) ✅

- [x] `:help` opens a searchable buffer; `:help <topic>` jumps to a section.
- [x] Source it from what already exists rather than a second copy of the truth:
      `docs/keybindings.md` and `docs/config-reference.md` are already complete
      and now CI-enforced. Embed them with `include_str!` and render as markup —
      `ruster-syntax` already highlights markdown.
- [x] `:help <command>` resolves a `:` command to its row in the table; `:help
      <setting>` to its row in the config reference.
- [x] Tests: topic resolution, including a topic that does not exist.

**Why second:** it turns the doc-sync guard into a user-facing feature. The docs
are already the single source of truth; this reads them.

---

## Stage 2 — Worth it, but a real project

### Task 3: Magit-style git porcelain ✅ — **planned separately**

Large enough to want its own document:
[2026-08-02-phase7-task3-magit.md](2026-08-02-phase7-task3-magit.md), which
carries the captured `porcelain=v2` fixtures, the staging design, and the
sequencing into four PRs. **Delivered across PRs #45–#48 plus #49**, every box
in that plan ticked, including hunk unstaging via the `:GitStaged` view.

`AGENTS.md` says "in Lua on top of `git2-rs`". Both halves deserve challenge:

- **Not Lua.** The Lua API is for user plugins; a core surface written through it
  would be slower, harder to test, and unable to use the existing widget layer.
  Write it in Rust in `ruster-git`, and expose hooks to Lua afterwards.
- **Probably not `git2-rs`.** `ruster-git` already shells out to `git` and that
  has been fine — no libgit2 build dependency, no version skew, and identical
  behaviour to the user's own CLI. Adding `git2-rs` buys speed we do not need at
  the cost of a C dependency on three platforms. Revisit only if shelling out
  proves too slow for the status view.

- [x] `:Git` status buffer: staged / unstaged / untracked, foldable by file.
- [x] Stage/unstage hunks (`s`/`u`), reusing the Task 8 hunk parser and the
      Task 14 `DiffHunk` two-sided coordinates.
- [x] Commit (`c`) opening a message buffer; push/pull behind confirmation.
- [x] Tests: status parsing from captured `git status --porcelain=v2` output —
      no test may require a real repository, matching `ruster-git`'s existing rule.

### Task 4: Music player

- [ ] `:Music` — control an already-running `mpd` over its plain-text protocol
      on `localhost:6600`. No bundled player, no audio decoding in the editor.
- [ ] Degrade silently when mpd is absent: this is a convenience, and an editor
      that complains about a missing music daemon on startup is broken.
- [ ] Tests: protocol response parsing from captured output.

**Honest note:** this is the least defensible feature in the phase. It is cheap
because mpd's protocol is trivial, but nobody chooses an editor for it. Build it
last, or not at all.

---

## Stage 3 — Recommend against, with reasons

### Task 5: Embedded web browser — **do not build**

`AGENTS.md` proposes `webkit2gtk` (Linux/macOS) / `WebView2` (Windows). This
conflicts directly with the parity constraint: a native webview is an OS window
composited by the platform, and there is no way to draw one inside a raylib
frame or a terminal cell grid. It would be a GUI-only feature on one backend,
which is the exact regression the parity rule exists to prevent — and it adds a
browser engine's attack surface and build burden to a text editor.

**Instead**, if the underlying want is "read a URL without leaving the editor":

- [ ] `:Browse <url>` fetching over HTTP and rendering as markup in a buffer,
      reusing the markdown path that already exists for `:help` and hover docs.
      Text-mode only, both backends, no engine.

### Task 6: Email client — **defer, and reconsider the scope**

Gmail over IMAP/SMTP means OAuth2, token storage, and a credential store, on top
of MIME parsing and HTML rendering. That is a mail client's worth of work and
risk — including storing someone's mail credentials — bolted to an editor, and
none of it shares code with anything else here.

If it is wanted, the defensible version is much smaller:

- [ ] Compose-only: open an editor buffer, hand the result to the system's
      configured MUA (`mailto:` / `sendmail`). No credentials, no IMAP, no
      inbox.

Full IMAP should be a plugin against the Lua API, not core.

---

## Suggested order

1. **Task 1 — Session management** (self-contained, no new deps, high value)
2. **Task 2 — Help menu** (reads docs that are already CI-enforced)
3. **Task 3 — Magit** (the real project; needs its own plan doc)
4. **Task 4 — Music player** (cheap, low value — last, or never)
5. **Task 5 — `:Browse`** only in the text-mode form above
6. **Task 6 — Email** only in the compose-only form, or as a plugin

## Out of scope, deliberately

- **Terminal scrollback in sessions** (Task 1) — replaying PTY output into a
  dead shell is not restoration.
- **`git2-rs`** (Task 3) unless shelling out measurably fails.
- **A bundled browser engine or IMAP stack** (Tasks 5, 6) — see above.
- **Extracting terminal and picker from `App`** — still deferred from Phase 6,
  and still the right call: the graphify run showed method extraction moves
  betweenness by ~4%, so it is not the lever it looks like.
