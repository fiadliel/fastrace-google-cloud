use std::collections::HashMap;

use opentelemetry_semantic_conventions::attribute;

pub fn opentelemetry_semantic_mapping() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        (attribute::OTEL_COMPONENT_TYPE, "/component"),
        (attribute::EXCEPTION_MESSAGE, "/error/message"),
        (attribute::EXCEPTION_MESSAGE, "/error/name"),
        (attribute::NETWORK_PROTOCOL_VERSION, "/http/client_protocol"),
        (attribute::SERVER_ADDRESS, "/http/host"),
        (attribute::CLIENT_ADDRESS, "/http/host"),
        (attribute::HTTP_HOST, "/http/host"),
        (attribute::HTTP_METHOD, "/http/method"),
        (attribute::HTTP_REQUEST_METHOD, "/http/method"),
        // Not a standard OTEL attribute, but some existing systems have this mapping
        ("http.path", "/http/path"),
        (attribute::URL_PATH, "/http/path"),
        (attribute::HTTP_REQUEST_SIZE, "/http/request/size"),
        (attribute::HTTP_RESPONSE_SIZE, "/http/response/size"),
        (attribute::HTTP_ROUTE, "/http/route"),
        (attribute::HTTP_RESPONSE_STATUS_CODE, "/http/status_code"),
        (attribute::HTTP_STATUS_CODE, "/http/status_code"),
        (attribute::HTTP_USER_AGENT, "/http/user_agent"),
        (attribute::USER_AGENT_ORIGINAL, "/http/user_agent"),
        (
            attribute::K8S_CLUSTER_NAME,
            "g.co/r/k8s_container/cluster_name",
        ),
        (
            attribute::K8S_NAMESPACE_NAME,
            "g.co/r/k8s_container/namespace",
        ),
        (attribute::K8S_POD_NAME, "g.co/r/k8s_container/pod_name"),
        (
            attribute::K8S_CONTAINER_NAME,
            "g.co/r/k8s_container/container_name",
        ),
    ])
}
