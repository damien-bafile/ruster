# Product

<!-- impeccable:product-schema 1 -->

## Platform

native desktop — dual-mode: raylib GUI and ratatui+crossterm TUI

## Users

Software developers — from individual hobbyists to professional engineers — who spend long hours editing code and want an editor that is both high-performance (60fps) and deeply extensible. Primary audience: developers already familiar with Neovim's modal editing or Emacs's modeless extensibility, who want a unified modern alternative.

## Product Purpose

ruster is a self-contained IDE and application platform — a hybrid editor combining Neovim's modal editing and Emacs's modeless paradigms — written in Rust for raw performance. Success means being a daily-driver editor that users choose over VS Code, Neovim, or Emacs for its speed, dual-paradigm flexibility, and Lua-powered extensibility.

## Positioning

The only editor that lets you toggle between Neovim-style modal editing and Emacs-style keychord editing at runtime, backed by Rust's performance — without sacrificing either paradigm's power features (text objects, kill-ring, macros, dot-repeat).

## Operating Context

Developer workstations running Windows, macOS, Linux, FreeBSD, or BeOS. Editor sessions involve editing source code, running shell commands in an embedded terminal, interacting with LSP/DAP servers for code intelligence and debugging, managing files via a built-in file explorer, and running project builds/tests — all within the editor.

## Capabilities and Constraints

- Dual editing paradigm (Neovim/Emacs) toggleable at runtime
- Seven-phase feature roadmap from core engine (Phase 0) through application platform (Phase 7)
- Cross-platform targets via winit + rustix
- Lua scripting (mlua/Luau) for ALL configuration and plugins
- Tree-sitter for syntax, LSP for code intelligence, DAP for debugging
- Embedded PTY terminal via portable-pty + alacritty_terminal
- Rope-based text buffer (ropey) with undo-tree
- Plugin system with sandboxed Lua environments
- Written entirely in Rust; no existing visual design system documented yet
- Technical constraints: 60fps target, async event loop (tokio), immediate-mode GUI (raylib)

## Brand Commitments

- Name: "ruster" (lowercase)
- Identity: a modern hybrid editor; CLI-native with TUI as first-class
- Voice and personality not yet defined
- No existing logo or visual assets documented
- No existing DESIGN.md

## Evidence on Hand

- AGENTS.md — full product spec with seven-phase roadmap
- docs/ — detailed reference docs (config, Lua API, keybindings)
- docs/superpowers/specs/ — design specs for all phases
- docs/superpowers/plans/ — implementation plans for all phases
- Rust crate workspace with 10 crates implementing the engine

## Product Principles

1. **Performance is a feature** — 60fps target in both GUI and TUI; no jank, no wasted cycles.
2. **Dual-paradigm, not one-size-fits-all** — Neovim and Emacs users should both feel at home; neither mode is secondary.
3. **Extensibility by default** — Lua scripting is not optional; every surface is scriptable.
4. **Self-contained** — ruster should not depend on external tools for core functionality; IDE features are built in.
5. **Cross-platform without compromise** — the same experience on every target OS.

## Accessibility & Inclusion

Not yet established — no specific standards or user needs documented beyond general software accessibility.
