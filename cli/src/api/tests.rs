use super::*;
use ts_rs::TS;

const API_TYPES: &str = include_str!("../../../web/src/api/types.generated.ts");

#[test]
fn cli_auth_dtos_match_generated_api_contract() {
    assert_type_matches::<SessionIdentity>("SessionIdentity");
    assert_type_matches::<RepositoryActor>("RepositoryActor");
    assert_type_matches::<RepoPublicationState>("RepoPublicationState");
    assert_type_matches::<UserResponse>("UserResponse");
    assert_type_matches::<AccountSessionResponse>("AccountSessionResponse");
    assert_type_matches::<DeviceLoginStatus>("DeviceLoginStatus");
    assert_type_matches::<DeviceLoginStartResponse>("DeviceLoginStartResponse");
    assert_type_matches::<DeviceLoginPollResponse>("DeviceLoginPollResponse");
    assert_type_matches::<BrowserLoginStartRequest>("BrowserLoginStartRequest");
    assert_type_matches::<BrowserLoginStartResponse>("BrowserLoginStartResponse");
    assert_type_matches::<BrowserLoginExchangeRequest>("BrowserLoginExchangeRequest");
    assert_type_matches::<CliSessionTokenResponse>("CliSessionTokenResponse");
    assert_type_matches::<CliExchangeGrantExchangeRequest>("CliExchangeGrantExchangeRequest");
}

#[test]
fn cli_auth_endpoints_match_generated_api_contract() {
    assert_endpoint_matches("accountSession", ACCOUNT_SESSION_PATH);
    assert_endpoint_matches("cliSession", CLI_SESSION_PATH);
    assert_endpoint_matches("deviceLoginStart", CLI_DEVICE_LOGIN_PATH);
    assert_endpoint_matches("deviceLoginPoll", CLI_DEVICE_LOGIN_POLL_PATH_TEMPLATE);
    assert_endpoint_matches("browserLoginStart", CLI_BROWSER_LOGIN_PATH);
    assert_endpoint_matches(
        "browserLoginExchange",
        CLI_BROWSER_LOGIN_EXCHANGE_PATH_TEMPLATE,
    );
    assert_endpoint_matches("exchangeGrantExchange", CLI_EXCHANGE_GRANTS_EXCHANGE_PATH);
}

#[test]
fn duplicate_repo_error_names_repo_and_next_steps() {
    let message = duplicate_repo_error_message("scope-vcs");

    assert!(message.contains("Scope repository \"scope-vcs\" already exists"));
    assert!(message.contains("scope init --name <new-name>"));
    assert!(message.contains("scope push"));
}

fn assert_type_matches<T: TS>(name: &str) {
    let config = ts_rs::Config::new().with_large_int("number");
    let cli_declaration = format!("export {}", T::decl(&config));
    let api_declaration = exported_type_declaration(name);
    assert_eq!(cli_declaration, api_declaration, "{name} drifted");
}

fn exported_type_declaration(name: &str) -> String {
    let prefix = format!("export type {name} = ");
    API_TYPES
        .lines()
        .find(|line| line.starts_with(&prefix))
        .unwrap_or_else(|| panic!("missing generated API declaration for {name}"))
        .to_string()
}

fn assert_endpoint_matches(name: &str, cli_path: &str) {
    let api_path = exported_endpoint_path(name);
    assert_eq!(cli_path, api_path, "{name} endpoint drifted");
}

fn exported_endpoint_path(name: &str) -> &str {
    let prefix = format!("  {name}: \"");
    let line = API_TYPES
        .lines()
        .find(|line| line.starts_with(&prefix))
        .unwrap_or_else(|| panic!("missing generated API endpoint for {name}"));
    line.strip_prefix(&prefix)
        .and_then(|tail| tail.strip_suffix("\","))
        .unwrap_or_else(|| panic!("invalid generated API endpoint line for {name}"))
}
