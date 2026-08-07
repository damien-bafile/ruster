#[test]
fn tui_flag_fails_without_terminal() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ruster"))
        .args(["--tui", "Cargo.toml"])
        .output()
        .expect("failed to run ruster");
    assert!(!output.status.success(), "expected failure without terminal");
}
