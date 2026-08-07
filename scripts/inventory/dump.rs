//! Print the settings schema and the built-in themes, for the inventory harness.
//!
//! This is not compiled in-tree. `scripts/inventory.sh` installs it into a
//! throwaway worktree as `crates/ruster-lua/examples/dump.rs` and runs it there,
//! so the same dump can be produced at any ref — including refs from before this
//! file existed.
//!
//! It exists because the settings inventory cannot be scraped. rustfmt rewraps
//! the `add(...)` calls in `schema.rs`, so a `grep` for them returns 76 matches
//! on an unformatted ref and 0 on a formatted one, which reads as "every setting
//! was deleted". Asking the code to render its own schema is immune to that.
//!
//! Kept to `ruster_lua::{schema, config}` deliberately: those two modules are
//! stable across the refs being compared, so this compiles at all of them.

fn main() {
    // The generated config.lua is already a complete, ordered rendering of every
    // setting and its default — there is no better artifact to invent.
    println!("<<<SETTINGS>>>");
    print!("{}", ruster_lua::schema::generate_default_config());

    // Themes, with each palette role, so a colour disappearing from a theme is
    // visible rather than silent.
    println!("<<<THEMES>>>");
    for (name, theme) in ruster_lua::config::builtin_themes() {
        for (role, color) in &theme.palette {
            println!("{name}\t{role}\t{color:?}");
        }
    }
}
