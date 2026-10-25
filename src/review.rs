//! localhostだけで動く共同レビュー。任意のコマンドやファイルはHTTPで受け付けない。
use crate::{
    review_execution::{Controller, ExecuteRequest, ExecutionStatus, Job},
    workspace::{self, *},
};
use serde::{Deserialize, de::DeserializeOwned};
use std::{
    fs::File,
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
};

mod assets;
mod jobs;
mod transport;
use axum::response::Response;
const MAX_BODY: usize = 1024 * 1024;

/// 理解と試験の画面。デジタル庁デザインシステムのCSSを同じ応答に含める。
pub fn app_html(token: &str) -> String {
    styled_app(
        &assets::text("ui/app.html").expect("画面の資材を読み取れません"),
        token,
    )
}

fn workbench_html(token: &str) -> String {
    styled_app(
        &assets::text("ui/workbench.html").expect("画面の資材を読み取れません"),
        token,
    )
}

fn styled_app(html: &str, token: &str) -> String {
    html.replace(
        "__APP_STYLE__",
        &[
            assets::text("ui/dads/tokens.css").expect("画面の資材を読み取れません"),
            assets::text("ui/dads/components.css").expect("画面の資材を読み取れません"),
            assets::text("ui/app.css").expect("画面の資材を読み取れません"),
        ]
        .concat(),
    )
    .replace("__REVIEW_TOKEN__", token)
}

/// 画面版で使う、読取記録の保存先とAIの設定。
#[derive(Clone)]
pub struct App {
    pub store: crate::understanding::Store,
    pub ai: Option<crate::ai::Config>,
    pub dify: Option<crate::system_diagram::dialogue::DifyConfig>,
}

struct Request {
    method: String,
    path: String,
    body: Vec<u8>,
}
fn respond(stream: &mut Response, status: u16, content_type: &str, body: &[u8]) -> io::Result<()> {
    respond_encoded(stream, status, content_type, body, "")
}
fn respond_encoded(
    stream: &mut Response,
    status: u16,
    content_type: &str,
    body: &[u8],
    encoding: &str,
) -> io::Result<()> {
    let mut builder=axum::http::Response::builder()
        .status(status)
        .header("content-type",content_type)
        .header("content-length",body.len())
        .header("cache-control","no-store")
        .header("x-content-type-options","nosniff")
        .header("referrer-policy","no-referrer")
        .header("content-security-policy","default-src 'none'; script-src 'self' 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self' https://api.github.com https://raw.githubusercontent.com; img-src 'self' blob:; form-action 'none'; frame-ancestors 'none'; base-uri 'none'")
        .header("connection","close");
    if !encoding.is_empty() {
        builder = builder.header("content-encoding", "gzip");
    }
    *stream = builder
        .body(axum::body::Body::from(body.to_vec()))
        .map_err(io::Error::other)?;
    Ok(())
}

fn json(stream: &mut Response, status: u16, value: &impl serde::Serialize) -> io::Result<()> {
    respond(
        stream,
        status,
        "application/json; charset=utf-8",
        &serde_json::to_vec(value).map_err(io::Error::other)?,
    )
}

fn parse_body<T: DeserializeOwned>(body: &[u8]) -> io::Result<T> {
    serde_json::from_slice(body).map_err(|e| crate::error::invalid(e.to_string()))
}

fn html_response(stream: &mut Response, html: &str) -> io::Result<()> {
    respond(stream, 200, "text/html; charset=utf-8", html.as_bytes())
}

fn script_response(stream: &mut Response, script: &str) -> io::Result<()> {
    respond(
        stream,
        200,
        "text/javascript; charset=utf-8",
        script.as_bytes(),
    )
}

