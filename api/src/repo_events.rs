pub(crate) use scope_api_contract::{RepoChangeEvent, RepoChangeKind};
use scope_domain::requests::RequestAudience as DomainRequestAudience;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::broadcast;

const REPO_CHANGE_CHANNEL_CAPACITY: usize = 128;
const REQUEST_SUMMARY_REFRESH_VERSION: u64 = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RepoChangeReason {
    Connected,
    Lagged,
    RepoDeleted,
    ConfigApplied,
    RequestMerged,
    RequestDeleted,
    RequestClosed,
    RequestStarted,
    RequestIdentityEdited,
    RequestInviteeLeft,
    RequestRevised,
    MemberAdded,
    InviteUpdated,
    MemberPermissionsChanged,
    InviteRevoked,
    MemberRemoved,
    FirstPushApplied,
    PushReceived,
    RequestSubmitted,
    RequestInviteeAdded,
    RequestInviteeRemoved,
    #[cfg(test)]
    VisibilityChanged,
}

impl RepoChangeReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::Lagged => "lagged",
            Self::RepoDeleted => "repo-deleted",
            Self::ConfigApplied => "config-applied",
            Self::RequestMerged => "request-merged",
            Self::RequestDeleted => "request-deleted",
            Self::RequestClosed => "request-closed",
            Self::RequestStarted => "request-started",
            Self::RequestIdentityEdited => "request-identity-edited",
            Self::RequestInviteeLeft => "request-invitee-left",
            Self::RequestRevised => "request-revised",
            Self::MemberAdded => "member-added",
            Self::InviteUpdated => "invite-updated",
            Self::MemberPermissionsChanged => "member-permissions-changed",
            Self::InviteRevoked => "invite-revoked",
            Self::MemberRemoved => "member-removed",
            Self::FirstPushApplied => "first-push-applied",
            Self::PushReceived => "push-received",
            Self::RequestSubmitted => "request-submitted",
            Self::RequestInviteeAdded => "request-invitee-added",
            Self::RequestInviteeRemoved => "request-invitee-removed",
            #[cfg(test)]
            Self::VisibilityChanged => "visibility-changed",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct RepoChangeNotification {
    event: RepoChangeEvent,
    origin_id: String,
}

#[derive(Clone, Debug)]
pub(crate) struct RepoChangeBus {
    channels: Arc<Mutex<BTreeMap<String, broadcast::Sender<RepoChangeEvent>>>>,
    origin_id: Arc<str>,
}

impl Default for RepoChangeBus {
    fn default() -> Self {
        Self {
            channels: Arc::new(Mutex::new(BTreeMap::new())),
            origin_id: Arc::from(new_origin_id()),
        }
    }
}

impl RepoChangeBus {
    pub(crate) fn origin_id(&self) -> &str {
        &self.origin_id
    }

    pub(crate) fn subscribe(&self, repo_id: &str) -> broadcast::Receiver<RepoChangeEvent> {
        let mut channels = self
            .channels
            .lock()
            .expect("repo change bus lock must not be poisoned");
        let sender = channels
            .entry(repo_id.to_string())
            .or_insert_with(|| broadcast::channel(REPO_CHANGE_CHANNEL_CAPACITY).0);
        sender.subscribe()
    }

    pub(crate) fn remove_if_idle(&self, repo_id: &str) {
        let mut channels = self
            .channels
            .lock()
            .expect("repo change bus lock must not be poisoned");
        if channels
            .get(repo_id)
            .is_some_and(|sender| sender.receiver_count() == 0)
        {
            channels.remove(repo_id);
        }
    }

    pub(crate) fn publish_event(&self, event: RepoChangeEvent) {
        let mut channels = self
            .channels
            .lock()
            .expect("repo change bus lock must not be poisoned");
        let Some(sender) = channels.get(&event.repo_id).cloned() else {
            return;
        };
        if sender.receiver_count() == 0 || sender.send(event.clone()).is_err() {
            channels.remove(&event.repo_id);
        }
    }

    pub(crate) fn notification_payload(
        &self,
        event: &RepoChangeEvent,
    ) -> Result<String, serde_json::Error> {
        serde_json::to_string(&RepoChangeNotification {
            event: event.clone(),
            origin_id: self.origin_id().to_string(),
        })
    }

