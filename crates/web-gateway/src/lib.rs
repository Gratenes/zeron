use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use anyhow::Context as _;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message as UpstreamMessage;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tower_http::services::ServeDir;
use zeron_engine::auth::{Auth, AuthConfig};

const MAX_PROXY_BODY: usize = 32 * 1024;
const RPC_PROTOCOL: &str = "kratos-rpc";
const BEARER_PROTOCOL: &str = "kratos-bearer.";

#[derive(Clone)]
struct Gateway {
    upstream: String,
    allowed_host: String,
    http: reqwest::Client,
}

pub async fn serve(
    data_dir: std::path::PathBuf,
    hostname: String,
    port: u16,
) -> anyhow::Result<()> {
    let allowed_host = normalized_host(&hostname)
        .filter(|host| valid_external_host(host))
        .context("--hostname must be an external DNS hostname")?;
    let upstream = Auth::open(AuthConfig::new(data_dir))?
        .peer_endpoint()
        .context("initialize this installation with `zeron peer init` first")?;
    let state = Gateway {
        upstream,
        allowed_host,
        http: reqwest::Client::new(),
    };
    let app = gateway_router(state);
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(%address, hostname = %hostname, "web gateway listening");
    axum::serve(listener, app).await?;
    Ok(())
}

fn gateway_router(state: Gateway) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/pair/{*path}", any(proxy_pair))
        .route("/device/{device_id}/ws", get(proxy_socket))
        .fallback_service(
            // Serves `<file>.zst` / `<file>.gz` siblings (see apps/web/build-release.sh)
            // when the client accepts them; the plain file remains the fallback.
            ServeDir::new(web_assets_dir())
                .append_index_html_on_directories(true)
                .precompressed_zstd()
                .precompressed_gzip(),
        )
        .layer(middleware::from_fn(web_headers))
        .layer(middleware::from_fn_with_state(state.clone(), enforce_host))
        .with_state(state)
}

fn web_assets_dir() -> std::path::PathBuf {
    std::env::var_os("ZERON_WEB_ASSETS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("apps/web/dist"))
}

async fn web_headers(request: Request, next: Next) -> Response {
    let cache_control = cache_control(request.uri().path());
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        HeaderName::from_static("cross-origin-embedder-policy"),
        HeaderValue::from_static("require-corp"),
    );
    response.headers_mut().insert(
        HeaderName::from_static("cross-origin-opener-policy"),
        HeaderValue::from_static("same-origin"),
    );
    // Never let a missing hashed asset (404) be cached as immutable.
    let cache_control = if cache_control == IMMUTABLE && !response.status().is_success() {
        NO_CACHE
    } else {
        cache_control
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static(cache_control));
    // ServeDir picks a precompressed sibling per request but does not say so.
    response
        .headers_mut()
        .append(header::VARY, HeaderValue::from_static("accept-encoding"));
    response
}

const IMMUTABLE: &str = "public, max-age=31536000, immutable";
const NO_CACHE: &str = "no-cache";

/// Trunk content-hashes its bundle output (`/zeron-browser-<hash>*` from
/// wasm-bindgen, `/<hash>-<name>` for copied modules), so those are safe to
/// cache forever. The unhashed shell revalidates; API routes carry tokens.
fn cache_control(path: &str) -> &'static str {
    if path == "/healthz" || path.starts_with("/pair/") || path.starts_with("/device/") {
        "no-store"
    } else if is_hashed_asset(path) {
        IMMUTABLE
    } else {
        NO_CACHE
    }
}

fn is_hashed_asset(path: &str) -> bool {
    let name = path.trim_start_matches('/');
    name.starts_with("zeron-browser-")
        || name.split_once('-').is_some_and(|(hash, _)| {
            (8..=16).contains(&hash.len()) && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
}

async fn enforce_host(State(state): State<Gateway>, request: Request, next: Next) -> Response {
    let headers = request.headers();
    let forwarded = headers
        .get("x-forwarded-host")
        .and_then(|value| value.to_str().ok());
    let host = forwarded.or_else(|| {
        headers
            .get(header::HOST)
            .and_then(|value| value.to_str().ok())
    });
    let allowed = host.and_then(normalized_host).is_some_and(|host| {
        host == state.allowed_host || host == "localhost" || host == "127.0.0.1"
    });
    if !allowed {
        return (StatusCode::MISDIRECTED_REQUEST, "unrecognized host").into_response();
    }
    next.run(request).await
}

async fn health(State(state): State<Gateway>) -> Response {
    match state
        .http
        .get(format!("{}/pair/devices", state.upstream))
        .send()
        .await
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "local peer is not running").into_response(),
    }
}

