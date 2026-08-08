use clap::{CommandFactory, FromArgMatches, Parser, Subcommand, error::ErrorKind};
use scope_cli::{
    api::{api_url, http_client},
    auth::cached_cli_session,
    error::CliError,
    git_credential::run_git_credential,
    git_repo::discover_git_repo,
    login::session_from_cache_or_browser,
    request::{RequestArgs, prepare_request_command, run_request_command},
    review::run_standalone_review,
};
use std::{path::PathBuf, process::ExitCode};

#[derive(Parser)]
#[command(name = "scope")]
#[command(about = "Scope VCS command line")]
struct Cli {
    #[arg(
        long,
        global = true,
        help = "Print one machine-readable JSON document for request commands"
    )]
    json: bool,
    #[command(subcommand)]
    command: CommandKind,
}

#[derive(Subcommand)]
enum CommandKind {
    Init(InitArgs),
    Push(PushArgs),
    #[command(about = "Pull main and every visible request from Scope")]
    Pull(PullArgs),
    #[command(about = "Review file visibility config locally")]
    Review,
    #[command(about = "Manage repository contribution rules for coding agents")]
    Rules(RulesArgs),
    #[command(about = "Work with named Scope requests")]
    Request(RequestArgs),
    Clone(CloneArgs),
    Login(LoginArgs),
    Logout,
    Whoami,
    #[command(about = "Run committed workflows on self-hosted runners")]
    Run(RunArgs),
    #[command(about = "Install and manage this machine as a self-hosted runner")]
    Runner(RunnerArgs),
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
    #[arg(long, help = "Scope Git remote to push (auto-detected by default)")]
    remote: Option<String>,
    #[arg(
        long,
        help = "Skip local visibility review and push using committed config"
    )]
    no_review: bool,
    #[arg(long, help = "Wait for push-triggered workflows to finish")]
    wait: bool,
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
struct RulesArgs {
    #[command(subcommand)]
    command: RulesCommand,
}

#[derive(Subcommand)]
enum RulesCommand {
    #[command(about = "Create .scope/RULES.md and sync detected repo-level agent files")]
    Sync,
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

#[derive(Parser)]
struct RunArgs {
    #[arg(help = "Workflow name, or watch/cancel/retry")]
    target: String,
    #[arg(help = "Run ID for watch/cancel/retry")]
    run_id: Option<String>,
    #[arg(long, help = "Run on this repository-scoped runner name")]
    runner: Option<String>,
    #[arg(long, help = "Scope Git remote to use (auto-detected by default)")]
    remote: Option<String>,
    #[arg(long, help = "Queue or retry without following logs")]
    no_watch: bool,
}

#[derive(Parser)]
struct RunnerArgs {
    #[command(subcommand)]
    command: RunnerCommand,
}

#[derive(Subcommand)]
enum RunnerCommand {
    Install {
        #[arg(long)]
        name: String,
        #[arg(long, value_name = "OWNER/REPO")]
        repo: String,
        #[arg(long, value_name = "1-16")]
        max_concurrent_jobs: Option<u8>,
    },
    Status,
    Doctor,
    #[command(about = "Inspect or prune this runner's persistent caches")]
    Cache {
        #[command(subcommand)]
        command: RunnerCacheCommand,
    },
    AddRepo {
        repository: String,
    },
    RemoveRepo {
        repository: String,
    },
    #[command(hide = true)]
    Daemon {
        #[arg(long, hide = true)]
        config: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum RunnerCacheCommand {
    List,
    Prune {
        #[arg(
            long,
            help = "Remove every inactive Scope cache, not only enough to restore reserves"
        )]
        all: bool,
    },
}

fn main() -> ExitCode {
    let json_requested = std::env::args_os().any(|arg| arg == "--json");
    let matches = match Cli::command()
        .version(scope_cli::build::version_identity())
        .try_get_matches()
    {
        Ok(matches) => matches,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            error.exit()
        }
        Err(error) if json_requested => {
            let response = scope_cli::api::ErrorResponse::new(
                scope_cli::api::ErrorCode::BadRequest,
                error.to_string(),
            );
            eprintln!("{}", serde_json::to_string(&response).unwrap());
            return ExitCode::from(2);
        }
        Err(error) => error.exit(),
    };
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(error) => error.exit(),
    };
    let json = cli.json;
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if json {
                let response = scope_cli::error::response(&error);
                eprintln!("{}", serde_json::to_string(&response).unwrap());
            } else {
                eprintln!("{error:#}");
            }
            ExitCode::from(scope_cli::error::exit_code(&error))
        }
    }
}

