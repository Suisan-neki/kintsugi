//! HTTPの構文解析はhyper、受付と共通検査はaxumに任せる。
use super::*;
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Request as HttpRequest, State},
    middleware::{self, Next},
    response::Response,
};
use std::{sync::Arc, time::Duration};

#[derive(Clone)]
struct Server {
    directory: PathBuf,
    token: String,
    port: u16,
    controller: Option<Controller>,
    app: Option<App>,
    store: crate::understanding::Store,
    slow: Arc<tokio::sync::Semaphore>,
    jobs: super::jobs::Queue,
}
pub(super) fn failure(status: u16, message: &str) -> Response {
    let mut response = Response::new(Body::empty());
    let _ = json(&mut response, status, &serde_json::json!({"error":message}));
    response
}
struct Rejection(u16, &'static str);
fn authorize(request: &HttpRequest, server: &Server) -> Result<(), Rejection> {
    for name in [
        "host",
        "origin",
        "x-review-token",
        "content-type",
        "content-length",
    ] {
        if request.headers().get_all(name).iter().count() > 1 {
            return Err(Rejection(400, "HTTPヘッダーが重複しています"));
        }
    }
    let host = request
        .headers()
        .get("host")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if host != format!("127.0.0.1:{}", server.port) && host != format!("localhost:{}", server.port)
    {
        return Err(Rejection(403, "localhostのHostだけを受け付けます"));
    }
    let path = request.uri().path();
    if path.starts_with("/api/") && !path.starts_with("/api/v1/") {
        return Err(Rejection(
            410,
            "旧APIは廃止しました。/api/v1/を使用してください",
        ));
    }
    let authenticated = request
        .headers()
        .get("x-review-token")
        .and_then(|v| v.to_str().ok())
        == Some(&server.token);
    if path.starts_with("/api/v1/") && path != "/api/v1/capabilities" && !authenticated {
        return Err(Rejection(403, "APIの操作トークンが必要です"));
    }
    if !matches!(
        *request.method(),
        axum::http::Method::GET | axum::http::Method::HEAD
    ) {
        let origin = request.headers().get("origin");
        if !authenticated
            || origin.is_some_and(|v| v != format!("http://{host}").as_str())
            || request
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                != Some("application/json")
        {
            return Err(Rejection(
                403,
                "画面のOrigin・操作トークン・JSON形式が必要です",
            ));
        }
    }
    Ok(())
}
async fn gate(State(server): State<Server>, request: HttpRequest, next: Next) -> Response {
    match authorize(&request, &server) {
        Ok(()) => next.run(request).await,
        Err(Rejection(status, message)) => failure(status, message),
    }
}
async fn handle(server: Server, request: HttpRequest) -> Response {
    let asynchronous = request
        .headers()
        .get("prefer")
        .is_some_and(|v| v == "respond-async");
    let (parts, body) = request.into_parts();
    let body = match tokio::time::timeout(Duration::from_secs(3), to_bytes(body, MAX_BODY)).await {
        Ok(Ok(body)) => body.to_vec(),
        _ => return failure(400, "本文のサイズまたは待機上限を超えています"),
    };
    let path = parts
        .uri
        .path_and_query()
        .map_or("/", |p| p.as_str())
        .to_owned();
    let request = Request {
        method: parts.method.to_string(),
        path,
        body,
    };
    // AIと外部読取の待ちが、事例・結果の読出しの枠を使い切らないようにする。
    let slow = request.method == "POST"
        && matches!(
            request.path.as_str(),
            "/api/v1/system-diagram"
                | "/api/v1/system-diagram/question"
                | "/api/v1/system-diagram/dialogue"
                | "/api/v1/ai"
                | "/api/v1/understand"
                | "/api/v1/prepare"
        );
    if let Some(id) = request
        .path
        .strip_prefix("/api/v1/jobs/")
        .and_then(|id| id.parse::<u64>().ok())
    {
        let result = match request.method.as_str() {
            "GET" => server.jobs.get(id),
            "DELETE" => server.jobs.cancel(id),
            _ => return failure(404, "この操作はありません"),
        };
        let mut response = Response::new(Body::empty());
        let _ = result_response(&mut response, result);
        return response;
    }
    if slow && asynchronous {
        let queue = server.jobs.clone();
        let result = queue.start(move || dispatch(&server, request));
        let mut response = Response::new(Body::empty());
        match result {
            Ok(job) => {
                let _ = json(&mut response, 202, &job);
            }
            Err(error) => {
                let _ = result_response::<()>(&mut response, Err(error));
            }
        }
        return response;
    }
    let permit = if slow {
        match server.slow.clone().try_acquire_owned() {
            Ok(permit) => Some(permit),
            Err(_) => return failure(503, "読取・AI処理が進行中です。後で再試行してください"),
        }
    } else {
        None
    };
    match tokio::task::spawn_blocking(move || {
        let _permit = permit;
        dispatch(&server, request)
    })
    .await
    {
        Ok(response) => response,
        Err(_) => failure(500, "処理を完了できませんでした"),
    }
}
fn dispatch(server: &Server, request: Request) -> Response {
    let mut response = Response::new(Body::empty());
    match handle_request(
        request,
        &mut response,
        &server.directory,
        &server.token,
        server.controller.as_ref(),
        server.app.as_ref(),
        &server.store,
    ) {
        Ok(()) => response,
        Err(_) => failure(500, "操作を完了できませんでした"),
    }
}
async fn endpoint(State(server): State<Server>, request: HttpRequest) -> Response {
    handle(server, request).await
}
fn state(
    listener: &TcpListener,
    directory: PathBuf,
    token: String,
    controller: Option<Controller>,
    app: Option<App>,
) -> io::Result<Server> {
    let store = match &app {
        Some(app) => app.store.clone(),
        None => crate::understanding::Store::new(directory.join("snapshots"))?,
    };
    let slow = Arc::new(tokio::sync::Semaphore::new(4));
    Ok(Server {
        directory,
        token,
        port: listener.local_addr()?.port(),
        controller,
        app,
        store,
        jobs: super::jobs::Queue::new(slow.clone()),
        slow,
    })
}
pub(super) fn serve(
    listener: TcpListener,
    directory: PathBuf,
    token: String,
    controller: Option<Controller>,
    app: Option<App>,
) -> io::Result<()> {
    let server = state(&listener, directory, token, controller, app)?;
    listener.set_nonblocking(true)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .max_blocking_threads(64)
        .build()?;
    runtime.block_on(async move {
        let listener = tokio::net::TcpListener::from_std(listener)?;
        let router = Router::new()
            .fallback(endpoint)
            .layer(middleware::from_fn_with_state(server.clone(), gate))
            .with_state(server);
        axum::serve(listener, router).await
    })
}
pub(super) fn connection(
    stream: TcpStream,
    directory: &Path,
    token: &str,
    controller: Option<&Controller>,
    app: Option<&App>,
) -> io::Result<()> {
    let store = match &app {
        Some(app) => app.store.clone(),
        None => crate::understanding::Store::new(directory.join("snapshots"))?,
    };
    let slow = Arc::new(tokio::sync::Semaphore::new(4));
    let server = Server {
        directory: directory.to_owned(),
        token: token.to_owned(),
        port: stream.local_addr()?.port(),
        controller: controller.cloned(),
        app: app.cloned(),
        store,
        jobs: super::jobs::Queue::new(slow.clone()),
        slow,
    };
    stream.set_nonblocking(true)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let stream = tokio::net::TcpStream::from_std(stream)?;
        let service =
            hyper::service::service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
                let server = server.clone();
                async move {
                    let request = request.map(Body::new);
                    let response = match authorize(&request, &server) {
                        Ok(()) => handle(server, request).await,
                        Err(Rejection(status, message)) => failure(status, message),
                    };
                    Ok::<_, std::convert::Infallible>(response)
                }
            });
        hyper::server::conn::http1::Builder::new()
            .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
            .await
            .map_err(io::Error::other)
    })
}
