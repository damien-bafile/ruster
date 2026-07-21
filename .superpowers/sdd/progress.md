# Plan A Progress Ledger

Repo: /Users/daimyo/Dev/ruster (main branch)
Plan: docs/superpowers/plans/2026-07-20-plan-a-core-engine.md

## Tasks

Task 1: complete (commits 510adf1..80a8d02, scaffold green, 0 tests)
Task 2: complete (commits 80a8d02..fffa16b, 3 buffer tests green). NOTE: plan Task 2 test assertions were swapped; fixed inline. Patch plan doc later.
Task 6: complete (commits 65e0da5..f2a0a45, 19 tests). Plan defect fixed inline (Edge::End lands past last char; Vim $ must subtract 1 — deferred to Task 7).
Task 7: complete (commits f2a0a45..f754dc8, 25 tests). Subagent corrected gg/G to Motion::To(line_start) and stroke-count zero handling. Plan doc patch pending.
Task 8: complete (commits f754dc8..6e2ebf3, 31 tests). Subagent extended doubled-op convention to yy/cc (brief covered dd only).
Task 9: complete (commits 6e2ebf3..1b33b8b, 37 tests). Subagent fixed pair-finder off-by-one and symmetric-char-pair overload.
Task 11: complete (commits e10ba55..f28cdbc, 44 tests). Subagent added visual y cursor reset to selection start.
Task 12: complete (commits a58ef6f..0f0c998, 50 tests). Subagent caught split-session test bug; folded into one session. ALSO fixed: 'c' operator keeps single open batch (real Vim one-undo-unit), wired u/Ctrl-r keys (plan had no Normal-mode handler).
Task 13: complete (milestone tagged plan-a-core-complete)
Task 1: complete (commits 114f454..ff60730, review clean)
Task 2: complete (commits ff60730..5c59b11, review clean)
Task 3: complete (commits 5c59b11..f9d7cfd, review clean)
Task 4: complete (commit d01c4f1, 86 tests pass)
Cursor polish: complete (cafa967, review clean)
Task 1: complete (commits 426a3cc..0162ca0, review clean)
Task 2: complete (commits 0162ca0..1184e68, docs only)
