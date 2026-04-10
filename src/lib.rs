pub mod db;
pub mod routes;

pub use axum::Router;

use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub db: sqlx::PgPool,
    pub base_url: Arc<str>,
    pub max_collision_attempts: usize,
    pub url_cache: moka::future::Cache<String, String>,
}

pub fn create_app(state: AppState) -> Router {
    routes::routes()
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state)
}
