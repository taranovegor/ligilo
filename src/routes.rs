use axum::{
    extract::{FromRequest, Path, Request, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::future::Future;
use tracing::info;
use url::{Host, Url};

use crate::{db, AppState};

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

struct JsonPayload<T>(T);

impl<T, S> FromRequest<S> for JsonPayload<T>
where
    T: serde::de::DeserializeOwned + 'static,
    S: Send + Sync + 'static,
{
    type Rejection = (StatusCode, Json<ErrorResponse>);

    fn from_request(
        req: Request,
        state: &S,
    ) -> impl Future<Output = Result<Self, <Self as FromRequest<S>>::Rejection>> + Send {
        Box::pin(async move {
            match Json::<T>::from_request(req, state).await {
                Ok(Json(value)) => Ok(JsonPayload(value)),
                Err(_) => Err((
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "Invalid or missing JSON".to_string(),
                    }),
                )),
            }
        })
    }
}

fn is_safe_ipv4(ip: std::net::Ipv4Addr) -> bool {
    !ip.is_loopback()
        && !ip.is_private()
        && !ip.is_link_local()
        && !ip.is_unspecified()
        && !is_rfc6598_shared_space(ip)
}

fn is_safe_host(parsed: &Url) -> bool {
    match parsed.host() {
        Some(Host::Domain(d)) => {
            let d = d.to_lowercase();
            d != "localhost" && !d.ends_with(".local") && !d.ends_with(".internal")
        }
        Some(Host::Ipv4(ip)) => is_safe_ipv4(ip),
        Some(Host::Ipv6(ip)) => {
            // Check for IPv4-mapped IPv6 addresses (e.g., ::ffff:127.0.0.1)
            if let Some(ipv4) = ip.to_ipv4_mapped() {
                return is_safe_ipv4(ipv4);
            }
            !ip.is_loopback() && !ip.is_unicast_link_local() && !ip.is_unique_local()
        }
        None => false,
    }
}

fn is_rfc6598_shared_space(ip: std::net::Ipv4Addr) -> bool {
    // RFC 6598: 100.64.0.0/10 (carrier-grade NAT)
    ip.octets()[0] == 100 && (ip.octets()[1] & 0xC0) == 64
}

#[derive(Serialize, Deserialize, Debug)]
pub struct CreateUrlRequest {
    pub url: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct CreateUrlResponse {
    pub code: String,
    pub short_url: String,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/{code}", get(redirect))
        .route("/api/links", post(create_short_url))
}

async fn redirect(Path(code): Path<String>, State(state): State<AppState>) -> impl IntoResponse {
    if code.is_empty()
        || code.len() > 32
        || !code
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    {
        return StatusCode::NOT_FOUND.into_response();
    }

    if let Some(url) = state.url_cache.get(&code).await {
        info!("Redirect (cached): {} -> {}", code, url);
        return (StatusCode::FOUND, [("Location", url)], "").into_response();
    }

    match db::get_url(&state.db, &code).await {
        Ok(Some(url)) => {
            state.url_cache.insert(code.clone(), url.clone()).await;
            info!("Redirect (from DB): {} -> {}", code, url);
            (StatusCode::FOUND, [("Location", url)], "").into_response()
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!("Database error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

type ApiError = (StatusCode, Json<ErrorResponse>);

fn err(status: StatusCode, message: &str) -> ApiError {
    (
        status,
        Json(ErrorResponse {
            error: message.to_string(),
        }),
    )
}

async fn create_short_url(
    State(state): State<AppState>,
    JsonPayload(req): JsonPayload<CreateUrlRequest>,
) -> Result<Json<CreateUrlResponse>, ApiError> {
    let url = req.url.trim();
    if url.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "url is required"));
    }

    const MAX_URL_LEN: usize = 2048;
    if url.len() > MAX_URL_LEN {
        return Err(err(StatusCode::BAD_REQUEST, "url too long"));
    }

    let parsed = Url::parse(url).map_err(|_| err(StatusCode::BAD_REQUEST, "invalid url"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(err(StatusCode::BAD_REQUEST, "url must use http or https"));
    }
    if !is_safe_host(&parsed) {
        return Err(err(StatusCode::BAD_REQUEST, "url host is not allowed"));
    }

    let normalized = parsed.to_string();

    for attempt in 0..state.max_collision_attempts {
        let code = nanoid::nanoid!(5);
        match db::create_url(&state.db, &code, &normalized).await {
            Ok(()) => {
                let short_url = format!("{}/{}", state.base_url, code);
                info!("Created short URL: {} -> {}", code, normalized);
                return Ok(Json(CreateUrlResponse { code, short_url }));
            }
            Err(sqlx::Error::Database(ref db_err)) if db_err.code().as_deref() == Some("23505") => {
                tracing::warn!("Code collision on attempt {}: {}", attempt + 1, code);
            }
            Err(e) => {
                tracing::error!("Database error: {}", e);
                return Err(err(StatusCode::INTERNAL_SERVER_ERROR, "database error"));
            }
        }
    }

    Err(err(
        StatusCode::INTERNAL_SERVER_ERROR,
        "failed to generate unique code",
    ))
}
