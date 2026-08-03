mod support;

use support::*;

#[test]
fn local_authentication_and_usage_failures_use_stable_exit_categories() {
    let dir = TempDir::new("local-error-contract");
    let config = TempDir::new("local-error-config");

    let whoami = scope_command(dir.path())
        .env("XDG_CONFIG_HOME", config.path())
        .arg("whoami")
        .output()
        .unwrap();
    assert_eq!(whoami.status.code(), Some(3), "{whoami:?}");
    assert!(
        String::from_utf8(whoami.stderr)
            .unwrap()
            .contains("not signed in")
    );

    let invalid_login = scope_command(dir.path())
        .env("XDG_CONFIG_HOME", config.path())
        .args(["login", "--headless", "--exchange", "fixture"])
        .output()
        .unwrap();
    assert_eq!(invalid_login.status.code(), Some(2), "{invalid_login:?}");
    assert!(
        String::from_utf8(invalid_login.stderr)
            .unwrap()
            .contains("cannot be used together")
    );
}
