use crate::config::non_empty_env;
use posthog_rs::{ClientOptionsBuilder, Event};
use scope_domain::requests::{RequestActorRole, RequestAudience};
use serde_json::{Map, Value};
use std::{future::Future, pin::Pin, sync::Arc};

const POSTHOG_PROJECT_TOKEN_ENV: &str = "POSTHOG_PROJECT_TOKEN";
const POSTHOG_HOST_ENV: &str = "POSTHOG_HOST";
const PROCESS_PERSON_PROFILE_PROPERTY: &str = "$process_person_profile";
const SCOPE_USER_ID_PREFIX: &str = "scope_usr_";

#[derive(Clone)]
pub(crate) struct ProductAnalytics {
    sink: Arc<dyn ProductAnalyticsSink>,
}

impl ProductAnalytics {
    pub(crate) async fn from_env() -> anyhow::Result<Self> {
        let Some(project_token) = non_empty_env(POSTHOG_PROJECT_TOKEN_ENV) else {
            return Ok(Self::disabled());
        };

        let mut options = ClientOptionsBuilder::default();
        options
            .api_key(project_token)
            .disable_geoip(true)
            .is_server(true);
        if let Some(host) = non_empty_env(POSTHOG_HOST_ENV) {
            options.host(host);
        }
        options.on_error(|error| {
            tracing::warn!(error = ?error, "PostHog product analytics delivery failed");
        });
        let client = posthog_rs::client(options.build()?).await;

        Ok(Self {
            sink: Arc::new(PostHogSink { client }),
        })
    }

    pub(crate) fn disabled() -> Self {
        Self {
            sink: Arc::new(DisabledSink),
        }
    }

    pub(crate) fn capture(&self, event: ProductEvent) {
        if !event.distinct_id.starts_with(SCOPE_USER_ID_PREFIX) {
            tracing::warn!(
                event = event.name,
                "dropped product analytics event with a non-Scope distinct ID"
            );
            return;
        }
        if let Err(error) = self.sink.capture(event) {
            tracing::warn!(error = %error, "product analytics capture failed");
        }
    }

    pub(crate) async fn shutdown(&self) {
        self.sink.shutdown().await;
    }

    #[cfg(test)]
    pub(crate) fn recording() -> (Self, RecordingProductAnalytics) {
        let recording = RecordingProductAnalytics::default();
        (
            Self {
                sink: Arc::new(recording.clone()),
            },
            recording,
        )
    }
}

trait ProductAnalyticsSink: Send + Sync {
    fn capture(&self, event: ProductEvent) -> Result<(), ProductAnalyticsError>;

    fn shutdown(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(std::future::ready(()))
    }
}

struct DisabledSink;

impl ProductAnalyticsSink for DisabledSink {
    fn capture(&self, _event: ProductEvent) -> Result<(), ProductAnalyticsError> {
        Ok(())
    }
}

struct PostHogSink {
    client: posthog_rs::Client,
}

impl ProductAnalyticsSink for PostHogSink {
    fn capture(&self, event: ProductEvent) -> Result<(), ProductAnalyticsError> {
        let mut posthog_event = Event::new(event.name, event.distinct_id.as_str());
        posthog_event
            .insert_prop(PROCESS_PERSON_PROFILE_PROPERTY, false)
            .map_err(ProductAnalyticsError::posthog)?;
        for (name, value) in event.properties {
            posthog_event
                .insert_prop(name, value)
                .map_err(ProductAnalyticsError::posthog)?;
        }
        self.client.capture(posthog_event);
        Ok(())
    }

    fn shutdown(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(self.client.shutdown())
    }
}

#[derive(Debug)]
struct ProductAnalyticsError(String);

impl ProductAnalyticsError {
    fn posthog(error: posthog_rs::Error) -> Self {
        Self(error.to_string())
    }
}

impl std::fmt::Display for ProductAnalyticsError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ProductEvent {
    name: &'static str,
    distinct_id: String,
    properties: Map<String, Value>,
}

impl ProductEvent {
    pub(crate) fn account_created(actor_user_id: &str) -> Self {
        Self::new("account:user_create", actor_user_id)
    }

    pub(crate) fn repository_initialized(actor_user_id: &str) -> Self {
        Self::new("repository:repository_initialize", actor_user_id)
    }

    pub(crate) fn cli_session_created(actor_user_id: &str, method: CliSessionMethod) -> Self {
        let mut event = Self::new("cli:session_create", actor_user_id);
        event.properties.insert(
            "authentication_method".to_string(),
            Value::String(method.as_str().to_string()),
        );
        event
    }

    pub(crate) fn request_started(
        actor_user_id: &str,
        audience: RequestAudience,
        actor_role: RequestActorRole,
    ) -> Self {
        Self::request_event("request:request_start", actor_user_id, audience, actor_role)
    }

    pub(crate) fn request_submitted(
        actor_user_id: &str,
        audience: RequestAudience,
        actor_role: RequestActorRole,
    ) -> Self {
        Self::request_event(
            "request:request_submit",
            actor_user_id,
            audience,
            actor_role,
        )
    }