async fn proxy_pair(
    State(state): State<Gateway>,
    Path(path): Path<String>,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let body = match to_bytes(body, MAX_PROXY_BODY).await {
        Ok(body) => body,
        Err(_) => return (StatusCode::PAYLOAD_TOO_LARGE, "request too large").into_response(),
    };
    let mut upstream = state
        .http
        .request(parts.method, format!("{}/pair/{path}", state.upstream));
    for (name, value) in filtered_headers(&parts.headers) {
        upstream = upstream.header(name, value);
    }
    match upstream.body(body).send().await {
        Ok(response) => proxy_response(response).await,
        Err(_) => (StatusCode::BAD_GATEWAY, "local peer is not running").into_response(),
    }
}

async fn proxy_response(response: reqwest::Response) -> Response {
    let status = response.status();
    let content_type = response.headers().get(header::CONTENT_TYPE).cloned();
    match response.bytes().await {
        Ok(body) => {
            let mut output = Response::new(Body::from(body));
            *output.status_mut() = status;
            if let Some(content_type) = content_type {
                output
                    .headers_mut()
                    .insert(header::CONTENT_TYPE, content_type);
            }
            output
        }
        Err(_) => (StatusCode::BAD_GATEWAY, "invalid peer response").into_response(),
    }
}

fn filtered_headers(headers: &HeaderMap) -> impl Iterator<Item = (&HeaderName, &HeaderValue)> {
    headers.iter().filter(|(name, _)| {
        *name != header::HOST
            && *name != header::CONNECTION
            && *name != header::UPGRADE
            && !name.as_str().starts_with("x-forwarded-")
            && name.as_str() != "sec-websocket-protocol"
    })
}

async fn proxy_socket(
    State(state): State<Gateway>,
    Path(device_id): Path<String>,
    headers: HeaderMap,
    socket: WebSocketUpgrade,
) -> Response {
    if !valid_device_id(&device_id) {
        return (StatusCode::BAD_REQUEST, "invalid device id").into_response();
    }
    let Some(token) = socket_bearer(&headers) else {
        return (StatusCode::UNAUTHORIZED, "missing bearer protocol").into_response();
    };
    socket
        .protocols([RPC_PROTOCOL])
        .on_upgrade(move |browser| bridge_socket(browser, state, device_id, token))
}

async fn bridge_socket(browser: WebSocket, state: Gateway, device_id: String, token: String) {
    let url = format!("{}/device/{device_id}/ws", state.upstream).replacen("http", "ws", 1);
    let mut request = match url.into_client_request() {
        Ok(request) => request,
        Err(_) => return,
    };
    let Ok(value) = HeaderValue::from_str(&format!("Bearer {token}")) else {
        return;
    };
    request.headers_mut().insert(header::AUTHORIZATION, value);
    let Ok((upstream, _)) = tokio_tungstenite::connect_async(request).await else {
        return;
    };
    let (mut browser_out, mut browser_in) = browser.split();
    let (mut upstream_out, mut upstream_in) = upstream.split();
    loop {
        tokio::select! {
            message = browser_in.next() => match message {
                Some(Ok(message)) => match to_upstream(message) {
                    Some(message) => if upstream_out.send(message).await.is_err() { break },
                    None => break,
                },
                _ => break,
            },
            message = upstream_in.next() => match message {
                Some(Ok(message)) => if let Some(message) = to_browser(message)
                    && browser_out.send(message).await.is_err()
                {
                    break;
                },
                _ => break,
            }
        }
    }
}

fn to_upstream(message: Message) -> Option<UpstreamMessage> {
    match message {
        Message::Text(value) => Some(UpstreamMessage::Text(value.to_string())),
        Message::Binary(value) => Some(UpstreamMessage::Binary(value.to_vec())),
        Message::Ping(value) => Some(UpstreamMessage::Ping(value.to_vec())),
        Message::Pong(value) => Some(UpstreamMessage::Pong(value.to_vec())),
        Message::Close(_) => Some(UpstreamMessage::Close(None)),
    }
}

fn to_browser(message: UpstreamMessage) -> Option<Message> {
    match message {
        UpstreamMessage::Text(value) => Some(Message::Text(value.to_string().into())),
        UpstreamMessage::Binary(value) => Some(Message::Binary(value.into())),
        UpstreamMessage::Ping(value) => Some(Message::Ping(value.into())),
        UpstreamMessage::Pong(value) => Some(Message::Pong(value.into())),
        UpstreamMessage::Close(_) => Some(Message::Close(None)),
        UpstreamMessage::Frame(_) => None,
    }
}

