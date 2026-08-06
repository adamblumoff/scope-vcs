use crate::api::{
    LeaveRequestResponse, RepoSummaryResponse, RequestActivityPageResponse, RequestCloseResponse,
    RequestDiscussionReplyResponse, RequestDiscussionSummaryResponse,
    RequestInviteeMutationResponse, RequestListItemResponse, RequestMutationResponse,
    RequestRatingResponse, RequestSummaryResponse,
};
use anyhow::Context;
use scope_api_contract::CliSuccessEnvelope;
use serde::Serialize;

pub struct RequestCommandOutcome {
    command: &'static str,
    result: RequestCommandResult,
    human_lines: Vec<String>,
}

impl RequestCommandOutcome {
    pub(super) fn new(
        command: &'static str,
        result: RequestCommandResult,
        human_lines: Vec<String>,
    ) -> Self {
        Self {
            command,
            result,
            human_lines,
        }
    }

    pub fn render(self, json: bool) -> anyhow::Result<()> {
        if json {
            let envelope = CliSuccessEnvelope::new(self.command, self.result.json_value()?);
            println!(
                "{}",
                serde_json::to_string(&envelope).context("serialize request command result")?
            );
        } else if !self.human_lines.is_empty() {
            println!("{}", self.human_lines.join("\n"));
        }
        Ok(())
    }
}

#[derive(Serialize)]
#[serde(untagged)]
pub(super) enum RequestCommandResult {
    Started(StartResult),
    Detail(DetailResult),
    List(ListResult),
    Mutation(RepoResponse<RequestMutationResponse>),
    Invitee(RepoResponse<RequestInviteeMutationResponse>),
    Leave(TargetResponse<LeaveRequestResponse>),
    Close(TargetResponse<RequestCloseResponse>),
    Discussion(DiscussionResult),
    DiscussionReply(DiscussionReplyResult),
    Rating(TargetResponse<RequestRatingResponse>),
}

impl RequestCommandResult {
    fn json_value(&self) -> anyhow::Result<serde_json::Value> {
        serde_json::to_value(self).context("serialize typed request result")
    }
}

#[derive(Serialize)]
pub(super) struct StartResult {
    pub(super) repo: RepoSummaryResponse,
    pub(super) request: RequestSummaryResponse,
    pub(super) branch: String,
    pub(super) base_oid: String,
    pub(super) remote: String,
}

#[derive(Serialize)]
pub(super) struct DetailResult {
    pub(super) repo: RepoSummaryResponse,
    pub(super) request: RequestSummaryResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) activity: Option<RequestActivityPageResponse>,
}

#[derive(Serialize)]
pub(super) struct ListResult {
    pub(super) repo: RepoSummaryResponse,
    pub(super) requests: Vec<RequestListItemResponse>,
}

#[derive(Serialize)]
pub(super) struct RepoResponse<T> {
    pub(super) repo: RepoSummaryResponse,
    pub(super) response: T,
}

#[derive(Serialize)]
pub(super) struct TargetResponse<T> {
    pub(super) repo: RepoSummaryResponse,
    pub(super) request_id: String,
    pub(super) response: T,
}

#[derive(Serialize)]
pub(super) struct DiscussionResult {
    pub(super) repo: RepoSummaryResponse,
    pub(super) request_id: String,
    pub(super) discussion: RequestDiscussionSummaryResponse,
}

#[derive(Serialize)]
pub(super) struct DiscussionReplyResult {
    pub(super) repo: RepoSummaryResponse,
    pub(super) request_id: String,
    pub(super) discussion: RequestDiscussionSummaryResponse,
    pub(super) reply: RequestDiscussionReplyResponse,
}
