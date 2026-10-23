pub mod db;
pub mod metrics;
pub mod routes;

pub use axum::Router;

use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub db: sqlx::PgPool,
    pub base_url: Arc<str>,
    pub max_collision_attempts: usize,
    pub url_cache: moka::future::Cache<String, Option<String>>,
}

pub fn create_app(state: AppState) -> Router {
    routes::routes()
        .layer(axum::middleware::from_fn(metrics::track_http))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state)
}

pub fn create_metrics_app(
    state: AppState,
    handle: metrics_exporter_prometheus::PrometheusHandle,
) -> Router {
    metrics::router(handle).with_state(state)
}