fn socket_bearer(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::SEC_WEBSOCKET_PROTOCOL)?
        .to_str()
        .ok()?
        .split(',')
        .map(str::trim)
        .find_map(|protocol| protocol.strip_prefix(BEARER_PROTOCOL))
        .filter(|token| !token.is_empty() && token.len() <= 512)
        .map(str::to_owned)
}

fn normalized_host(value: &str) -> Option<String> {
    let value = value.trim().trim_end_matches('.');
    let host = value
        .strip_prefix('[')
        .and_then(|value| value.split_once(']').map(|(host, _)| host))
        .or_else(|| value.split_once(':').map(|(host, _)| host))
        .unwrap_or(value)
        .trim_end_matches('.')
        .to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

fn valid_device_id(value: &str) -> bool {
    value.starts_with("dev_")
        && value.len() <= 96
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn valid_external_host(value: &str) -> bool {
    value.len() <= 253
        && value.contains('.')
        && value.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    #[test]
    fn caches_hashed_bundle_forever_and_never_caches_api_routes() {
        assert_eq!(cache_control("/zeron-browser-a9e6_bg.wasm"), IMMUTABLE);
        assert_eq!(cache_control("/zeron-browser-a9e6.js"), IMMUTABLE);
        assert_eq!(cache_control("/1f3a9c07d2b45e68-zeron-browser-initializer.js"), IMMUTABLE);
        assert_eq!(cache_control("/abc-thing.js"), NO_CACHE);
        assert_eq!(cache_control("/"), NO_CACHE);
        assert_eq!(cache_control("/index.html"), NO_CACHE);
        assert_eq!(cache_control("/app.js"), NO_CACHE);
        assert_eq!(cache_control("/pair/authenticate"), "no-store");
        assert_eq!(cache_control("/device/abc/ws"), "no-store");
        assert_eq!(cache_control("/healthz"), "no-store");
    }

    #[test]
    fn parses_websocket_bearer_without_accepting_empty_tokens() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::SEC_WEBSOCKET_PROTOCOL,
            "kratos-rpc, kratos-bearer.abc_123".parse().unwrap(),
        );
        assert_eq!(socket_bearer(&headers).as_deref(), Some("abc_123"));
        headers.insert(
            header::SEC_WEBSOCKET_PROTOCOL,
            "kratos-bearer.".parse().unwrap(),
        );
        assert_eq!(socket_bearer(&headers), None);
    }

    #[test]
    fn host_and_device_validation_are_narrow() {
        assert_eq!(
            normalized_host("Dev.Embedez.com:443").as_deref(),
            Some("dev.embedez.com")
        );
        assert!(valid_device_id("dev_abc-123"));
        assert!(!valid_device_id("../device/dev_bad"));
        assert!(valid_external_host("dev.embedez.com"));
        assert!(!valid_external_host("localhost"));
        assert!(!valid_external_host("bad_host.example"));
    }

    #[tokio::test]
    async fn websocket_bearer_is_forwarded_and_binary_frames_round_trip() {
        async fn authenticated_echo(headers: HeaderMap, socket: WebSocketUpgrade) -> Response {
            if headers
                .get(header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                != Some("Bearer test-token")
            {
                return StatusCode::UNAUTHORIZED.into_response();
            }
            socket.on_upgrade(|mut socket| async move {
                if let Some(Ok(message)) = socket.recv().await {
                    let _ = socket.send(message).await;
                }
            })
        }

        let upstream = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_address = upstream.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(
                upstream,
                Router::new().route("/device/{device_id}/ws", get(authenticated_echo)),
            )
            .await
            .unwrap();
        });

        let gateway = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let gateway_address = gateway.local_addr().unwrap();
        let state = Gateway {
            upstream: format!("http://{upstream_address}"),
            allowed_host: "dev.embedez.com".into(),
            http: reqwest::Client::new(),
        };
        tokio::spawn(async move {
            axum::serve(gateway, gateway_router(state)).await.unwrap();
        });

        let mut request = format!("ws://{gateway_address}/device/dev_engine/ws")
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            header::SEC_WEBSOCKET_PROTOCOL,
            "kratos-rpc, kratos-bearer.test-token".parse().unwrap(),
        );
        let (mut socket, response) = tokio_tungstenite::connect_async(request).await.unwrap();
        assert_eq!(
            response.headers().get(header::SEC_WEBSOCKET_PROTOCOL),
            Some(&HeaderValue::from_static(RPC_PROTOCOL))
        );
        socket
            .send(UpstreamMessage::Binary(vec![1, 2, 3]))
            .await
            .unwrap();
        assert_eq!(
            socket.next().await.unwrap().unwrap(),
            UpstreamMessage::Binary(vec![1, 2, 3])
        );
    }
}
