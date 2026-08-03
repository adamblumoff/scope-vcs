use std::process::Command;

#[test]
fn version_reports_package_build_and_protocol_identity() {
    let output = Command::new(env!("CARGO_BIN_EXE_scope"))
        .arg("--version")
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with(&format!("scope {}", env!("CARGO_PKG_VERSION"))));
    assert!(stdout.contains("build "));
    assert!(stdout.contains("protocol 1"));
}