    pub(crate) fn publish_notification_payload(&self, payload: &str) {
        match serde_json::from_str::<RepoChangeNotification>(payload) {
            Ok(notification) if notification.origin_id != self.origin_id() => {
                self.publish_event(notification.event);
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(
                %error,
                payload,
                "ignored malformed repo change notification"
            ),
        }
    }
}

pub(crate) fn repository_change_event(
    repo_id: &str,
    version: u64,
    reason: RepoChangeReason,
) -> RepoChangeEvent {
    RepoChangeEvent {
        repo_id: repo_id.to_string(),
        version,
        kind: match reason {
            RepoChangeReason::Connected => RepoChangeKind::Connected,
            RepoChangeReason::Lagged => RepoChangeKind::Lagged,
            reason => RepoChangeKind::RepositoryChanged {
                reason: reason.as_str().to_string(),
            },
        },
    }
}

pub(crate) fn request_timeline_change_event(
    repo_id: &str,
    request_id: String,
    discussion_id: String,
    through_position: u64,
    audience: DomainRequestAudience,
) -> RepoChangeEvent {
    RepoChangeEvent {
        repo_id: repo_id.to_string(),
        version: 0,
        kind: RepoChangeKind::RequestTimelineChanged {
            request_id,
            discussion_id,
            through_position,
            audience: audience.into(),
        },
    }
}

impl crate::state::AppState {
    pub(crate) async fn publish_repo_change(
        &self,
        repo_id: &str,
        version: u64,
        reason: RepoChangeReason,
    ) {
        let event = repository_change_event(repo_id, version, reason);
        self.repo_events.publish_event(event.clone());
        let payload = match self.repo_events.notification_payload(&event) {
            Ok(payload) => payload,
            Err(error) => {
                tracing::warn!(repo_id, error = %error, "failed to serialize repo change notification");
                return;
            }
        };
        if let Err(error) = self
            .metadata
            .repositories()
            .notify_repo_change(&payload)
            .await
        {
            tracing::warn!(
                repo_id,
                version,
                reason = reason.as_str(),
                error = %error.message,
                "failed to publish repo change notification"
            );
        }
    }

    pub(crate) async fn publish_request_summary_refresh(
        &self,
        repo_id: &str,
        reason: RepoChangeReason,
    ) {
        self.publish_repo_change(repo_id, REQUEST_SUMMARY_REFRESH_VERSION, reason)
            .await;
    }

    pub(crate) async fn publish_request_timeline_change(
        &self,
        repo_id: &str,
        request_id: String,
        discussion_id: String,
        through_position: u64,
        audience: scope_domain::requests::RequestAudience,
    ) {
        let event = request_timeline_change_event(
            repo_id,
            request_id,
            discussion_id,
            through_position,
            audience,
        );
        self.repo_events.publish_event(event.clone());
        let payload = match self.repo_events.notification_payload(&event) {
            Ok(payload) => payload,
            Err(error) => {
                tracing::warn!(repo_id, error = %error, "failed to serialize repo change notification");
                return;
            }
        };
        if let Err(error) = self
            .metadata
            .repositories()
            .notify_repo_change(&payload)
            .await
        {
            tracing::warn!(
                repo_id,
                through_position,
                error = %error.message,
                "failed to publish request discussion notification"
            );
        }
    }
}

fn new_origin_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{}-{nanos}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(repo_id: &str) -> RepoChangeEvent {
        repository_change_event(repo_id, 4, RepoChangeReason::PushReceived)
    }

    #[test]
    fn typed_reasons_preserve_special_kinds_and_wire_reason() {
        assert_eq!(
            repository_change_event("repo", 1, RepoChangeReason::Connected).kind,
            RepoChangeKind::Connected
        );
        assert_eq!(
            repository_change_event("repo", 1, RepoChangeReason::Lagged).kind,
            RepoChangeKind::Lagged
        );
        assert_eq!(
            repository_change_event("repo", 1, RepoChangeReason::ConfigApplied).kind,
            RepoChangeKind::RepositoryChanged {
                reason: "config-applied".to_string(),
            }
        );
    }

    #[test]
    fn notification_payload_suppresses_same_origin_and_forwards_other_origins() {
        let bus = RepoChangeBus::default();
        let mut receiver = bus.subscribe("repo");

        let local_payload = bus.notification_payload(&event("repo")).unwrap();
        bus.publish_notification_payload(&local_payload);
        assert!(matches!(
            receiver.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));

        let external_payload = serde_json::to_string(&RepoChangeNotification {
            event: event("repo"),
            origin_id: "another-process".to_string(),
        })
        .unwrap();
        bus.publish_notification_payload(&external_payload);
        assert_eq!(receiver.try_recv().unwrap(), event("repo"));
    }

    #[test]
    fn malformed_notifications_are_dropped() {
        let bus = RepoChangeBus::default();
        let mut receiver = bus.subscribe("repo");

        bus.publish_notification_payload("not-json");

        assert!(matches!(
            receiver.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn idle_channels_are_removed_and_recreated_on_subscribe() {
        let bus = RepoChangeBus::default();
        let receiver = bus.subscribe("repo");
        drop(receiver);
        bus.remove_if_idle("repo");

        let mut replacement = bus.subscribe("repo");
        bus.publish_event(event("repo"));

        assert_eq!(replacement.try_recv().unwrap(), event("repo"));
    }
}
