//! Example: Send a basic span to Google Cloud Trace.
//!
//! Usage:
//!     cargo run --example simple <PROJECT_ID> <SERVICE_NAME>

use fastrace::{collector::Config, prelude::*};
use fastrace_google_cloud::GoogleCloudReporter;
use google_cloud_rpc::model::{Code, Status};
use google_cloud_trace_v2::model::span::SpanKind;
use std::{env, time::Duration};

#[fastrace::trace]
async fn child() {
    LocalSpan::add_property(|| ("/http/host", "test"));
    LocalSpan::add_property(|| ("/http/method", "GET"));
    LocalSpan::add_property(|| ("/http/path", "/"));

    tokio::time::sleep(Duration::from_secs(1)).await
}

#[tokio::main]
async fn main() {
    // Parse command-line arguments
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: {} <PROJECT_ID> <SERVICE_NAME>", args[0]);
        std::process::exit(1);
    }
    let project_id = &args[1];
    let service_name = &args[2];

    // Build the GoogleCloudReporter
    let reporter = GoogleCloudReporter::builder()
        .trace_project_id(project_id)
        .service_name(service_name)
        .span_kind_converter(|_, _| SpanKind::Client) // Pretend we're instrumenting a client
        .status_converter(|_, _| Some(Status::new().set_code(Code::Ok.value().unwrap()))) // Pretend all calls are successes
        .build()
        .await
        .expect("Could not set up Google Cloud reporter");

    fastrace::set_reporter(reporter, Config::default());

    {
        let root = Span::root("simple", SpanContext::random());
        let _guard = root.set_local_parent();
        child().await
    }

    fastrace::flush();
}
