use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Duration;

use fastrace::collector::{EventRecord, Reporter};
use fastrace::prelude::*;
use google_cloud_gax::exponential_backoff;
use google_cloud_gax::retry_policy::{self, RetryPolicyExt as _};
use google_cloud_rpc::model::Status;
pub use google_cloud_trace_v2::Error as TraceClientError;
use google_cloud_trace_v2::client::TraceService;
use google_cloud_trace_v2::model::span::time_event::Annotation;
use google_cloud_trace_v2::model::span::{Attributes, SpanKind, TimeEvent, TimeEvents};
use google_cloud_trace_v2::model::{
    AttributeValue, Span as GoogleSpan, StackTrace, TruncatableString,
};
use google_cloud_wkt::Timestamp;
use opentelemetry_semantic_conventions::attribute as attribute_sem;

fn default_tokio_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .unwrap()
}

async fn default_trace_client() -> TraceService {
    google_cloud_trace_v2::client::TraceService::builder()
        .with_retry_policy(retry_policy::Aip194Strict.with_time_limit(Duration::from_secs(120)))
        .with_backoff_policy(
            exponential_backoff::ExponentialBackoffBuilder::new()
                .with_initial_delay(Duration::from_millis(100))
                .with_maximum_delay(Duration::from_secs(30))
                .with_scaling(2)
                .build()
                .unwrap(),
        )
        .build()
        .await
        .unwrap()
}

fn default_span_kind_converter(
    _span_record: &SpanRecord,
    attribute_map: &mut HashMap<String, AttributeValue>,
) -> SpanKind {
    let span_kind = attribute_map.remove("span.kind");

    span_kind
        .as_ref()
        .and_then(|value| value.string_value())
        .map(|s| SpanKind::from(s.value.as_ref()))
        .unwrap_or(SpanKind::Internal)
}

pub struct GoogleCloudReporter {
    tokio_runtime: std::sync::LazyLock<tokio::runtime::Runtime>,
    client: TraceService,
    trace_project_id: String,
    service_name: Option<String>,
    attribute_name_mappings: Option<HashMap<&'static str, &'static str>>,
    status_converter: fn(&SpanRecord, &mut HashMap<String, AttributeValue>) -> Option<Status>,
    span_kind_converter: fn(&SpanRecord, &mut HashMap<String, AttributeValue>) -> SpanKind,
    stack_trace_converter:
        fn(&SpanRecord, &mut HashMap<String, AttributeValue>) -> Option<StackTrace>,
}

#[bon::bon]
impl GoogleCloudReporter {
    #[builder]
    pub async fn new(
        tokio_runtime: Option<fn() -> tokio::runtime::Runtime>,
        client: Option<TraceService>,
        trace_project_id: impl Into<String>,
        service_name: Option<impl Into<String>>,
        attribute_name_mappings: Option<HashMap<&'static str, &'static str>>,
        status_converter: Option<
            fn(&SpanRecord, &mut HashMap<String, AttributeValue>) -> Option<Status>,
        >,
        span_kind_converter: Option<
            fn(&SpanRecord, &mut HashMap<String, AttributeValue>) -> SpanKind,
        >,
        stack_trace_converter: Option<
            fn(&SpanRecord, &mut HashMap<String, AttributeValue>) -> Option<StackTrace>,
        >,
    ) -> Self {
        Self {
            tokio_runtime: LazyLock::new(tokio_runtime.unwrap_or(default_tokio_runtime)),
            client: client.unwrap_or(default_trace_client().await),
            trace_project_id: trace_project_id.into(),
            service_name: service_name.map(Into::into),
            attribute_name_mappings,
            status_converter: status_converter.unwrap_or(|_, _| None),
            span_kind_converter: span_kind_converter.unwrap_or(default_span_kind_converter),
            stack_trace_converter: stack_trace_converter.unwrap_or(|_, _| None),
        }
    }
}

pub fn opentelemetry_semantic_mapping() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        (attribute_sem::OTEL_COMPONENT_TYPE, "/component"),
        (attribute_sem::EXCEPTION_MESSAGE, "/error/message"),
        (attribute_sem::EXCEPTION_MESSAGE, "/error/name"),
        (
            attribute_sem::NETWORK_PROTOCOL_VERSION,
            "/http/client_protocol",
        ),
        (attribute_sem::HTTP_HOST, "/http/host"),
        (attribute_sem::HTTP_METHOD, "/http/method"),
        (attribute_sem::HTTP_REQUEST_METHOD, "/http/method"),
        // Not a standard OTEL attribute, but some existing systems have this mapping
        ("http.path", "/http/path"),
        (attribute_sem::URL_PATH, "/http/path"),
        (attribute_sem::HTTP_REQUEST_SIZE, "/http/request/size"),
        (attribute_sem::HTTP_RESPONSE_SIZE, "/http/response/size"),
        (attribute_sem::HTTP_ROUTE, "/http/route"),
        (
            attribute_sem::HTTP_RESPONSE_STATUS_CODE,
            "/http/status_code",
        ),
        (attribute_sem::HTTP_STATUS_CODE, "/http/status_code"),
        (attribute_sem::HTTP_USER_AGENT, "/http/user_agent"),
        (attribute_sem::USER_AGENT_ORIGINAL, "/http/user_agent"),
        (
            attribute_sem::K8S_CLUSTER_NAME,
            "g.co/r/k8s_container/cluster_name",
        ),
        (
            attribute_sem::K8S_NAMESPACE_NAME,
            "g.co/r/k8s_container/namespace",
        ),
        (attribute_sem::K8S_POD_NAME, "g.co/r/k8s_container/pod_name"),
        (
            attribute_sem::K8S_CONTAINER_NAME,
            "g.co/r/k8s_container/container_name",
        ),
    ])
}

