use clap::{Parser, Subcommand, ValueEnum};
use scope_core::domain::requests::{RequestAudience, ResolutionDisposition};

#[derive(Parser)]
pub struct RequestArgs {
    #[command(subcommand)]
    pub(super) command: RequestCommand,
}

#[derive(Subcommand)]
pub(super) enum RequestCommand {
    Start(RequestStartArgs),
    Submit(RequestSubmitArgs),
    Push(RequestPushArgs),
    Delete(RequestDeleteArgs),
    Status(RequestStatusArgs),
    #[command(about = "Start a top-level discussion on a request")]
    Discuss(RequestDiscussArgs),
    NeedsResponse(RequestNeedsResponseArgs),
    Respond(RequestRespondArgs),
    Resolve(RequestResolveArgs),
    Merge(RequestMergeArgs),
}

#[derive(Parser)]
pub(super) struct RequestStartArgs {
    #[arg(help = "Stable kebab-case request name used as the Git branch")]
    pub(super) name: String,
    #[arg(long)]
    pub(super) remote: Option<String>,
    #[arg(long, help = "Display title (defaults to the request name)")]
    pub(super) title: Option<String>,
    #[arg(
        long,
        value_enum,
        help = "Public or private request audience (defaults to repository visibility)"
    )]
    pub(super) audience: Option<RequestAudienceArg>,
}

#[derive(Parser)]
pub(super) struct RequestSubmitArgs {
    #[arg(long)]
    pub(super) remote: Option<String>,
    #[arg(long)]
    pub(super) stake_credits: Option<u32>,
}

#[derive(Parser)]
pub(super) struct RequestPushArgs {
    #[arg(long)]
    pub(super) remote: Option<String>,
    #[arg(help = "Request name or req_ ID (defaults to the current branch)")]
    pub(super) request: Option<String>,
}

#[derive(Parser)]
pub(super) struct RequestDeleteArgs {
    #[arg(long)]
    pub(super) remote: Option<String>,
    #[arg(help = "Request name or req_ ID (defaults to the current branch)")]
    pub(super) request: Option<String>,
}

#[derive(Parser)]
pub(super) struct RequestStatusArgs {
    #[arg(long)]
    pub(super) remote: Option<String>,
    #[arg(help = "Request name or req_ ID (defaults to the current branch)")]
    pub(super) request: Option<String>,
}

#[derive(Parser)]
pub(super) struct RequestDiscussArgs {
    #[arg(long)]
    pub(super) remote: Option<String>,
    #[arg(help = "Request name or req_ ID (defaults to the current branch)")]
    pub(super) request: Option<String>,
    #[arg(long, help = "Markdown body for the new discussion")]
    pub(super) body: String,
}

#[derive(Parser)]
pub(super) struct RequestNeedsResponseArgs {
    #[arg(long)]
    pub(super) remote: Option<String>,
    #[arg(help = "Request name or req_ ID (defaults to the current branch)")]
    pub(super) request: Option<String>,
    #[arg(long)]
    pub(super) body: String,
}

#[derive(Parser)]
pub(super) struct RequestRespondArgs {
    #[arg(long)]
    pub(super) remote: Option<String>,
    #[arg(help = "Request name or req_ ID (defaults to the current branch)")]
    pub(super) request: Option<String>,
    #[arg(long)]
    pub(super) body: Option<String>,
}

#[derive(Parser)]
pub(super) struct RequestResolveArgs {
    #[arg(long)]
    pub(super) remote: Option<String>,
    #[arg(help = "Request name or req_ ID (defaults to the current branch)")]
    pub(super) request: Option<String>,
    #[arg(long, value_enum)]
    pub(super) disposition: RequestResolveDisposition,
    #[arg(long)]
    pub(super) body: Option<String>,
}

#[derive(Parser)]
pub(super) struct RequestMergeArgs {
    #[arg(long)]
    pub(super) remote: Option<String>,
    #[arg(help = "Request name or req_ ID (defaults to the current branch)")]
    pub(super) request: Option<String>,
    #[arg(long)]
    pub(super) body: Option<String>,
    #[arg(long)]
    pub(super) yes: bool,
}

#[derive(Clone, Copy, ValueEnum)]
pub(super) enum RequestResolveDisposition {
    UsefulNotMerged,
    HiddenContext,
    NotAligned,
    Duplicate,
    Abandoned,
    LowQuality,
}

impl From<RequestResolveDisposition> for ResolutionDisposition {
    fn from(disposition: RequestResolveDisposition) -> Self {
        match disposition {
            RequestResolveDisposition::UsefulNotMerged => ResolutionDisposition::UsefulNotMerged,
            RequestResolveDisposition::HiddenContext => ResolutionDisposition::HiddenContext,
            RequestResolveDisposition::NotAligned => ResolutionDisposition::NotAligned,
            RequestResolveDisposition::Duplicate => ResolutionDisposition::Duplicate,
            RequestResolveDisposition::Abandoned => ResolutionDisposition::Abandoned,
            RequestResolveDisposition::LowQuality => ResolutionDisposition::LowQuality,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
pub(super) enum RequestAudienceArg {
    Public,
    Private,
}

impl From<RequestAudienceArg> for RequestAudience {
    fn from(audience: RequestAudienceArg) -> Self {
        match audience {
            RequestAudienceArg::Public => RequestAudience::Public,
            RequestAudienceArg::Private => RequestAudience::Private,
        }
    }
}
