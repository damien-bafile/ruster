#[test]
fn binary_prints_usage_without_args() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ruster"))
        .output()
        .expect("failed to run ruster");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Usage: ruster <file>"));
}
