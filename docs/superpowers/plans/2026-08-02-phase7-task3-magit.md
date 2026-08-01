# Phase 7 Task 3 — Magit-style git porcelain

**Status:** planning, 2026-08-02. Phase 7 Tasks 1–2 are merged; this is the one
real project left in the phase and is large enough to want its own plan.

## Two departures from the `AGENTS.md` line

`AGENTS.md` says *"a full-featured Git porcelain UI built in Lua on top of
`git2-rs`"*. Both halves are wrong for this codebase, and the reasons are worth
recording so they are not re-litigated.

**Not in Lua.** The Lua API exists so *users* can extend the editor. A core
surface written through it would be slower (every status refresh crossing the
mlua boundary), harder to test (no `cargo test` over Lua behaviour), and unable
to use the widget layer that `trouble`/`dialog`/`mason` already share. It would
also mean the Lua runtime became load-bearing for a core feature — and PR #36
exists because that runtime had a soundness bug. Write it in Rust; expose hooks
to Lua afterwards.

**Probably not `git2-rs`.** `ruster-git` already shells out and it has been fine
for gutter signs, `:Diffview` and hunk navigation. Shelling out means: no
libgit2 C dependency on three CI platforms, no version skew between the linked
library and the user's `git`, and behaviour identical to what the user gets in
their own terminal — which for a porcelain is the *point*. `git2-rs` buys speed
this does not need. Revisit only if a status refresh measurably stalls a frame,
and note that the fix then is a background thread (the pattern already used for
LSP, DAP, the runner and git hunks), not a new dependency.

## What already exists

`ruster-git` gives us, all shell-outs, all with pure parsers:

| Have | Use here |
| --- | --- |
| `parse_diff_hunks` → `DiffHunk` | the two-sided coordinates a stage-hunk patch needs |
| `align`, `file_at_head` | already powering `:Diffview` |
| `diff_hunks`, `next_hunk`/`prev_hunk` | gutter signs and `]h`/`[h` |
| `is_repo` | the guard every command needs |

So Task 3 adds **status parsing** and **patch construction**, and reuses the
rest.

---

## Stage 1 — Read-only status

### 1.1 Parse `git status --porcelain=v2 --branch`

Captured from a real repository, and these are the shapes the parser must take:

```text
# branch.oid 76fd70c99b302a57e5c24987709af4b683e9c72e
# branch.head main
1 A. N... 000000 100644 100644 0000000 3e75765 staged.txt
1 MM N... 100644 100644 100644 814f4a4 05b65e8 tracked.txt
2 R. N... 100644 100644 100644 7b26523 7b26523 R100 moved.txt→tracked.txt
? untracked.txt
```

- [ ] `1` = ordinary change, `2` = rename/copy, `?` = untracked, `u` = unmerged,
      `#` = header.
- [ ] The `XY` field is the whole point: **X is the staged status, Y the
      unstaged one**. `MM` above is one file appearing in *both* lists — get
      this wrong and the two sections lie.
- [ ] A `2` line's paths are `new<TAB>old`, **tab-separated**, not space. A
      space-splitting parser silently mangles every rename.
- [ ] Tests over captured output only — no test may require a real repository,
      matching `ruster-git`'s existing rule. The samples above go in as fixtures.

### 1.2 The `:Git` status buffer

- [ ] A special buffer like `:Trouble` and `:Mason`: `SpecialKind::Git`, sections
      for **Staged**, **Unstaged**, **Untracked**, foldable per file with `za`.
- [ ] Branch and upstream in the header, from the `# branch.*` lines.
- [ ] `Enter` opens the file; `Tab`/`za` folds; `g`/`r` refreshes; `q` closes.
- [ ] Reuse `trouble.rs`'s grouping and row-resolution shape — it already solves
      "screen row → item" for a foldable, sectioned list, and duplicating it
      would be the third copy of that logic.

**Ship 1.1 + 1.2 as one PR.** A read-only status view is useful on its own and
carries none of the risk below.

---

## Stage 2 — Staging, and the part that is actually hard

### 2.1 Stage and unstage whole files

- [ ] `s` → `git add -- <path>`, `u` → `git restore --staged -- <path>`.
      Straightforward, and worth landing before hunks.

### 2.2 Stage and unstage *hunks*

This is the whole difficulty of the task and should be planned before it is
started.

Staging a hunk means feeding `git apply --cached` a patch containing **only that
hunk**, which must be reconstructed — not sliced out of the original diff text,
because the header line counts have to be recomputed for a patch that contains
one hunk instead of several.

- [ ] Generate with **context** (`git diff -U3`), not the `-U0` the gutter uses.
      `git apply` needs context lines to locate the hunk; a `-U0` patch is
      rejected or misapplies.
- [ ] Rebuild the `@@ -a,b +c,d @@` header for the single hunk. `DiffHunk`
      already carries both sides, which is why Task 14's two-sided coordinates
      were worth keeping.
- [ ] Unstaging is the same patch applied with `--cached --reverse`.
- [ ] **Never write to the working tree.** `--cached` only touches the index, so
      a bug loses staging, not the user's edits. Discarding changes (a
      `git checkout --` equivalent) is genuinely destructive and belongs behind
      the confirmation dialog, if it is offered at all.
- [ ] Tests: build a patch from captured diff output and assert the exact bytes,
      including the recomputed header — then a round-trip test that
      apply-then-reverse is identity. No repository required.

---

## Stage 3 — Writing operations

Every one of these changes history or the remote, so each is gated.

- [ ] `c` → commit: open a message buffer, `:w` commits. An empty message
      aborts.
- [ ] `P` → push, `F` → pull: **behind the confirmation dialog**, showing the
      exact command, the pattern `:Mason` established. Stream output through
      `RunnerKind` into a results buffer like an install.
- [ ] No force-push, no history rewriting, no `reset --hard` from the UI. A
      porcelain that can silently destroy work is worse than no porcelain; the
      user still has a terminal for those.

---

## Risks, named

- **The `XY` split.** The single most likely correctness bug, and invisible
  until someone stages half a file. Fixture tests for `MM`, `A.`, `.M`, `R.`.
- **Patch reconstruction.** Fiddly, and a wrong patch either fails loudly
  (fine) or applies to the wrong lines (not fine). `--cached` bounds the damage
  to the index.
- **Blocking the frame.** `git status` on a large repository is not instant.
  Start synchronous, measure, and move to the existing background-thread pattern
  if a refresh is visible — do not pre-emptively thread it.
- **Scope.** "Full-featured Magit" is years of work. This plan deliberately
  stops at status, staging, and commit/push/pull. Log browsing, rebasing,
  cherry-picking, stashing and blame are **out of scope**; revisit individually
  once the base exists.

## Suggested sequencing

1. **PR 1** — status parsing + read-only `:Git` buffer (Stage 1). Useful alone.
2. **PR 2** — whole-file staging (2.1).
3. **PR 3** — hunk staging (2.2), the risky one, on its own so it can be
   reviewed and reverted independently.
4. **PR 4** — commit, then push/pull behind confirmation (Stage 3).

## Out of scope, deliberately

- `git2-rs`, unless shelling out measurably fails.
- Writing any of this in Lua.
- Log/rebase/cherry-pick/stash/blame — a second plan, if wanted.
- Force-push and any history rewriting from the UI, permanently.
