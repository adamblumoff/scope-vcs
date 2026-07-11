use clap::{Parser, Subcommand};
use scope_cli::{
    api::{api_url, http_client},
    git_credential::run_git_credential,
    git_repo::discover_git_repo,
    login::session_from_cache_or_browser,
    push::DEFAULT_SCOPE_REMOTE,
    request::{RequestArgs, prepare_request_command, run_request_command},
    review::run_standalone_review,
};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "scope")]
#[command(about = "Scope VCS command line")]
struct Cli {
    #[command(subcommand)]
    command: CommandKind,
}

#[derive(Subcommand)]
enum CommandKind {
    Init(InitArgs),
    Push(PushArgs),
    #[command(about = "Pull main and every visible request from Scope")]
    Pull(PullArgs),
    #[command(about = "Review repo visibility config locally")]
    Review,
    #[command(about = "Work with named Scope requests")]
    Request(RequestArgs),
    Clone(CloneArgs),
    Login(LoginArgs),
    Logout,
    Whoami,
    #[command(name = "git-credential", hide = true)]
    GitCredential(GitCredentialArgs),
}

#[derive(Parser)]
struct InitArgs {
    #[arg(long)]
    name: Option<String>,
}

#[derive(Parser)]
struct PushArgs {
    #[arg(long, default_value = DEFAULT_SCOPE_REMOTE)]
    remote: String,
    #[arg(
        long,
        help = "Skip local visibility review and push using committed config"
    )]
    no_review: bool,
}

#[derive(Parser)]
struct PullArgs {
    #[arg(long, help = "Scope Git remote to fetch (auto-detected by default)")]
    remote: Option<String>,
}

#[derive(Parser)]
struct CloneArgs {
    repository: String,
    destination: Option<PathBuf>,
}

#[derive(Parser)]
struct LoginArgs {
    #[arg(long)]
    headless: bool,
    #[arg(long, value_name = "TOKEN")]
    exchange: Option<String>,
}

#[derive(Parser)]
struct GitCredentialArgs {
    operation: String,
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        CommandKind::Init(args) => scope_cli::init::run(args.name),
        CommandKind::Push(args) => scope_cli::push::run(&args.remote, args.no_review),
        CommandKind::Pull(args) => scope_cli::pull::run(args.remote.as_deref()),
        CommandKind::Review => {
            let repo = discover_git_repo("scope review")?;
            run_standalone_review(&repo)
        }
        CommandKind::Request(args) => run_request(args),
        CommandKind::Clone(args) => {
            scope_cli::clone::clone_repo(&args.repository, args.destination.as_deref())
        }
        CommandKind::Login(args) => scope_cli::login::login(args.headless, args.exchange),
        CommandKind::Logout => scope_cli::login::logout(),
        CommandKind::Whoami => scope_cli::login::whoami(),
        CommandKind::GitCredential(args) => run_git_credential(&args.operation),
    }
}

fn run_request(args: RequestArgs) -> anyhow::Result<()> {
    let command = prepare_request_command(args)?;
    let api_url = api_url();
    let client = http_client()?;
    let session = session_from_cache_or_browser(&client, &api_url)?;
    run_request_command(command, &client, &api_url, &session.token)
}