/// 任意ファイルへのアクセスや任意コマンド実行は公開しない。変更JSONのimportはCLI専用。
pub fn handle_connection(stream: TcpStream, directory: &Path, token: &str) -> io::Result<()> {
    transport::connection(stream, directory, token, None, None)
}
pub fn handle_connection_with_execution(
    stream: TcpStream,
    directory: &Path,
    token: &str,
    controller: &Controller,
) -> io::Result<()> {
    transport::connection(stream, directory, token, Some(controller), None)
}
pub fn handle_connection_app(
    stream: TcpStream,
    directory: &Path,
    token: &str,
    controller: &Controller,
    app: &App,
) -> io::Result<()> {
    transport::connection(stream, directory, token, Some(controller), Some(app))
}
fn handle_request(
    mut request: Request,
    stream: &mut Response,
    directory: &Path,
    token: &str,
    controller: Option<&Controller>,
    app: Option<&App>,
    store: &crate::understanding::Store,
) -> io::Result<()> {
    // 画面の復元用クエリはブラウザが読む。APIの経路は曖昧にしない。
    if request.method == "GET"
        && let Some((path, _)) = request.path.split_once('?')
        && matches!(path, "/" | "/architecture" | "/workbench" | "/user-trial")
    {
        request.path = path.to_owned();
    }
    if request.method == "GET" && !request.body.is_empty() {
        return json(
            stream,
            400,
            &serde_json::json!({"error": "GETに本文は指定できません"}),
        );
    }
    if request.method == "GET" && request.path == "/api/v1/capabilities" {
        let mut routes = vec![
            "case",
            "change",
            "compare",
            "backup",
            "profiles",
            "report",
            "report-html",
            "execution",
            "execute",
            "browser-screenshot",
            "architecture",
            "understand",
        ];
        if app.is_some() {
            routes.extend(["prepare", "ai", "system-diagram", "browser-recipes"]);
        }
        return json(
            stream,
            200,
            &serde_json::json!({
                "schema_version": 1,
                "api_version": "v1",
                "scope": "localhost",
                "routes": routes,
                "reported_observations": true,
                "limits": {
                    "request_bytes": MAX_BODY,
                    "backup_bytes": crate::backup::MAX_BACKUP,
                    "reader_request_bytes": MAX_BODY,
                    "reader_seconds": 30,
                    "reader_output_bytes": 8 * 1024 * 1024,
                },
                "understanding": "作者の説明と静的解析。実行経路・業務上の意味は未確認",
            }),
        );
    }
    if let Some(app) = app {
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/" | "/architecture") => {
                return html_response(stream, &app_html(token));
            }
            ("GET", "/workbench") => {
                return html_response(stream, &workbench_html(token));
            }
            ("GET", "/user-trial") => {
                return html_response(
                    stream,
                    &styled_app(
                        &assets::text("ui/web-trial.html").expect("画面の資材を読み取れません"),
                        token,
                    ),
                );
            }
            ("GET", "/ui/api.js" | "/ui/workbench-dom.js") => {
                return script_response(
                    stream,
                    &assets::text(request.path.trim_start_matches('/'))?,
                );
            }
            ("GET", "/ui/web-trial.js") => {
                return script_response(stream, &assets::text("ui/web-trial.js")?);
            }
            ("GET", "/ui/workbench.js") => {
                return script_response(stream, &assets::text("ui/workbench.js")?);
            }
            ("GET", "/ui/repository-diagram.js") => {
                return script_response(stream, &assets::text("ui/repository-diagram.js")?);
            }
            ("GET", "/ui/app.js") => {
                return script_response(stream, &assets::text("ui/app.js")?);
            }
            ("GET", "/api/v1/ai") => {
                return result_response(stream, Ok(crate::ai::status(app.ai.as_ref())));
            }
            ("GET", "/api/v1/browser-recipes") => {
                return result_response(stream, crate::browser_recipes::catalog());
            }
            ("POST", "/api/v1/system-diagram") => {
                let result = parse_body::<crate::system_diagram::Request>(&request.body)
                    .and_then(|r| crate::system_diagram::generate(app.ai.as_ref(), &app.store, r));
                return result_response(stream, result);
            }
            ("POST", "/api/v1/system-diagram/question") => {
                let result = parse_body::<crate::system_diagram::Question>(&request.body)
                    .and_then(|r| crate::system_diagram::answer(app.ai.as_ref(), &app.store, r));
                return result_response(stream, result);
            }
            ("POST", "/api/v1/system-diagram/dialogue/read") => {
                let result = parse_body::<crate::system_diagram::dialogue::Binding>(&request.body)
                    .and_then(|binding| crate::system_diagram::dialogue::read(&app.store, binding));
                return result_response(stream, result);
            }
            ("POST", "/api/v1/system-diagram/dialogue") => {
                let result = parse_body::<crate::system_diagram::dialogue::Message>(&request.body)
                    .and_then(|message| {
                        crate::system_diagram::dialogue::send(
                            app.ai.as_ref(),
                            app.dify.as_ref(),
                            &app.store,
                            directory,
                            message,
                        )
                    });
                return result_response(stream, result);
            }
            ("POST", "/api/v1/ai") => {
                let result = parse_body::<crate::ai::AiRequest>(&request.body)
                    .and_then(|r| crate::ai::explain(app.ai.as_ref(), &app.store, r));
                return result_response(stream, result);
            }
            ("POST", "/api/v1/prepare") => {
                let result = controller
                    .ok_or_else(|| io::Error::other("実行の設定がありません"))
                    .and_then(|c| {
                        parse_body::<crate::review_execution::PrepareRequest>(&request.body)
                            .and_then(|r| c.prepare(r))
                    });
                return result_response(stream, result);
            }
            _ => {}
        }
    }
    if request.method == "POST" && request.path == "/api/v1/repository" {
        return json(
            stream,
            410,
            &serde_json::json!({"error":"本文を受け取る読取APIは廃止しました。/api/v1/understandにリポジトリとコミットを指定してください"}),
        );
    }
    if request.method == "POST" && request.path == "/api/v1/understand" {
        let result = parse_body::<crate::understanding::UnderstandRequest>(&request.body)
            .and_then(|r| store.handle(r));
        return result_response(stream, result);
    }
    if request.method == "GET" && request.path == "/ui/repository-elk.js" {
        return respond_encoded(
            stream,
            200,
            "text/javascript; charset=utf-8",
            &assets::layout()?,
            "Content-Encoding: gzip\r\n",
        );
    }
    if request.method == "GET" && request.path == "/api/v1/architecture" {
        let result = workspace::load(directory).map(|w| {
            serde_json::json!({"revision":w.revision,"models":w.architecture_models,"links":crate::architecture::links(&w)})
        });
        return result_response(stream, result);
    }
    let api_path = request.path.as_str();
    if request.method == "GET" && api_path.starts_with("/api/v1/browser-screenshot/") {
        let parts = api_path
            .trim_start_matches("/api/v1/browser-screenshot/")
            .split('/')
            .collect::<Vec<_>>();
        let result = (|| {
            if parts.len() != 2 {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "画面写真の識別子が不正です",
                ));
            }
            let case = workspace::load(directory)?;
            let evidence = case
                .evidence
                .iter()
                .find(|e| e.id == parts[0])
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::NotFound, "対応する試行がありません")
                })?;
            let trace = evidence.execution.as_ref().ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "ブラウザの試行がありません")
            })?;
            crate::browser_evidence::screenshot(directory, trace, parts[1])
        })();
        return match result {
            Ok(bytes) => respond(stream, 200, "image/png", &bytes),
            Err(error) => json(stream, 404, &serde_json::json!({"error":error.to_string()})),
        };
    }
    match (request.method.as_str(), api_path) {
        ("GET", "/api/v1/profiles") => json(
            stream,
            200,
            &serde_json::json!({
                "catalog_version": crate::profile::CATALOG_VERSION,
                "templates": crate::profile::templates(),
                "notice": "基盤による確認候補です。担当者の回答・業務要件ではありません。モードの選択だけで安全性を判定しません。"
            }),
        ),
        ("GET", "/api/v1/report") => result_response(
            stream,
            workspace::load(directory).and_then(crate::review_report::build),
        ),
        ("GET" | "POST", "/api/v1/report-html") => {
            let result = (|| {
                let workspace = workspace::load(directory)?;
                if request.method == "POST" {
                    #[derive(Deserialize)]
                    #[serde(deny_unknown_fields)]
                    struct ReportRequest {
                        expected_revision: u64,
                    }
                    let expected: ReportRequest = parse_body(&request.body)?;
                    if expected.expected_revision != workspace.revision {
                        return Err(io::Error::new(
                            io::ErrorKind::AlreadyExists,
                            "事例が更新されています。確認資料を読み直してください",
                        ));
                    }
                }
                crate::review_report::html(&crate::review_report::build(workspace)?)
            })();
            match result {
                Ok(html) => html_response(stream, &html),
                Err(error) => result_response(stream, Err::<(), _>(error)),
            }
        }
        ("GET", "/api/v1/backup") => result_response(stream, crate::backup::snapshot(directory)),
        ("POST", "/api/v1/compare") => {
            let result = parse_body::<crate::comparison::CompareRequest>(&request.body).and_then(
                |request| crate::comparison::compare(workspace::load(directory)?, request),
            );
            result_response(stream, result)
        }
        ("GET", "/api/v1/case") => match workspace::load(directory) {
            Ok(case) => json(stream, 200, &view(case)),
            Err(error) => json(
                stream,
                400,
                &serde_json::json!({"error": error.to_string()}),
            ),
        },
        ("GET", "/api/v1/execution") => {
            let result =
                controller.map_or_else(|| Ok(ExecutionStatus::disabled()), Controller::status);
            result_response(stream, result)
        }
        ("POST", "/api/v1/execute") => {
            let result = controller
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "画面からの実行は起動時にリポジトリと方式を指定した場合だけ利用できます",
                    )
                })
                .and_then(|controller| {
                    parse_body::<ExecuteRequest>(&request.body)
                        .and_then(|request| controller.start(request))
                });
            match result {
                Ok(job) => json(stream, 202, &job),
                Err(error) => result_response(stream, Err::<Job, _>(error)),
            }
        }
        ("POST", "/api/v1/change") => {
            let result = parse_body::<ChangeRequest>(&request.body).and_then(|change| {
                if matches!(change.action, Change::Import { .. }) {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "試行の取り込みはCLIから行ってください",
                    ));
                }
                workspace::change(directory, change)
            });
            result_response(stream, result)
        }
        _ => json(
            stream,
            404,
            &serde_json::json!({"error": "この操作はありません"}),
        ),
    }
}
fn result_response<T: serde::Serialize>(
    stream: &mut Response,
    result: io::Result<T>,
) -> io::Result<()> {
    match result {
        Ok(value) => json(stream, 200, &value),
        Err(error) => {
            let error = crate::error::Failure::from(error);
            let status = error.status();
            json(
                stream,
                status,
                &serde_json::json!({"error": error.to_string()}),
            )
        }
    }
}
/// 理解と試験の画面を起動する。データの保存先に、事例・読取記録・取得したリポジトリ・試行を置く。
pub fn serve_app(
    data: PathBuf,
    port: u16,
    mode: crate::execution::ExecutionMode,
) -> io::Result<()> {
    std::fs::create_dir_all(&data)?;
    let case = data.join("case");
    if !case.exists() {
        workspace::initialize(
            &case,
            workspace::empty_workspace("app".into(), "期待する動作と試験の記録".into()),
            "基盤",
            "画面の初回起動で、空の事例を作成した",
        )?;
    }
    workspace::load(&case)?;
    let controller = Controller::managed(case.clone(), data.join("repositories"), mode)?;
    let app = App {
        store: crate::understanding::Store::new(data.join("snapshots"))?,
        ai: crate::ai::Config::for_app(&data)?,
        dify: crate::system_diagram::dialogue::DifyConfig::for_app(&data)?,
    };
    println!(
        "データの保存先: {}\nAIによる説明: {}",
        data.display(),
        crate::ai::status(app.ai.as_ref())["message"]
            .as_str()
            .unwrap_or_default()
    );
    serve_mode(case, port, Some(controller), None, Some(app))
}

pub fn serve_api(
    directory: PathBuf,
    port: u16,
    token_file: PathBuf,
    execution: Option<(PathBuf, crate::execution::ExecutionMode)>,
) -> io::Result<()> {
    workspace::load(&directory)?;
    let controller = execution
        .map(|(root, mode)| Controller::new(directory.clone(), root, mode))
        .transpose()?;
    serve_mode(directory, port, controller, Some(token_file), None)
}

fn serve_mode(
    directory: PathBuf,
    port: u16,
    controller: Option<Controller>,
    token_file: Option<PathBuf>,
    app: Option<App>,
) -> io::Result<()> {
    let mut bytes = [0; 24];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    let token = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    if let Some(path) = token_file {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(token.as_bytes())?;
        file.sync_all()?;
    }
    if app.is_some() {
        println!(
            "理解と試験の画面: http://{}\n期待・試行・判断の記録の保存先: {}\n終了: Ctrl+C。",
            listener.local_addr()?,
            directory.display()
        );
    } else {
        println!("API: http://{}\n終了: Ctrl+C。", listener.local_addr()?);
    }
    transport::serve(listener, directory, token, controller, app)
}
