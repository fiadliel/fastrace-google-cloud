mod opentelemetry;

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
pub use opentelemetry::opentelemetry_semantic_mapping;

fn default_tokio_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .unwrap()
}

async fn default_trace_client() -> Result<TraceService, google_cloud_gax::client_builder::Error> {
    google_cloud_trace_v2::client::TraceService::builder()
        .with_retry_policy(retry_policy::Aip194Strict.with_time_limit(Duration::from_secs(120)))
        .with_backoff_policy(
            exponential_backoff::ExponentialBackoffBuilder::new()
                .with_initial_delay(Duration::from_millis(100))
                .with_maximum_delay(Duration::from_secs(30))
                .with_scaling(2)
                .build()
                .expect("Invalid scaling parameters set for default trace service client"),
        )
        .build()
        .await
}

fn default_span_kind_converter(
    _span_record: &SpanRecord,
    attribute_map: &mut HashMap<String, AttributeValue>,
) -> SpanKind {
    let span_kind = attribute_map.remove("span.kind");

    span_kind
        .as_ref()
        .and_then(|value| value.string_value())
        .map(|s| SpanKind::from(&*s.value))
        .unwrap_or(SpanKind::Internal)
}

#[derive(bon::Builder)]
#[builder(finish_fn(vis = "", name = build_internal))]
pub struct GoogleCloudReporter {
    #[builder(default = LazyLock::new(default_tokio_runtime), with = |s: fn() -> tokio::runtime::Runtime| LazyLock::new(s))]
    tokio_runtime: std::sync::LazyLock<tokio::runtime::Runtime>,
    trace_client: Option<TraceService>,
    #[builder(into)]
    trace_project_id: String,
    #[builder(into)]
    service_name: Option<String>,
    attribute_name_mappings: Option<HashMap<&'static str, &'static str>>,
    #[builder(default = |_, _| None)]
    status_converter: fn(&SpanRecord, &mut HashMap<String, AttributeValue>) -> Option<Status>,
    #[builder(default = default_span_kind_converter)]
    span_kind_converter: fn(&SpanRecord, &mut HashMap<String, AttributeValue>) -> SpanKind,
    #[builder(default = |_, _| None)]
    stack_trace_converter:
        fn(&SpanRecord, &mut HashMap<String, AttributeValue>) -> Option<StackTrace>,
}

impl<S: google_cloud_reporter_builder::IsComplete> GoogleCloudReporterBuilder<S> {
    pub async fn build(
        self,
    ) -> Result<GoogleCloudReporter, google_cloud_gax::client_builder::Error> {
        let mut reporter = self.build_internal();

        if reporter.trace_client.is_none() {
            reporter.trace_client = Some(default_trace_client().await?)
        }

        Ok(reporter)
    }
}

impl GoogleCloudReporter {
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
            .set_or_clear_status(status)
            .set_span_kind(span_kind)
            .set_or_clear_stack_trace(stack_trace)
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
            self.trace_client
                .as_ref()
                .unwrap()
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