impl GoogleCloudReporter {
    pub fn new_with_client(client: TraceService, trace_project_id: impl Into<String>) -> Self {
        Self {
            tokio_runtime: LazyLock::new(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_io()
                    .enable_time()
                    .build()
                    .unwrap()
            }),
            client,
            trace_project_id: trace_project_id.into(),
            service_name: None,
            attribute_name_mappings: None,
            status_converter: |_, _| None,
            span_kind_converter: |_, attribute_map| {
                let span_kind = attribute_map.remove("span.kind");

                span_kind
                    .as_ref()
                    .and_then(|value| value.string_value())
                    .map(|s| SpanKind::from(s.value.as_ref()))
                    .unwrap_or(SpanKind::Internal)
            },
            stack_trace_converter: |_, _| None,
        }
    }

    fn convert_span(&self, span: SpanRecord) -> GoogleSpan {
        let span_id = span.span_id.to_string();

        let mut attributes =
            self.convert_properties(&span.properties, self.attribute_name_mappings.as_ref());
        let status = (self.status_converter)(&span, &mut attributes.attribute_map);
        let span_kind = (self.span_kind_converter)(&span, &mut attributes.attribute_map);
        let stack_trace = (self.stack_trace_converter)(&span, &mut attributes.attribute_map);

        let mut google_span = GoogleSpan::new()
            .set_name(format!(
                "projects/{}/traces/{}/spans/{}",
                self.trace_project_id, span.trace_id, span_id
            ))
            .set_span_id(span_id)
            .set_display_name(TruncatableString::new().set_value(span.name))
            .set_start_time(convert_unix_ns(span.begin_time_unix_ns))
            .set_end_time(convert_unix_ns(span.begin_time_unix_ns + span.duration_ns))
            .set_attributes(attributes)
            .set_status(status)
            .set_span_kind(span_kind)
            .set_stack_trace(stack_trace)
            .set_time_events(
                TimeEvents::new()
                    .set_time_event(span.events.into_iter().map(|e| self.convert_event(e))),
            );

        if let Some(parent_span_id) = convert_parent_span_id(span.parent_id) {
            google_span = google_span.set_parent_span_id(parent_span_id);
        }

        google_span
    }

    fn convert_event(&self, event: EventRecord) -> TimeEvent {
        TimeEvent::new()
            .set_time(convert_unix_ns(event.timestamp_unix_ns))
            .set_annotation(
                Annotation::new()
                    .set_attributes(self.convert_properties(
                        &event.properties,
                        self.attribute_name_mappings.as_ref(),
                    ))
                    .set_description(TruncatableString::new().set_value(event.name)),
            )
    }

    fn convert_properties(
        &self,
        properties: &[(Cow<'static, str>, Cow<'static, str>)],
        attribute_name_mappings: Option<&HashMap<&'static str, &'static str>>,
    ) -> Attributes {
        let mut attributes = HashMap::with_capacity(properties.len() + 1);

        if let Some(service_name) = &self.service_name {
            attributes.insert(
                "service.name".to_string(),
                AttributeValue::new()
                    .set_string_value(TruncatableString::new().set_value(service_name)),
            );
        }

        attributes.extend(properties.iter().map(|(k, v)| {
            let key = attribute_name_mappings
                .as_ref()
                .and_then(|m| m.get(k.as_ref()).copied())
                .unwrap_or(k.as_ref());
            (
                key.to_string(),
                AttributeValue::new()
                    .set_string_value(TruncatableString::new().set_value(v.to_string())),
            )
        }));

        Attributes::new().set_attribute_map(attributes)
    }

    fn try_report(&self, spans: Vec<SpanRecord>) -> google_cloud_trace_v2::Result<()> {
        self.tokio_runtime.block_on(
            self.client
                .batch_write_spans()
                .set_name(format!("projects/{}", self.trace_project_id))
                .set_spans(spans.into_iter().map(|s| self.convert_span(s)))
                .send(),
        )
    }
}

impl Reporter for GoogleCloudReporter {
    fn report(&mut self, spans: Vec<SpanRecord>) {
        if spans.is_empty() {
            return;
        }

        if let Err(err) = self.try_report(spans) {
            log::error!("report to Google Cloud Trace failed: {err}");
        }
    }
}

fn convert_unix_ns(unix_time: u64) -> Timestamp {
    Timestamp::clamp(
        (unix_time / 1_000_000_000) as i64,
        (unix_time % 1_000_000_000) as i32,
    )
}

/// Convert a parent span ID to a string representation.
///
/// Returns `None` if the parent span ID is invalid (zero).
fn convert_parent_span_id(span_id: SpanId) -> Option<String> {
    (span_id.0 != 0).then_some(span_id.to_string())
}
