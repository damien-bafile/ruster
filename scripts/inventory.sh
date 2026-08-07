#!/usr/bin/env bash
#
# Capture a deterministic inventory of the product's features at a given git ref.
#
#   scripts/inventory.sh <ref> <outdir>     capture one ref
#   scripts/inventory.sh --check <dir>...   gate three captured dirs
#
# The point is to be able to prove, across a merge, that nothing was lost: run it
# at both parents and at the result, then check that the result is a superset of
# the union of the parents. Additions are expected; removals are the finding.
#
# Two rules make this work, and both are load bearing.
#
# It never checks anything out. Every text artifact comes from `git show
# <ref>:<path>`, so the script can inventory a ref that predates the script
# itself — which is exactly what is needed the first time it runs.
#
# It compiles the settings inventory rather than grepping it. rustfmt rewraps the
# `add(...)` calls in schema.rs, so a text scrape reports every setting as deleted
# the moment a formatting pass lands between two refs. Anything that survives a
# reformat is scraped; anything that does not is built.
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"

APP=crates/ruster-tui/src/app.rs

# ---------------------------------------------------------------- capture ----

# Read one file at a ref. Missing files are empty rather than fatal, so the
# script can inventory refs from before a file existed.
at() { git show "$REF:$2" 2>/dev/null || true; }

capture() {
    REF="$(git rev-parse "$1")"
    local out="$2"
    mkdir -p "$out"
    echo "  $1 ($(git rev-parse --short "$REF")) -> $out"

    # Every tracked path. The most direct possible proof that nothing was deleted.
    git ls-tree -r --name-only "$REF" | sort > "$out/files.txt"

    # Commands the cmdline parser actually accepts. Deliberately looser than the
    # scrape in tests/docs_in_sync.rs, because this one has to survive a rewrap.
    at "$REF" "$APP" \
        | awk '/fn parse_cmdline/{f=1} f&&/^    fn [a-z_]+\(/&&!/parse_cmdline/{exit} f' \
        | grep -oE '"[A-Za-z][A-Za-z0-9 /!_-]*"' | tr -d '"' | sort -u > "$out/commands.txt"

    # The CmdAction enum: one variant per thing the editor can be told to do.
    at "$REF" "$APP" \
        | awk '/^enum CmdAction \{/{f=1;next} f&&/^\}/{exit} f' \
        | grep -oE '^    [A-Z][A-Za-z0-9]*' | tr -d ' ' | sort -u > "$out/cmdactions.txt"

    # The leader-key tree: key, label and action for every node.
    at "$REF" "$APP" \
        | grep -oE 'LeaderNode::(Action|Group)\("[^"]*"(, LeaderAction::[A-Za-z0-9]*)?' \
        | sort -u > "$out/leader.txt"

    # The curated M-x palette.
    at "$REF" "$APP" \
        | awk '/^const PALETTE_COMMANDS/{f=1;next} f&&/^\];/{exit} f' \
        | grep -oE '^\s*\("[^"]*"' | tr -d ' (' | sort -u > "$out/palette.txt"

    # Everything a config.lua can call.
    at "$REF" crates/ruster-lua/src/api.rs \
        | grep -oE '\.set\("[^"]+"' | grep -oE '"[^"]+"' | tr -d '"' | sort -u > "$out/luaapi.txt"

    # Events a plugin can hook. Nothing else in the tree guards this list.
    at "$REF" "$APP" \
        | grep -oE 'fire_event(_str|_nums)?\("[^"]+"' \
        | grep -oE '"[^"]+"' | tr -d '"' | sort -u > "$out/luaevents.txt"

    # Commands the dashboard advertises. These are prose, not parser branches, so
    # they can and did drift into advertising commands that never existed.
    { at "$REF" crates/ruster-tui/src/widgets/mod.rs
      at "$REF" crates/ruster-tui/src/widgets.rs
      at "$REF" crates/ruster-render-raylib/src/lib.rs
    } | grep -ohE '":[A-Za-z][A-Za-z0-9]*' | sed 's/^"//' | sort -u > "$out/advertised.txt"

    # Doc structure, so a whole section going missing is visible. Report-only.
    : > "$out/docheads.txt"
    for d in $(git ls-tree -r --name-only "$REF" -- docs | grep '\.md$' | sort); do
        at "$REF" "$d" | grep -E '^#{1,4} ' | sed "s|^|$(basename "$d")\t|"
    done | sort >> "$out/docheads.txt"

    capture_built "$REF" "$out"
}

