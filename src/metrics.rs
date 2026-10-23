use axum::{
    extract::{MatchedPath, Request, State},
    http::{header::CONTENT_TYPE, Method},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};
use std::time::{Duration, Instant};

use crate::AppState;

const HTTP_REQUESTS_TOTAL: &str = "http_requests_total";
const HTTP_REQUEST_DURATION: &str = "http_request_duration_seconds";

const HTTP_DURATION_BUCKETS: &[f64] = &[
    0.0001, 0.00025, 0.0005, 0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0,
];

pub fn install_recorder() -> PrometheusHandle {
    let handle = PrometheusBuilder::new()
        .set_buckets_for_metric(
            Matcher::Full(HTTP_REQUEST_DURATION.to_string()),
            HTTP_DURATION_BUCKETS,
        )
        .expect("Invalid histogram buckets")
        .install_recorder()
        .expect("Failed to install Prometheus recorder");

    let upkeep = handle.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        loop {
            interval.tick().await;
            upkeep.run_upkeep();
        }
    });

    handle
}

fn method_label(method: &Method) -> &'static str {
    match *method {
        Method::GET => "GET",
        Method::POST => "POST",
        Method::PUT => "PUT",
        Method::DELETE => "DELETE",
        Method::HEAD => "HEAD",
        Method::OPTIONS => "OPTIONS",
        Method::PATCH => "PATCH",
        Method::CONNECT => "CONNECT",
        Method::TRACE => "TRACE",
        _ => "OTHER",
    }
}

pub async fn track_http(req: Request, next: Next) -> Response {
    let start = Instant::now();
    let method = method_label(req.method());
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_owned())
        .unwrap_or_else(|| "unmatched".to_owned());

    let response = next.run(req).await;

    let labels = [
        ("method", method.to_owned()),
        ("route", route),
        ("status", response.status().as_u16().to_string()),
    ];
    metrics::counter!(HTTP_REQUESTS_TOTAL, &labels).increment(1);
    metrics::histogram!(HTTP_REQUEST_DURATION, &labels).record(start.elapsed().as_secs_f64());

    response
}

pub fn router(handle: PrometheusHandle) -> Router<AppState> {
    Router::new().route(
        "/metrics",
        get(move |State(state): State<AppState>| async move {
            state.url_cache.run_pending_tasks().await;
            metrics::gauge!("cache_entries").set(state.url_cache.entry_count() as f64);
            let idle = state.db.num_idle();
            let active = (state.db.size() as usize).saturating_sub(idle);
            metrics::gauge!("db_pool_connections", "state" => "idle").set(idle as f64);
            metrics::gauge!("db_pool_connections", "state" => "active").set(active as f64);

            (
                [(CONTENT_TYPE, "text/plain; version=0.0.4")],
                handle.render(),
            )
                .into_response()
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request as HttpRequest, middleware};
    use tower::ServiceExt;

    #[tokio::test]
    async fn track_http_labels_by_route_template() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        let _guard = metrics::set_default_local_recorder(&recorder);

        let app: Router = Router::new()
            .route("/items/{id}", get(|| async { "ok" }))
            .layer(middleware::from_fn(track_http));

        app.oneshot(
            HttpRequest::builder()
                .uri("/items/42")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

        let output = handle.render();
        assert!(output.contains(r#"route="/items/{id}""#), "{output}");
        assert!(!output.contains("/items/42"), "{output}");
        assert!(output.contains(r#"status="200""#), "{output}");
    }

    #[tokio::test]
    async fn track_http_labels_unmatched_route() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        let _guard = metrics::set_default_local_recorder(&recorder);

        let app: Router = Router::new()
            .route("/items/{id}", get(|| async { "ok" }))
            .layer(middleware::from_fn(track_http));

        app.oneshot(
            HttpRequest::builder()
                .uri("/no/such/path")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

        let output = handle.render();
        assert!(output.contains(r#"route="unmatched""#), "{output}");
        assert!(output.contains(r#"status="404""#), "{output}");
        assert!(!output.contains("/no/such/path"), "{output}");
    }

    #[tokio::test]
    async fn track_http_collapses_unknown_methods() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        let _guard = metrics::set_default_local_recorder(&recorder);

        let app: Router = Router::new()
            .route("/items/{id}", get(|| async { "ok" }))
            .layer(middleware::from_fn(track_http));

        app.oneshot(
            HttpRequest::builder()
                .method("FOOBAR")
                .uri("/items/42")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

        let output = handle.render();
        assert!(output.contains(r#"method="OTHER""#), "{output}");
        assert!(!output.contains("FOOBAR"), "{output}");
    }

    #[tokio::test]
    async fn metrics_endpoint_renders_prometheus_text() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        let _guard = metrics::set_default_local_recorder(&recorder);

        let state = AppState {
            db: sqlx::postgres::PgPoolOptions::new()
                .connect_lazy("postgres://localhost/unused")
                .unwrap(),
            base_url: std::sync::Arc::from("http://localhost:8080"),
            max_collision_attempts: 1,
            url_cache: moka::future::Cache::new(10),
        };
        let app = router(handle).with_state(state);

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        assert_eq!(
            response.headers().get(CONTENT_TYPE).unwrap(),
            "text/plain; version=0.0.4"
        );

        let bytes = http_body_util::BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        let output = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(output.contains("cache_entries 0"), "{output}");
        assert!(
            output.contains(r#"db_pool_connections{state="idle"} 0"#),
            "{output}"
        );
        assert!(
            output.contains(r#"db_pool_connections{state="active"} 0"#),
            "{output}"
        );
    }
}