    pub(crate) fn request_merged(
        actor_user_id: &str,
        audience: RequestAudience,
        actor_role: RequestActorRole,
    ) -> Self {
        Self::request_event("request:request_merge", actor_user_id, audience, actor_role)
    }

    pub(crate) fn request_revised(actor_user_id: &str, audience: RequestAudience) -> Self {
        let mut event = Self::new("request:revision_create", actor_user_id);
        event.insert_request_audience(audience);
        event
    }

    pub(crate) fn request_rated(actor_user_id: &str, audience: RequestAudience, score: u8) -> Self {
        let mut event = Self::new("request:rating_create", actor_user_id);
        event.insert_request_audience(audience);
        event
            .properties
            .insert("score".to_string(), Value::Number(score.into()));
        event
    }

    pub(crate) fn discussion_created(
        actor_user_id: &str,
        audience: RequestAudience,
        actor_role: RequestActorRole,
        anchored: bool,
    ) -> Self {
        let mut event = Self::request_event(
            "discussion:discussion_create",
            actor_user_id,
            audience,
            actor_role,
        );
        event
            .properties
            .insert("anchored".to_string(), Value::Bool(anchored));
        event
    }

    pub(crate) fn discussion_resolved(
        actor_user_id: &str,
        audience: RequestAudience,
        actor_role: RequestActorRole,
    ) -> Self {
        Self::request_event(
            "discussion:discussion_resolve",
            actor_user_id,
            audience,
            actor_role,
        )
    }

    pub(crate) fn request_closed(
        actor_user_id: &str,
        audience: RequestAudience,
        actor_role: RequestActorRole,
        outcome: RequestCloseOutcome,
    ) -> Self {
        let mut event =
            Self::request_event("request:request_close", actor_user_id, audience, actor_role);
        event.properties.insert(
            "outcome".to_string(),
            Value::String(outcome.as_str().to_string()),
        );
        event
    }

    fn request_event(
        name: &'static str,
        actor_user_id: &str,
        audience: RequestAudience,
        actor_role: RequestActorRole,
    ) -> Self {
        let mut event = Self::new(name, actor_user_id);
        event.insert_request_audience(audience);
        event.properties.insert(
            "actor_role".to_string(),
            Value::String(request_actor_role_name(actor_role).to_string()),
        );
        event
    }

    fn insert_request_audience(&mut self, audience: RequestAudience) {
        self.properties.insert(
            "request_audience".to_string(),
            Value::String(request_audience_name(audience).to_string()),
        );
    }

    fn new(name: &'static str, distinct_id: &str) -> Self {
        Self {
            name,
            distinct_id: distinct_id.to_string(),
            properties: Map::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CliSessionMethod {
    Browser,
    Device,
    ExchangeGrant,
}

impl CliSessionMethod {
    fn as_str(self) -> &'static str {
        match self {
            Self::Browser => "browser",
            Self::Device => "device",
            Self::ExchangeGrant => "exchange_grant",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RequestCloseOutcome {
    Closed,
    DraftDeleted,
}

impl RequestCloseOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Closed => "closed",
            Self::DraftDeleted => "draft_deleted",
        }
    }
}

fn request_audience_name(audience: RequestAudience) -> &'static str {
    match audience {
        RequestAudience::Public => "public",
        RequestAudience::Private => "private",
    }
}

fn request_actor_role_name(role: RequestActorRole) -> &'static str {
    match role {
        RequestActorRole::Public => "public",
        RequestActorRole::Member => "member",
        RequestActorRole::Owner => "owner",
    }
}

#[cfg(test)]
#[derive(Clone, Default)]
pub(crate) struct RecordingProductAnalytics {
    events: Arc<std::sync::Mutex<Vec<ProductEvent>>>,
}

#[cfg(test)]
impl RecordingProductAnalytics {
    pub(crate) fn events(&self) -> Vec<ProductEvent> {
        self.events.lock().unwrap().clone()
    }

    pub(crate) fn event_names(&self) -> Vec<&'static str> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .map(|event| event.name)
            .collect()
    }
}

#[cfg(test)]
impl ProductAnalyticsSink for RecordingProductAnalytics {
    fn capture(&self, event: ProductEvent) -> Result<(), ProductAnalyticsError> {
        self.events.lock().unwrap().push(event);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_event_contract_contains_only_safe_static_properties() {
        let (analytics, recording) = ProductAnalytics::recording();
        analytics.capture(ProductEvent::request_started(
            "scope_usr_test",
            RequestAudience::Private,
            RequestActorRole::Owner,
        ));

        assert_eq!(
            recording.events(),
            vec![ProductEvent {
                name: "request:request_start",
                distinct_id: "scope_usr_test".to_string(),
                properties: Map::from_iter([
                    ("actor_role".to_string(), Value::String("owner".to_string())),
                    (
                        "request_audience".to_string(),
                        Value::String("private".to_string()),
                    ),
                ]),
            }]
        );
    }

    #[test]
    fn non_scope_distinct_ids_are_rejected_before_the_sink() {
        let (analytics, recording) = ProductAnalytics::recording();
        analytics.capture(ProductEvent::repository_initialized("clerk_user_secret"));

        assert!(recording.events().is_empty());
    }
}
