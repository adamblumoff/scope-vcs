use clap::{Parser, Subcommand, ValueEnum};
use scope_core::domain::requests::ResolutionDisposition;

#[derive(Parser)]
pub struct RequestArgs {
    #[command(subcommand)]
    pub(super) command: RequestCommand,
}

#[derive(Subcommand)]
pub(super) enum RequestCommand {
    Start(RequestStartArgs),
    Join(RequestJoinArgs),
    Submit(RequestSubmitArgs),
    Pull(RequestPullArgs),
    Push(RequestPushArgs),
    SyncMain(RequestSyncMainArgs),
    Delete(RequestDeleteArgs),
    Share(RequestShareArgs),
    Status(RequestStatusArgs),
    Comment(RequestCommentArgs),
    NeedsResponse(RequestNeedsResponseArgs),
    Respond(RequestRespondArgs),
    Resolve(RequestResolveArgs),
    Merge(RequestMergeArgs),
}

#[derive(Parser)]
pub(super) struct RequestStartArgs {
    #[arg(long)]
    pub(super) remote: Option<String>,
    #[arg(long)]
    pub(super) branch: Option<String>,
    #[arg(long)]
    pub(super) title: String,
}

#[derive(Parser)]
pub(super) struct RequestJoinArgs {
    #[arg(long)]
    pub(super) remote: Option<String>,
    pub(super) id: String,
}

#[derive(Parser)]
pub(super) struct RequestSubmitArgs {
    #[arg(long)]
    pub(super) remote: Option<String>,
    #[arg(long)]
    pub(super) stake_credits: Option<u32>,
}

#[derive(Parser)]
pub(super) struct RequestPullArgs {
    #[arg(long)]
    pub(super) remote: Option<String>,
    pub(super) id: Option<String>,
}

#[derive(Parser)]
pub(super) struct RequestPushArgs {
    #[arg(long)]
    pub(super) remote: Option<String>,
    pub(super) id: Option<String>,
}

#[derive(Parser)]
pub(super) struct RequestSyncMainArgs {
    #[arg(long)]
    pub(super) remote: Option<String>,
}

#[derive(Parser)]
pub(super) struct RequestDeleteArgs {
    #[arg(long)]
    pub(super) remote: Option<String>,
    pub(super) id: Option<String>,
}

#[derive(Parser)]
pub(super) struct RequestShareArgs {
    #[arg(long)]
    pub(super) remote: Option<String>,
    pub(super) id: Option<String>,
}

#[derive(Parser)]
pub(super) struct RequestStatusArgs {
    #[arg(long)]
    pub(super) remote: Option<String>,
    pub(super) id: Option<String>,
}

#[derive(Parser)]
pub(super) struct RequestCommentArgs {
    #[arg(long)]
    pub(super) remote: Option<String>,
    pub(super) id: Option<String>,
    #[arg(long)]
    pub(super) body: String,
}

#[derive(Parser)]
pub(super) struct RequestNeedsResponseArgs {
    #[arg(long)]
    pub(super) remote: Option<String>,
    pub(super) id: Option<String>,
    #[arg(long)]
    pub(super) body: String,
}

#[derive(Parser)]
pub(super) struct RequestRespondArgs {
    #[arg(long)]
    pub(super) remote: Option<String>,
    pub(super) id: Option<String>,
    #[arg(long)]
    pub(super) body: Option<String>,
}

#[derive(Parser)]
pub(super) struct RequestResolveArgs {
    #[arg(long)]
    pub(super) remote: Option<String>,
    pub(super) id: Option<String>,
    #[arg(long, value_enum)]
    pub(super) disposition: RequestResolveDisposition,
    #[arg(long)]
    pub(super) body: Option<String>,
}

#[derive(Parser)]
pub(super) struct RequestMergeArgs {
    #[arg(long)]
    pub(super) remote: Option<String>,
    pub(super) id: Option<String>,
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
