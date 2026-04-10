use ligilo::{create_app, AppState};
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
        .init();

    println!("{}! v{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));

    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let base_url =
        std::env::var("BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());
    let port = std::env::var("PORT")
        .unwrap_or_else(|_| "8080".to_string())
        .parse::<u16>()
        .expect("PORT must be a valid u16");
    let max_collision_attempts = std::env::var("MAX_COLLISION_ATTEMPTS")
        .unwrap_or_else(|_| "3".to_string())
        .parse::<usize>()
        .expect("MAX_COLLISION_ATTEMPTS must be a valid usize");
    let db_max_connections = std::env::var("DB_MAX_CONNECTIONS")
        .unwrap_or_else(|_| "5".to_string())
        .parse::<u32>()
        .expect("DB_MAX_CONNECTIONS must be a valid u32");
    let cache_max_capacity = std::env::var("CACHE_MAX_CAPACITY")
        .unwrap_or_else(|_| "100000".to_string())
        .parse::<u64>()
        .expect("CACHE_MAX_CAPACITY must be a valid u64");
    let cache_ttl_secs = std::env::var("CACHE_TTL_SECS")
        .unwrap_or_else(|_| "300".to_string())
        .parse::<u64>()
        .expect("CACHE_TTL_SECS must be a valid u64");

    if cache_ttl_secs == 0 {
        panic!("CACHE_TTL_SECS must be greater than 0");
    }
    if cache_max_capacity == 0 {
        panic!("CACHE_MAX_CAPACITY must be greater than 0");
    }

    let pool = PgPoolOptions::new()
        .max_connections(db_max_connections)
        .connect(&database_url)
        .await
        .expect("Failed to connect to database");

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    let url_cache = moka::future::Cache::builder()
        .max_capacity(cache_max_capacity)
        .time_to_live(Duration::from_secs(cache_ttl_secs))
        .build();

    tracing::info!(
        "Cache configured: capacity={}, ttl={}s",
        cache_max_capacity,
        cache_ttl_secs
    );

    let state = AppState {
        db: pool,
        base_url: Arc::from(base_url.as_str()),
        max_collision_attempts,
        url_cache,
    };
    let app = create_app(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("Failed to bind");

    tracing::info!("Server listening on {}", addr);

    axum::serve(listener, app).await.expect("Server error");
}
