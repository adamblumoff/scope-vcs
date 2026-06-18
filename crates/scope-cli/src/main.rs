use anyhow::{Context, bail};
use clap::{Parser, Subcommand, ValueEnum};
use scope_crypto::{ManifestMixedPolicy, PushManifest, sign_manifest};
use scope_git::build_virtual_git_projection;
use scope_policy::ScopePath;
use scope_projection::project_graph;
use scope_store::{
    BOOTSTRAP_REPO_ID, BOOTSTRAP_REPO_NAME, BOOTSTRAP_REPO_OWNER, VerifiedEmail, app_catalog,
};

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
    Repo {
        #[command(subcommand)]
        command: RepoCommand,
    },
    Manifest {
        #[command(subcommand)]
        command: ManifestCommand,
    },
}

#[derive(Debug, Subcommand)]
enum RepoCommand {
    Projection {
        #[arg(long, default_value = BOOTSTRAP_REPO_OWNER)]
        owner: String,
        #[arg(long, default_value = BOOTSTRAP_REPO_NAME)]
        repo: String,
        #[arg(long)]
        email: Option<String>,
        #[arg(long)]
        verified: bool,
        #[arg(long)]
        git: bool,
    },
    Check {
        #[arg(long, default_value = BOOTSTRAP_REPO_OWNER)]
        owner: String,
        #[arg(long, default_value = BOOTSTRAP_REPO_NAME)]
        repo: String,
        #[arg(long)]
        email: Option<String>,
        #[arg(long)]
        verified: bool,
        #[arg(long)]
        path: String,
        #[arg(long, default_value = "read")]
        operation: Operation,
    },
}

#[derive(Debug, Subcommand)]
enum ManifestCommand {
    Create {
        #[arg(long, default_value = BOOTSTRAP_REPO_ID)]
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
        Command::Repo { command } => run_repo(command),
        Command::Manifest { command } => run_manifest(command),
    }
}

fn run_repo(command: RepoCommand) -> anyhow::Result<()> {
    let catalog = app_catalog();
    match command {
        RepoCommand::Projection {
            owner,
            repo,
            email,
            verified,
            git,
        } => {
            let repository = catalog
                .repository(&owner, &repo)
                .with_context(|| format!("repo {owner}/{repo} not found"))?;
            let identity = email.map(|email| VerifiedEmail::new(email, verified));
            let principal = catalog.principal_for_repo(repository, identity.as_ref());
            if !catalog.can_read_path(repository, &principal, &ScopePath::root()) {
                bail!("repo {owner}/{repo} is not readable by {}", principal.id);
            }
            let projection = project_graph(&repository.policy, &repository.graph, &principal);
            if git {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&build_virtual_git_projection(&projection))?
                );
            } else {
                println!("{}", serde_json::to_string_pretty(&projection)?);
            }
        }
        RepoCommand::Check {
            owner,
            repo,
            email,
            verified,
            path,
            operation,
        } => {
            let repo = catalog
                .repository(&owner, &repo)
                .with_context(|| format!("repo {owner}/{repo} not found"))?;
            let identity = email.map(|email| VerifiedEmail::new(email, verified));
            let principal = catalog.principal_for_repo(repo, identity.as_ref());
            let path = ScopePath::parse(path).context("invalid scope path")?;
            let allowed = match operation {
                Operation::Read => catalog.can_read_path(repo, &principal, &path),
                Operation::Write => catalog.can_write_path(repo, &principal, &path),
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
