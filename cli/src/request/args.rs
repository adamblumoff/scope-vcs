use clap::{ArgGroup, Args, Parser, Subcommand, ValueEnum};
use scope_api_contract::RequestAudience;
use std::path::PathBuf;

#[derive(Parser)]
pub struct RequestArgs {
    #[command(subcommand)]
    pub(super) command: RequestCommand,
}

#[derive(Subcommand)]
pub(super) enum RequestCommand {
    #[command(about = "Start a draft request and create its local branch")]
    Start(RequestStartArgs),
    #[command(about = "Push the current commit to a request branch")]
    Push(RequestPushArgs),
    #[command(about = "Submit a request to its maintainers")]
    Submit(RequestSubmitArgs),
    #[command(about = "Close a request")]
    Close(RequestCloseArgs),
    #[command(about = "Edit a request title or description")]
    Edit(RequestEditArgs),
    #[command(about = "Invite a user to push a public request branch")]
    Invite(RequestInviteArgs),
    #[command(about = "Remove a request invitee")]
    Uninvite(RequestUninviteArgs),
    #[command(about = "Leave a request that invited you")]
    Leave(RequestLeaveArgs),
    #[command(about = "Merge a request into main")]
    Merge(RequestMergeArgs),
    #[command(about = "Rate the other terminal request participant")]
    Rate(RequestRateArgs),
    #[command(about = "Work with request discussions")]
    Discussion(RequestDiscussionArgs),
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
        help = "Public or private request audience (defaults to private for maintainers)"
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
pub(super) struct RequestSubmitArgs {
    #[command(flatten)]
    pub(super) target: RequestTargetArgs,
    #[arg(long, help = "Confirm one-way submission")]
    pub(super) yes: bool,
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
pub(super) struct RequestMergeArgs {
    #[command(flatten)]
    pub(super) target: RequestTargetArgs,
    #[arg(long, help = "Confirm the merge")]
    pub(super) yes: bool,
}

#[derive(Parser)]
pub(super) struct RequestRateArgs {
    #[command(flatten)]
    pub(super) target: RequestTargetArgs,
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=5), help = "Rating from 1 to 5")]
    pub(super) score: u8,
    #[arg(long, help = "Required reason for the rating")]
    pub(super) reason: String,
}

#[derive(Parser)]
pub(super) struct RequestDiscussionArgs {
    #[command(subcommand)]
    pub(super) command: RequestDiscussionCommand,
}

#[derive(Subcommand)]
pub(super) enum RequestDiscussionCommand {
    #[command(about = "Start a top-level discussion")]
    Start(RequestDiscussionStartArgs),
    #[command(about = "Reply to an open discussion")]
    Reply(RequestDiscussionReplyArgs),
    #[command(about = "Resolve a discussion")]
    Resolve(RequestDiscussionResolveArgs),
    #[command(about = "Reopen a discussion with a reply")]
    Reopen(RequestDiscussionReopenArgs),
}

#[derive(Args)]
#[group(id = "discussion_body", required = true, multiple = false)]
pub(super) struct RequestDiscussionBodyArgs {
    #[arg(long, help = "Literal Markdown body")]
    pub(super) body: Option<String>,
    #[arg(
        long,
        value_name = "PATH",
        help = "Read the Markdown body from a file, or - for stdin"
    )]
    pub(super) body_file: Option<PathBuf>,
}

#[derive(Parser)]
pub(super) struct RequestDiscussionStartArgs {
    #[command(flatten)]
    pub(super) target: RequestTargetArgs,
    #[command(flatten)]
    pub(super) content: RequestDiscussionBodyArgs,
    #[arg(long, value_name = "REVISION", help = "Request revision to reference")]
    pub(super) revision: Option<String>,
    #[arg(
        long,
        value_name = "OID",
        requires = "revision",
        help = "Commit within the referenced revision"
    )]
    pub(super) commit: Option<String>,
    #[arg(
        long,
        value_name = "PATH",
        requires = "commit",
        help = "File within the referenced revision"
    )]
    pub(super) path: Option<String>,
}

#[derive(Parser)]
pub(super) struct RequestDiscussionReplyArgs {
    #[arg(value_name = "DISCUSSION", help = "Discussion ID")]
    pub(super) discussion_id: String,
    #[command(flatten)]
    pub(super) target: RequestTargetArgs,
    #[command(flatten)]
    pub(super) content: RequestDiscussionBodyArgs,
}

#[derive(Parser)]
pub(super) struct RequestDiscussionResolveArgs {
    #[arg(value_name = "DISCUSSION", help = "Discussion ID")]
    pub(super) discussion_id: String,
    #[command(flatten)]
    pub(super) target: RequestTargetArgs,
}

#[derive(Parser)]
pub(super) struct RequestDiscussionReopenArgs {
    #[arg(value_name = "DISCUSSION", help = "Discussion ID")]
    pub(super) discussion_id: String,
    #[command(flatten)]
    pub(super) target: RequestTargetArgs,
    #[command(flatten)]
    pub(super) content: RequestDiscussionBodyArgs,
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