# The two artifacts that have to be compiled: the settings schema (rewrap-proof
# only when rendered by the code that owns it) and the test-name list.
capture_built() {
    local ref="$1" out="$2"
    local wt=".worktrees/inv-$(git rev-parse --short "$ref")"

    git worktree add -q --detach "$wt" "$ref" 2>/dev/null || {
        echo "    ! could not create worktree for $ref; skipping built artifacts" >&2
        : > "$out/schema-and-themes.txt"; : > "$out/tests.txt"; return
    }
    install -m644 scripts/inventory/dump.rs "$wt/crates/ruster-lua/examples/dump.rs" 2>/dev/null \
        || mkdir -p "$wt/crates/ruster-lua/examples" \
        && install -m644 scripts/inventory/dump.rs "$wt/crates/ruster-lua/examples/dump.rs"

    # One shared target dir across refs: the dependency graph barely moves, so
    # this turns three cold builds into one.
    export CARGO_TARGET_DIR="$REPO/target/inventory"

    ( cd "$wt" && cargo run -q -p ruster-lua --example dump 2>/dev/null ) > "$out/schema-and-themes.txt"

    # Per package, so names are attributed and the ordering is ours rather than
    # cargo's. stderr carries binary paths with unstable hashes — drop it.
    #
    # The package list must come from `.packages[].name` specifically. Grepping
    # the metadata JSON for `"name"` also matches every entry in each package's
    # dependencies array, and `cargo test -p` accepts those, so the inventory
    # quietly filled up with lsp-types' and serde's unit tests — 97 of them,
    # tracking dependency versions rather than our own code.
    ( cd "$wt" && cargo metadata --no-deps --format-version 1 2>/dev/null ) \
        | jq -r '.packages[].name' | sort -u \
        | while read -r p; do
            ( cd "$wt" && cargo test -q -p "$p" -- --list 2>/dev/null ) \
                | grep ': test$' | sed "s|^|$p |"
          done | sort -u > "$out/tests.txt"

    git worktree remove --force "$wt" 2>/dev/null || true
}

# ------------------------------------------------------------------ check ----

# Set-valued artifacts. The result must be a superset of the union of the
# parents: that catches both "main lost something" and "the branch's own
# contribution was dropped while resolving a conflict".
SETS=(files commands cmdactions leader palette luaapi luaevents advertised tests)

check() {
    local a="$1" b="$2" after="$3"
    local whitelist="scripts/inventory/whitelist.txt"
    local failed=0

    echo "Removals (must be empty except whitelisted):"
    for s in "${SETS[@]}"; do
        local lost
        lost="$(comm -23 <(sort -u "$a/$s.txt" "$b/$s.txt") <(sort -u "$after/$s.txt") \
                | { [ -f "$whitelist" ] && grep -vxFf <(grep -v '^#' "$whitelist" | grep -v '^$') || cat; })"
        if [ -n "$lost" ]; then
            failed=1
            echo "  FAIL $s"
            echo "$lost" | sed 's/^/        - /'
        else
            echo "  ok   $s"
        fi
    done

    # The schema is a rendered document, not a set. Every group.key must survive;
    # a changed default is a whitelist matter, not a failure.
    local keys_lost
    keys_lost="$(comm -23 \
        <(cat "$a/schema-and-themes.txt" "$b/schema-and-themes.txt" | grep -oE '^[a-z_]+\.[a-z_]+' | sort -u) \
        <(grep -oE '^[a-z_]+\.[a-z_]+' "$after/schema-and-themes.txt" | sort -u))"
    if [ -n "$keys_lost" ]; then
        failed=1; echo "  FAIL settings keys"; echo "$keys_lost" | sed 's/^/        - /'
    else
        echo "  ok   settings keys"
    fi

    # Reported against the *first* parent rather than the union: the union
    # already contains everything the second parent brought, so measuring
    # against it would always read as zero. What a reviewer wants to know is
    # what the destination branch gained.
    echo
    echo "Gained by $(basename "$a") (informational):"
    for s in "${SETS[@]}"; do
        local n
        n="$(comm -13 <(sort -u "$a/$s.txt") <(sort -u "$after/$s.txt") | wc -l)"
        [ "$n" -gt 0 ] && echo "  +$n $s"
    done

    echo
    [ "$failed" -eq 0 ] && echo "PASS — nothing lost." || echo "FAIL — see removals above."
    return "$failed"
}

# ------------------------------------------------------------------- main ----

case "${1:-}" in
    --check)
        shift
        [ $# -eq 3 ] || { echo "usage: $0 --check <before-a> <before-b> <after>" >&2; exit 2; }
        check "$@"
        ;;
    "" | -h | --help)
        sed -n '2,20p' "${BASH_SOURCE[0]}" | sed 's|^# \?||'
        ;;
    *)
        [ $# -eq 2 ] || { echo "usage: $0 <ref> <outdir>" >&2; exit 2; }
        echo "Capturing inventory:"
        capture "$1" "$2"
        ;;
esac
