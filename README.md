# fastrace-google-cloud

A high-performance, async-first reporter for [fastrace](https://github.com/fast/fastrace) that sends distributed tracing data to [Google Cloud Trace](https://cloud.google.com/trace).

## Overview

`fastrace-google-cloud` provides a flexible and extensible way to export spans and events from Rust applications using the `fastrace` distributed tracing library to Google Cloud Trace. It supports custom attribute mappings, span kind conversion, status conversion, and stack trace conversion, making it suitable for a wide range of cloud-native and microservice architectures.

## Features

- **Distributed tracing**: Integrates seamlessly with `fastrace` to capture and propagate trace context across async boundaries and threads.
- **Google Cloud Trace reporting**: Batch reports spans and events to Google Cloud Trace using the official client.
- **Customizable**: Supports custom attribute mappings and converters for span kind, status, and stack trace.
- **Async and high-throughput**: Uses Tokio for efficient, non-blocking reporting.
- **Extensible**: Easily add custom logic for mapping, conversion, and error handling.

## Installation

Add the following to your `Cargo.toml`:

```toml
[dependencies]
fastrace-google-cloud = "0.1"
fastrace = { version = "0.7", features = ["enable"] }
```

## Usage

Here's a minimal example of reporting a span to Google Cloud Trace:

```rust
use fastrace::{collector::Config, prelude::*};
use fastrace_google_cloud::GoogleCloudReporter;

#[tokio::main]
async fn main() {
    // Build the reporter with your GCP project ID and optional service name
    let mut reporter = GoogleCloudReporter::builder()
        .trace_project_id("your-gcp-project-id")
        .service_name("my-service")
        .build()
        .await
        .unwrap();

    fastrace::set_reporter(reporter, Config::default());


    {
        // Create and finish a root span
        let root = Span::root("root_span", SpanContext::random());
        let _guard = root.set_local_parent();
    }

    // Report all spans to Google Cloud Trace
    fastrace::flush();
}
```

### Customization

You can customize attribute mappings and converters using the builder pattern:

```rust
use fastrace_google_cloud::{GoogleCloudReporter, opentelemetry_semantic_mapping};

let mut reporter = GoogleCloudReporter::builder()
    .trace_project_id("your-gcp-project-id")
    .service_name("my-service")
    .attribute_name_mappings(opentelemetry_semantic_mapping())
    // Optionally set custom converters for status, span kind, stack trace
    .build()
    .await
    .unwrap();
```

## Configuration

- **trace_project_id**: *(required)* Your Google Cloud project ID.
- **service_name**: *(optional)* Service name to associate with traces.
- **attribute_name_mappings**: *(optional)* Custom mapping from OpenTelemetry attributes to Google Cloud Trace attributes.
- **status_converter**: *(optional)* Function to convert span status.
- **span_kind_converter**: *(optional)* Function to convert span kind.
- **stack_trace_converter**: *(optional)* Function to convert stack trace.

## Documentation

- API documentation: [docs.rs/fastrace-google-cloud](https://docs.rs/fastrace-google-cloud)
- Upstream tracing library: [fastrace](https://github.com/fast/fastrace)
- Google Cloud Trace: [cloud.google.com/trace](https://cloud.google.com/trace)

## License

Licensed under either of

- Apache License, Version 2.0
- MIT license

at your option.

---

*Contributions and issues are welcome!*
