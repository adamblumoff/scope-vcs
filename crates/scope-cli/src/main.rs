use anyhow::{Context, bail};
use clap::{Parser, Subcommand, ValueEnum};
use scope_crypto::{ManifestMixedPolicy, PushManifest, sign_manifest};
use scope_git::build_virtual_git_projection;
use scope_policy::ScopePath;
use scope_projection::project_graph;
use scope_store::{DemoRepository, demo_repository};

const MANIFEST_SIGNING_SECRET_ENV: &str = "SCOPE_MANIFEST_SIGNING_SECRET";

#[derive(Debug, Parser)]
#[command(name = "sx")]
#[command(about = "Scope VCS command-line prototype")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Demo {
        #[command(subcommand)]
        command: DemoCommand,
    },
    Manifest {
        #[command(subcommand)]
        command: ManifestCommand,
    },
}

#[derive(Debug, Subcommand)]
enum DemoCommand {
    Projection {
        #[arg(long, default_value = "public")]
        principal: String,
        #[arg(long)]
        git: bool,
    },
    Check {
        #[arg(long)]
        principal: String,
        #[arg(long)]
        path: String,
        #[arg(long, default_value = "read")]
        operation: Operation,
    },
}

#[derive(Debug, Subcommand)]
enum ManifestCommand {
    Create {
        #[arg(long, default_value = "scope-demo")]
        repo: String,
        #[arg(long)]
        principal: String,
        #[arg(long, default_value = "dev-device")]
        device: String,
        #[arg(long)]
        graph: String,
        #[arg(long = "path", required = true)]
        paths: Vec<String>,
        #[arg(long, default_value = "synthetic-public-commit")]
        mixed_policy: CliMixedPolicy,
    },
}

#[derive(Clone, Debug, ValueEnum)]
enum Operation {
    Read,
    Write,
}

#[derive(Clone, Debug, ValueEnum)]
enum CliMixedPolicy {
    SyntheticPublicCommit,
    OmitFromPublic,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Demo { command } => run_demo(command),
        Command::Manifest { command } => run_manifest(command),
    }
}

fn run_demo(command: DemoCommand) -> anyhow::Result<()> {
    let demo = demo_repository();
    match command {
        DemoCommand::Projection { principal, git } => {
            let principal = DemoRepository::projection_principal(&principal);
            let projection = project_graph(&demo.policy, &demo.graph, &principal);
            if git {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&build_virtual_git_projection(&projection))?
                );
            } else {
                println!("{}", serde_json::to_string_pretty(&projection)?);
            }
        }
        DemoCommand::Check {
            principal,
            path,
            operation,
        } => {
            let principal = DemoRepository::projection_principal(&principal);
            let path = ScopePath::parse(path).context("invalid scope path")?;
            let allowed = match operation {
                Operation::Read => demo.policy.can_read(&principal, &path),
                Operation::Write => demo.policy.can_write(&principal, &path),
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "principal": principal.id,
                    "path": path.as_str(),
                    "operation": format!("{operation:?}").to_lowercase(),
                    "allowed": allowed
                }))?
            );
        }
    }
    Ok(())
}

fn run_manifest(command: ManifestCommand) -> anyhow::Result<()> {
    match command {
        ManifestCommand::Create {
            repo,
            principal,
            device,
            graph,
            paths,
            mixed_policy,
        } => {
            let mixed_policy = match mixed_policy {
                CliMixedPolicy::SyntheticPublicCommit => ManifestMixedPolicy::SyntheticPublicCommit,
                CliMixedPolicy::OmitFromPublic => ManifestMixedPolicy::OmitFromPublic,
            };
            let manifest = PushManifest::new(repo, principal, device, graph, paths, mixed_policy);
            let signing_secret = manifest_signing_secret()?;
            let signed = sign_manifest(manifest, signing_secret.as_bytes())?;
            println!("{}", serde_json::to_string_pretty(&signed)?);
        }
    }
    Ok(())
}

fn manifest_signing_secret() -> anyhow::Result<String> {
    let secret = std::env::var(MANIFEST_SIGNING_SECRET_ENV).with_context(|| {
        format!("{MANIFEST_SIGNING_SECRET_ENV} is required for manifest signing")
    })?;
    if secret.is_empty() {
        bail!("{MANIFEST_SIGNING_SECRET_ENV} cannot be empty");
    }
    Ok(secret)
}