fn run(cli: Cli) -> anyhow::Result<()> {
    if cli.json && !matches!(&cli.command, CommandKind::Request(_)) {
        return Err(
            scope_cli::error::CliError::new(scope_cli::api::ErrorResponse::new(
                scope_cli::api::ErrorCode::BadRequest,
                "--json currently supports request commands only",
            ))
            .into(),
        );
    }
    match cli.command {
        CommandKind::Init(args) => scope_cli::init::run(args.name),
        CommandKind::Push(args) => {
            scope_cli::push::run(args.remote.as_deref(), args.no_review, args.wait)
        }
        CommandKind::Pull(args) => scope_cli::pull::run(args.remote.as_deref()),
        CommandKind::Review => {
            let repo = discover_git_repo("scope review")?;
            run_standalone_review(&repo)
        }
        CommandKind::Rules(args) => run_rules(args.command),
        CommandKind::Request(args) => run_request(args, cli.json),
        CommandKind::Clone(args) => {
            scope_cli::clone::clone_repo(&args.repository, args.destination.as_deref())
        }
        CommandKind::Login(args) => scope_cli::login::login(args.headless, args.exchange),
        CommandKind::Logout => scope_cli::login::logout(),
        CommandKind::Whoami => scope_cli::login::whoami(),
        CommandKind::Run(args) => run_workflow(args),
        CommandKind::Runner(args) => run_runner(args.command),
        CommandKind::GitCredential(args) => run_git_credential(&args.operation),
    }
}

fn run_rules(command: RulesCommand) -> anyhow::Result<()> {
    let repo = discover_git_repo("scope rules")?;
    match command {
        RulesCommand::Sync => {
            let result = scope_cli::agent_context::sync_repo_rules(&repo.root)?;
            if result.changed_paths.is_empty() {
                println!("Scope rules context is already in sync.");
            } else {
                for path in result.changed_paths {
                    println!("Updated {}", path.display());
                }
            }
            Ok(())
        }
    }
}

fn run_request(args: RequestArgs, json: bool) -> anyhow::Result<()> {
    let command = prepare_request_command(args)?;
    let api_url = api_url();
    let client = http_client()?;
    let session = if json {
        let Some(session) = cached_cli_session(&client, &api_url)? else {
            return Err(CliError::new(scope_cli::api::ErrorResponse::new(
                scope_cli::api::ErrorCode::Unauthorized,
                "not signed in; run scope login",
            ))
            .into());
        };
        session
    } else {
        session_from_cache_or_browser(&client, &api_url)?
    };
    run_request_command(command, &client, &api_url, &session.token, json)?.render(json)
}

fn run_workflow(args: RunArgs) -> anyhow::Result<()> {
    match (args.target.as_str(), args.run_id.as_deref()) {
        ("watch", Some(run_id)) if args.runner.is_none() && !args.no_watch => {
            scope_cli::run::watch(run_id, args.remote.as_deref())
        }
        ("cancel", Some(run_id)) if args.runner.is_none() && !args.no_watch => {
            scope_cli::run::cancel(run_id, args.remote.as_deref())
        }
        ("retry", Some(run_id)) if args.runner.is_none() => {
            scope_cli::run::retry(run_id, args.remote.as_deref(), args.no_watch)
        }
        ("watch" | "cancel", Some(_)) => {
            Err(CliError::usage(
                "--runner is only valid when starting a workflow; --no-watch is not valid for watch or cancel"
            )
            .into())
        }
        ("retry", Some(_)) => {
            Err(CliError::usage("--runner is only valid when starting a workflow").into())
        }
        ("watch" | "cancel" | "retry", None) => {
            Err(CliError::usage(format!(
                "scope run {} requires a run ID",
                args.target
            ))
            .into())
        }
        (_, Some(_)) => Err(CliError::usage(
            "a run ID is accepted only by `scope run watch`, `scope run cancel`, or `scope run retry`"
        )
        .into()),
        (workflow, None) => scope_cli::run::start(
            workflow,
            args.runner.as_deref(),
            args.remote.as_deref(),
            args.no_watch,
        ),
    }
}

fn run_runner(command: RunnerCommand) -> anyhow::Result<()> {
    match command {
        RunnerCommand::Install {
            name,
            repo,
            max_concurrent_jobs,
        } => scope_cli::runner::install(&name, &repo, max_concurrent_jobs),
        RunnerCommand::Status => scope_cli::runner::status(),
        RunnerCommand::Doctor => scope_cli::runner::doctor(),
        RunnerCommand::Cache { command } => match command {
            RunnerCacheCommand::List => scope_cli::runner::list_caches(),
            RunnerCacheCommand::Prune { all } => scope_cli::runner::prune_caches(all),
        },
        RunnerCommand::AddRepo { repository } => scope_cli::runner::add_repository(&repository),
        RunnerCommand::RemoveRepo { repository } => {
            scope_cli::runner::remove_repository(&repository)
        }
        RunnerCommand::Daemon { config } => scope_cli::runner::daemon(config.as_deref()),
    }
}
