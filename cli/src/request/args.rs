use clap::{ArgGroup, Args, Parser, Subcommand, ValueEnum};
use scope_core::domain::requests::{RequestAssessmentOutcome, RequestAudience};
use std::path::PathBuf;

#[derive(Parser)]
pub struct RequestArgs {
    #[command(subcommand)]
    pub(super) command: RequestCommand,
}

#[derive(Subcommand)]
pub(super) enum RequestCommand {
    #[command(about = "Start a Working request and create its local branch")]
    Start(RequestStartArgs),
    #[command(about = "Push the current commit to a request branch")]
    Push(RequestPushArgs),
    #[command(about = "Publish a request as ready for review")]
    Ready(RequestReadyArgs),
    #[command(about = "Return a request to Working")]
    Working(RequestWorkingArgs),
    #[command(about = "Close a Working request")]
    Close(RequestCloseArgs),
    #[command(about = "Edit a request title or description")]
    Edit(RequestEditArgs),
    #[command(about = "Invite a user to push a public request branch")]
    Invite(RequestInviteArgs),
    #[command(about = "Remove a request invitee")]
    Uninvite(RequestUninviteArgs),
    #[command(about = "Leave a request that invited you")]
    Leave(RequestLeaveArgs),
    #[command(about = "Place a ready request on hold")]
    Hold(RequestHoldArgs),
    #[command(about = "Release a request review hold")]
    Unhold(RequestUnholdArgs),
    #[command(about = "Return a request to Working for changes")]
    RequestChanges(RequestChangesArgs),
    #[command(about = "Complete a request with a review assessment")]
    Assess(RequestAssessArgs),
    #[command(about = "Merge a request into main")]
    Merge(RequestMergeArgs),
    #[command(about = "Start a top-level discussion on a request")]
    Discuss(RequestDiscussArgs),
    #[command(about = "Show one request")]
    Show(RequestShowArgs),
    #[command(about = "List visible requests")]
    List(RequestListArgs),
    #[command(about = "Show the current request or repository request status")]
    Status(RequestStatusArgs),
}

#[derive(Parser)]
pub(super) struct RequestStartArgs {
    #[arg(help = "Stable kebab-case request name used as the Git branch")]
    pub(super) name: String,
    #[arg(long, help = "Scope Git remote for the target repository")]
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

#[derive(Args)]
pub(super) struct RequestTargetArgs {
    #[arg(long, help = "Scope Git remote for the target repository")]
    pub(super) remote: Option<String>,
    #[arg(
        long,
        value_name = "REQUEST",
        help = "Request name or req_ ID (defaults to the current branch or request ref)"
    )]
    pub(super) request: Option<String>,
}

#[derive(Parser)]
pub(super) struct RequestPushArgs {
    #[command(flatten)]
    pub(super) target: RequestTargetArgs,
}

#[derive(Parser)]
pub(super) struct RequestReadyArgs {
    #[command(flatten)]
    pub(super) target: RequestTargetArgs,
    #[arg(long, value_name = "CREDITS", help = "Credits to stake on review")]
    pub(super) stake: u32,
    #[arg(long, help = "Confirm publication and the credit stake")]
    pub(super) yes: bool,
}

#[derive(Parser)]
pub(super) struct RequestWorkingArgs {
    #[command(flatten)]
    pub(super) target: RequestTargetArgs,
}

#[derive(Parser)]
pub(super) struct RequestCloseArgs {
    #[command(flatten)]
    pub(super) target: RequestTargetArgs,
    #[arg(long, help = "Confirm closing the request")]
    pub(super) yes: bool,
}

#[derive(Parser)]
#[command(group(
    ArgGroup::new("request_edit")
        .required(true)
        .multiple(true)
        .args(["title", "description_file"])
))]
pub(super) struct RequestEditArgs {
    #[command(flatten)]
    pub(super) target: RequestTargetArgs,
    #[arg(long, value_name = "TITLE", help = "New display title")]
    pub(super) title: Option<String>,
    #[arg(
        long,
        value_name = "PATH",
        help = "Read the new Markdown description from this file"
    )]
    pub(super) description_file: Option<PathBuf>,
}

#[derive(Parser)]
pub(super) struct RequestInviteArgs {
    #[command(flatten)]
    pub(super) target: RequestTargetArgs,
    #[arg(value_name = "HANDLE", help = "Exact Scope handle to invite")]
    pub(super) handle: String,
}

#[derive(Parser)]
pub(super) struct RequestUninviteArgs {
    #[command(flatten)]
    pub(super) target: RequestTargetArgs,
    #[arg(value_name = "HANDLE", help = "Exact Scope handle to remove")]
    pub(super) handle: String,
}

#[derive(Parser)]
pub(super) struct RequestLeaveArgs {
    #[command(flatten)]
    pub(super) target: RequestTargetArgs,
}

#[derive(Parser)]
pub(super) struct RequestHoldArgs {
    #[command(flatten)]
    pub(super) target: RequestTargetArgs,
}

#[derive(Parser)]
pub(super) struct RequestUnholdArgs {
    #[command(flatten)]
    pub(super) target: RequestTargetArgs,
}

#[derive(Parser)]
pub(super) struct RequestChangesArgs {
    #[command(flatten)]
    pub(super) target: RequestTargetArgs,
}

#[derive(Parser)]
pub(super) struct RequestAssessArgs {
    #[command(flatten)]
    pub(super) target: RequestTargetArgs,
    #[arg(value_enum)]
    pub(super) outcome: RequestAssessmentArg,
    #[arg(
        required_if_eq("outcome", "rejected"),
        long,
        value_name = "MARKDOWN",
        help = "Assessment message (required when rejecting)"
    )]
    pub(super) message: Option<String>,
    #[arg(long, help = "Confirm completion and credit settlement")]
    pub(super) yes: bool,
}

#[derive(Parser)]
pub(super) struct RequestMergeArgs {
    #[command(flatten)]
    pub(super) target: RequestTargetArgs,
    #[arg(long, help = "Confirm the merge and any resulting assessment")]
    pub(super) yes: bool,
}

#[derive(Parser)]
pub(super) struct RequestDiscussArgs {
    #[command(flatten)]
    pub(super) target: RequestTargetArgs,
    #[arg(long, help = "Markdown body for the new discussion")]
    pub(super) body: String,
}

#[derive(Parser)]
pub(super) struct RequestShowArgs {
    #[command(flatten)]
    pub(super) target: RequestTargetArgs,
}

#[derive(Parser)]
pub(super) struct RequestListArgs {
    #[arg(long, help = "Scope Git remote for the target repository")]
    pub(super) remote: Option<String>,
}

#[derive(Parser)]
pub(super) struct RequestStatusArgs {
    #[command(flatten)]
    pub(super) target: RequestTargetArgs,
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

#[derive(Clone, Copy, ValueEnum)]
pub(super) enum RequestAssessmentArg {
    Accepted,
    Neutral,
    Rejected,
}

impl From<RequestAssessmentArg> for RequestAssessmentOutcome {
    fn from(outcome: RequestAssessmentArg) -> Self {
        match outcome {
            RequestAssessmentArg::Accepted => RequestAssessmentOutcome::Accepted,
            RequestAssessmentArg::Neutral => RequestAssessmentOutcome::Neutral,
            RequestAssessmentArg::Rejected => RequestAssessmentOutcome::Rejected,
        }
    }
}
