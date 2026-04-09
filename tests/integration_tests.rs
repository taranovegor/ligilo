use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;

// --- helpers -----------------------------------------------------------------

async fn create_test_router() -> impl tower::Service<
    Request<Body>,
    Response = axum::response::Response,
    Error = std::convert::Infallible,
> {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:@localhost/shortener".to_string());

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("Failed to connect to test database");

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    let state = ligilo::AppState {
        db: pool,
        base_url: std::sync::Arc::from("http://localhost:8080"),
        max_collision_attempts: 2,
    };

    ligilo::create_app(state)
}

async fn json_body(response: axum::response::Response) -> Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

async fn post_links(
    app: impl tower::Service<
        Request<Body>,
        Response = axum::response::Response,
        Error = std::convert::Infallible,
    >,
    url: &str,
) -> axum::response::Response {
    app.oneshot(
        Request::builder()
            .uri("/api/links")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(json!({ "url": url }).to_string()))
            .unwrap(),
    )
    .await
    .unwrap()
}

// --- redirect ----------------------------------------------------------------

#[tokio::test]
async fn test_redirect_not_found() {
    let app = create_test_router().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/nonexistent")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_create_then_redirect() {
    let app = create_test_router().await;

    let create_response = post_links(app, "https://example.com/round-trip").await;
    assert_eq!(create_response.status(), StatusCode::OK);

    let body = json_body(create_response).await;
    let code = body["code"].as_str().unwrap().to_string();
    assert_eq!(code.len(), 5);

    let app = create_test_router().await;
    let redirect_response = app
        .oneshot(
            Request::builder()
                .uri(format!("/{}", code))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(redirect_response.status(), StatusCode::FOUND);
    assert_eq!(
        redirect_response.headers().get("location").unwrap(),
        "https://example.com/round-trip"
    );
}

// --- POST /api/links -------------------------------------------------------

#[tokio::test]
async fn test_shorten_valid_url() {
    let app = create_test_router().await;

    let response = post_links(app, "https://example.com/very/long/path").await;
    assert_eq!(response.status(), StatusCode::OK);

    let body = json_body(response).await;
    let code = body["code"].as_str().expect("code must be a string");
    assert_eq!(code.len(), 5, "code must be 5 characters");

    let short_url = body["short_url"]
        .as_str()
        .expect("short_url must be a string");
    assert!(
        short_url.ends_with(code),
        "short_url must end with the code"
    );
}

#[tokio::test]
async fn test_shorten_empty_url() {
    let app = create_test_router().await;
    let response = post_links(app, "").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json_body(response).await;
    assert!(body["error"].is_string());
}

#[tokio::test]
async fn test_shorten_invalid_scheme() {
    let app = create_test_router().await;
    let response = post_links(app, "javascript:alert(1)").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json_body(response).await;
    assert!(body["error"].is_string());
}

#[tokio::test]
async fn test_shorten_ftp_scheme_rejected() {
    let app = create_test_router().await;
    let response = post_links(app, "ftp://example.com/file").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_shorten_localhost_rejected() {
    let app = create_test_router().await;
    let response = post_links(app, "http://localhost/admin").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_shorten_private_ip_rejected() {
    let app = create_test_router().await;
    let response = post_links(app, "http://192.168.1.1/").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_shorten_link_local_ip_rejected() {
    let app = create_test_router().await;
    let response = post_links(app, "http://169.254.169.254/latest/meta-data/").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_shorten_ipv6_unique_local_rejected() {
    let app = create_test_router().await;
    let response = post_links(app, "http://[fd12:3456:789a:b::1]/internal").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_shorten_url_too_long() {
    let app = create_test_router().await;
    let long_url = format!("https://example.com/{}", "a".repeat(2048));
    let response = post_links(app, &long_url).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json_body(response).await;
    assert!(body["error"].is_string());
}

// --- GET /{code} path validation -----------------------------------------------

#[tokio::test]
async fn test_redirect_invalid_code_too_long() {
    let app = create_test_router().await;
    let code = "a".repeat(33); // longer than 32
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/{}", code))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_redirect_invalid_code_special_chars() {
    let app = create_test_router().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/abc@def")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_redirect_valid_code_with_underscore() {
    let app = create_test_router().await;
    let response = post_links(app, "https://example.com/test").await;
    assert_eq!(response.status(), StatusCode::OK);

    let body = json_body(response).await;
    let code = body["code"].as_str().unwrap().to_string();

    let app = create_test_router().await;
    let redirect_response = app
        .oneshot(
            Request::builder()
                .uri(format!("/{}", code))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(redirect_response.status(), StatusCode::FOUND);
}

// --- SSRF Protection: IPv6-mapped IPv4 -----------------------------------------

#[tokio::test]
async fn test_shorten_ipv4_mapped_ipv6_loopback_rejected() {
    let app = create_test_router().await;
    let response = post_links(app, "http://[::ffff:127.0.0.1]/admin").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_shorten_ipv4_mapped_ipv6_private_rejected() {
    let app = create_test_router().await;
    let response = post_links(app, "http://[::ffff:192.168.1.1]/admin").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// --- SSRF Protection: Unspecified and RFC 6598 --------------------------------

#[tokio::test]
async fn test_shorten_unspecified_ip_rejected() {
    let app = create_test_router().await;
    let response = post_links(app, "http://0.0.0.0/").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_shorten_rfc6598_shared_space_rejected() {
    let app = create_test_router().await;
    // RFC 6598: 100.64.0.0/10
    let response = post_links(app, "http://100.64.0.1/").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_shorten_rfc6598_upper_bound_rejected() {
    let app = create_test_router().await;
    // RFC 6598: up to 100.127.255.255
    let response = post_links(app, "http://100.127.255.255/").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// --- SSRF Protection: Valid public IPs ------------------------------------------

#[tokio::test]
async fn test_shorten_valid_public_ipv4() {
    let app = create_test_router().await;
    // 8.8.8.8 is public DNS
    let response = post_links(app, "http://8.8.8.8/").await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_shorten_valid_public_ipv6() {
    let app = create_test_router().await;
    // 2001:4860:4860::8888 is public DNS (Google)
    let response = post_links(app, "http://[2001:4860:4860::8888]/").await;
    assert_eq!(response.status(), StatusCode::OK);
}
