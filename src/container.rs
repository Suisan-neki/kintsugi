//! 試験専用のコンテナを、試行ごとに作って止めて消す。
//! イメージはダイジェスト（`@sha256:` と64桁）付きだけを受け付ける。タグだけでは起動しない。
//! 待ち受けは 127.0.0.1 だけに公開する。Dockerが使えないときはほかの方式へ切り替えない。
use crate::error::invalid;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read, Write},
    net::{Ipv4Addr, Shutdown, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

static SESSION: AtomicU64 = AtomicU64::new(1);

pub const PORT_PREFIX: &str = "{{port:";
pub const MAX_READY_MS: u64 = 300_000;
const MAX_SERVICES: usize = 8;
const MAX_LOG: usize = 8192;
const MAX_BODY: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceRole {
    App,
    Database,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadyCheck {
    Tcp,
    Http {
        path: String,
        tls: bool,
    },
    /// コンテナ内でコマンドが成功するまで待つ（MariaDBの healthcheck など）。
    Exec {
        args: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VolumeMount {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Service {
    pub name: String,
    pub image: String,
    pub role: ServiceRole,
    /// コンテナ内の待ち受けポート。ホスト側は基盤が 127.0.0.1 の空きポートを割り当てる。
    pub expose: u16,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub volumes: Vec<VolumeMount>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web_build: Option<crate::prepared_web::WebBuild>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command: Vec<String>,
    pub ready: ReadyCheck,
    pub ready_timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContainerSpec {
    pub services: Vec<Service>,
    /// 対象のコンテナから外部への通信。指定しなければ遮断する。
    #[serde(default, skip_serializing_if = "Egress::is_blocked")]
    pub egress: Egress,
}

/// 試験用ネットワークの外部通信。`Allowed`は対象が外部へ通信できることを記録に残す。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Egress {
    #[default]
    Blocked,
    Allowed,
}
impl Egress {
    fn is_blocked(&self) -> bool {
        *self == Self::Blocked
    }
    fn label(self) -> &'static str {
        match self {
            Self::Blocked => "blocked",
            Self::Allowed => "allowed",
        }
    }
}

/// 遮断時に、各対象の前に置く中継コンテナ。ホストの127.0.0.1と対象、対象どうしの接続をここで通し、記録する。対象ではなく基盤が使う。
/// 対象の本体はネットワーク別名`<名前>-real`で動き、ほかの対象は元の名前で中継へ接続する。
const GATEWAY_IMAGE: &str =
    "nginx@sha256:4ff102c5d78d254a6f0da062b3cf39eaf07f01eec0927fd21e219d0af8bc0591";
const MAX_TRAFFIC_LOG: usize = 4 * 1024 * 1024;
const MAX_TRAFFIC_ROWS: usize = 64;
/// 対象のコンテナに残す権限。ほかはすべて外す。
const KEPT_CAPABILITIES: [&str; 6] = [
    "CHOWN",
    "DAC_OVERRIDE",
    "FOWNER",
    "SETGID",
    "SETUID",
    "NET_BIND_SERVICE",
];
const PIDS_LIMIT: &str = "2048";

/// コンテナへ基盤が行うHTTP操作。対象プロセスの起動とは別。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpOp {
    pub service: String,
    pub method: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServicePhase {
    pub name: String,
    pub role: ServiceRole,
    pub image: String,
    pub digest: String,
    pub container: String,
    pub port: u16,
    pub started: bool,
    pub ready: bool,
    pub ready_after_ms: Option<u64>,
    pub running_after_stop: bool,
    pub log_excerpt: String,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web_build: Option<crate::prepared_web::BuildBasis>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerPhase {
    pub engine: String,
    pub network: String,
    /// 外部通信の方針（blocked/allowed）。古い記録にはない。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub egress: String,
    /// 基盤が試験用ネットワークを読み出して、外部へ出る経路がないと確認できたか。
    #[serde(default)]
    pub egress_confirmed: bool,
    /// 中継が記録した、対象への接続の集計。古い記録にはない。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub traffic: Vec<TrafficSummary>,
    /// 通信の記録で観測できたことと、できなかったこと。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub traffic_note: String,

    pub volumes: Vec<String>,
    pub services: Vec<ServicePhase>,
    pub leftovers: Vec<String>,
    pub database_query: Option<String>,
    pub http: Option<HttpResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser: Option<crate::browser::BrowserReport>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub database_reads: Vec<crate::browser::DatabaseRead>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub http_reads: Vec<crate::http_probe::HttpRead>,
    pub detail: String,
}

/// 基盤から対象への接続を表す接続元の名前。ブラウザ操作器・HTTP操作・準備確認など。
pub const HOST_CLIENT: &str = "host（基盤・ブラウザ操作器）";

/// 接続元から接続先の対象への接続を集計したもの。通信の中身は含まない。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrafficSummary {
    pub from: String,
    pub to: String,
    /// 開始した接続の数（`unfinished`を含む）。
    pub connections: u32,
    /// 閉じた接続のうち、中継の状態が200以外だったもの。接続拒否だけでなく、クライアントの切断や中継停止を含む。アプリの処理の成否ではない。
    pub failed: u32,
    /// 開始したが、記録を取る時点までに閉じなかった接続。接続先が止まって待たされた接続を含む。
    #[serde(default)]
    pub unfinished: u32,
    pub bytes_from_client: u64,
    pub bytes_to_client: u64,
    pub first_at_ms: u64,
    pub last_at_ms: u64,
}

struct Addresses {
    /// 内部ネットワークのIPアドレス → 対象の名前
    clients: BTreeMap<String, String>,
    /// 中継のコンテナ名 → 外部向けネットワークのIPアドレス
    edges: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct TapLine {
    id: u64,
    t: f64,
    client: String,
    local: String,
    status: String,
    #[serde(rename = "in")]
    bytes_in: u64,
    out: u64,
}

fn record_connection(
    rows: &mut BTreeMap<(String, String), TrafficSummary>,
    from: &str,
    to: &str,
    entry: &TapLine,
) {
    let at = (entry.t * 1000.0) as u64;
    let row = rows
        .entry((from.to_owned(), to.to_owned()))
        .or_insert_with(|| TrafficSummary {
            from: from.to_owned(),
            to: to.to_owned(),
            connections: 0,
            failed: 0,
            unfinished: 0,
            bytes_from_client: 0,
            bytes_to_client: 0,
            first_at_ms: at,
            last_at_ms: at,
        });
    row.connections += 1;
    if entry.status != "200" {
        row.failed += 1;
    }
    row.bytes_from_client += entry.bytes_in;
    row.bytes_to_client += entry.out;
    row.first_at_ms = if row.first_at_ms == 0 {
        at
    } else {
        row.first_at_ms.min(at)
    };
    row.last_at_ms = row.last_at_ms.max(at);
}

fn record_unfinished(rows: &mut BTreeMap<(String, String), TrafficSummary>, from: &str, to: &str) {
    let row = rows
        .entry((from.to_owned(), to.to_owned()))
        .or_insert_with(|| TrafficSummary {
            from: from.to_owned(),
            to: to.to_owned(),
            connections: 0,
            failed: 0,
            unfinished: 0,
            bytes_from_client: 0,
            bytes_to_client: 0,
            first_at_ms: 0,
            last_at_ms: 0,
        });
    row.connections += 1;
    row.unfinished += 1;
}

/// 中継のエラーログ（infoレベル）にある「接続を受けた」行から、接続の番号と接続元のIPアドレスを読む。
fn started_connections(errors: &str) -> Vec<(u64, String)> {
    errors
        .lines()
        .filter_map(|line| {
            let rest = line.split_once(": *")?.1;
            let (id, rest) = rest.split_once(" client ")?;
            let (address, _) = rest.split_once(" connected to ")?;
            let ip = address.rsplit_once(':')?.0;
            Some((id.parse().ok()?, ip.to_owned()))
        })
        .collect()
}

/// コンテナが指定のネットワークで持つIPv4アドレス。
fn container_ip(container: &str, network: &str) -> Option<String> {
    let json = docker_text(&[
        "inspect",
        "--format",
        "{{json .NetworkSettings.Networks}}",
        container,
    ])
    .ok()?;
    let value: serde_json::Value = serde_json::from_str(&json).ok()?;
    let ip = value.get(network)?.get("IPAddress")?.as_str()?;
    (!ip.is_empty()).then(|| ip.to_owned())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpResult {
    pub method: String,
    pub url: String,
    pub status: Option<u16>,
    pub tls_insecure: bool,
    pub body_excerpt: String,
    pub detail: String,
}

pub struct Session {
    trial: String,
    network: String,
    edge: String,
    egress: Egress,
    egress_confirmed: bool,
    volumes: Vec<String>,
    services: Vec<Running>,
    engine: String,
    cleaned: bool,
}

struct Running {
    spec: Service,
    container: String,
    port: u16,
    digest: String,
    started: bool,
    ready: bool,
    ready_after_ms: Option<u64>,
    detail: String,
    web_build: Option<crate::prepared_web::BuildBasis>,
    tap: Option<String>,
}

pub fn image_digest(image: &str) -> Option<&str> {
    let rest = image.rsplit_once("@sha256:")?.1;
    (rest.len() == 64
        && rest
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()))
    .then_some(rest)
}

fn token(value: &str, what: &str, max: usize) -> io::Result<()> {
    if value.is_empty()
        || value.len() > max
        || !value.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        || !value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(invalid(format!(
            "{what}は、英小文字で始まり英小文字・数字・ハイフンだけの{max}文字以内です"
        )));
    }
    Ok(())
}

pub fn validate_spec(spec: &ContainerSpec) -> io::Result<()> {
    if spec.services.is_empty() || spec.services.len() > MAX_SERVICES {
        return Err(invalid(format!(
            "コンテナは1〜{MAX_SERVICES}件で指定してください"
        )));
    }
    let mut names = std::collections::BTreeSet::new();
    let mut databases = 0;
    for service in &spec.services {
        if let Some(build) = &service.web_build {
            crate::prepared_web::validate(build)?;
        }
        token(&service.name, "コンテナの名前", 32)?;
        if !names.insert(service.name.as_str()) {
            return Err(invalid("コンテナの名前が重複しています"));
        }
        if image_digest(&service.image).is_none()
            || service.image.len() > 256
            || service.image.contains('\0')
            || service.image.chars().any(char::is_whitespace)
        {
            return Err(invalid(format!(
                "イメージ {} はタグだけではなく、@sha256: と64桁のダイジェストを付けて指定してください",
                service.image
            )));
        }
        if service.expose == 0 {
            return Err(invalid("コンテナ内の待ち受けポートを指定してください"));
        }
        if !(1..=MAX_READY_MS).contains(&service.ready_timeout_ms) {
            return Err(invalid("準備の待機上限は1〜300000ミリ秒です"));
        }
        if service.env.len() > 32
            || service.volumes.len() > 8
            || service.command.len() > 16
            || service
                .command
                .iter()
                .any(|a| a.len() > 1024 || a.contains('\0'))
        {
            return Err(invalid("コンテナの環境変数・ボリューム・引数が多すぎます"));
        }
        for (key, value) in &service.env {
            if key.is_empty()
                || key.len() > 64
                || !key
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
                || value.len() > 4096
                || value.contains('\0')
            {
                return Err(invalid("環境変数の名前と値を確認してください"));
            }
        }
        for volume in &service.volumes {
            token(&volume.name, "ボリュームの名前", 32)?;
            if volume.path.is_empty()
                || !volume.path.starts_with('/')
                || volume.path.contains('\0')
                || volume.path.len() > 256
            {
                return Err(invalid("ボリュームのコンテナ内パスは / から始めてください"));
            }
        }
        match &service.ready {
            ReadyCheck::Http { path, .. } => http_path(path)?,
            ReadyCheck::Exec { args } => {
                if args.is_empty()
                    || args.len() > 16
                    || args
                        .iter()
                        .any(|a| a.is_empty() || a.len() > 256 || a.contains('\0'))
                {
                    return Err(invalid("準備確認のコマンドを確認してください"));
                }
            }
            ReadyCheck::Tcp => {}
        }
        if service.role == ServiceRole::Database {
            databases += 1;
        }
    }
    if spec.egress == Egress::Blocked
        && spec
            .services
            .iter()
            .any(|s| names.contains(format!("{}-real", s.name).as_str()))
    {
        return Err(invalid("コンテナの名前が中継用の接続先名と重複しています"));
    }
    if databases > 1 {
        return Err(invalid("データベースの役割のコンテナは1件にしてください"));
    }
    Ok(())
}

pub fn validate_http(op: &HttpOp, spec: &ContainerSpec) -> io::Result<()> {
    if !["GET", "POST", "PUT"].contains(&op.method.as_str()) {
        return Err(invalid("HTTPの操作は GET・POST・PUT のいずれかです"));
    }
    http_path(&op.path)?;
    if spec.services.iter().all(|s| s.name != op.service) {
        return Err(invalid(format!(
            "HTTPの操作先 {} は、起動するコンテナの名前と一致しません",
            op.service
        )));
    }
    if let Some(file) = &op.body_file
        && (file.is_empty()
            || file.len() > 1024
            || file.contains('\0')
            || Path::new(file).is_absolute())
    {
        return Err(invalid("HTTPの本文ファイルは固定版の中の相対パスです"));
    }
    if op
        .content_type
        .as_ref()
        .is_some_and(|t| t.len() > 128 || t.contains('\0'))
    {
        return Err(invalid("Content-Type を確認してください"));
    }
    Ok(())
}

fn http_path(path: &str) -> io::Result<()> {
    if !path.starts_with('/')
        || path.len() > 1024
        || path.contains('\0')
        || path
            .chars()
            .any(|c| c.is_ascii_control() || c.is_whitespace())
    {
        return Err(invalid("HTTPのパスは / から始まる空白なしの文字列です"));
    }
    Ok(())
}

/// Dockerのエンジンに接続できるか。失敗してもほかの実行方式へは切り替えない。
pub fn engine() -> Result<String, String> {
    let output = docker(&["version", "--format", "{{.Server.Version}}"]).map_err(|e| {
        format!("Dockerのコマンドを起動できません: {e}。コンテナの試行はほかの方式へ切り替えません")
    })?;
    if !output.status.success() {
        return Err(format!(
            "Dockerのエンジンに接続できません: {}。コンテナの試行はほかの方式へ切り替えません",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let version = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if version.is_empty() {
        return Err(
            "Dockerのエンジンの版を取得できません。コンテナの試行はほかの方式へ切り替えません"
                .into(),
        );
    }
    Ok(version)
}

fn docker(args: &[&str]) -> io::Result<Output> {
    let mut command = Command::new("docker");
    command
        .args(args)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .stdin(Stdio::null());
    if let Some(host) = std::env::var_os("DOCKER_HOST") {
        command.env("DOCKER_HOST", host);
    }
    command.output()
}

fn docker_text(args: &[&str]) -> io::Result<String> {
    let output = docker(args)?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        Err(io::Error::other(format!(
            "docker {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

pub fn placeholder(name: &str) -> String {
    format!("{PORT_PREFIX}{name}}}")
}

pub fn substitute_ports(value: &str, ports: &BTreeMap<String, u16>) -> String {
    let mut out = value.to_owned();
    for (name, port) in ports {
        out = out.replace(&placeholder(name), &port.to_string());
    }
    out
}

impl Session {
    pub fn start(trial: &str, spec: &ContainerSpec) -> io::Result<Self> {
        validate_spec(spec)?;
        let engine = engine().map_err(io::Error::other)?;
        let trial = format!(
            "{}-{}",
            sanitize_trial(trial),
            SESSION.fetch_add(1, Ordering::Relaxed)
        );
        let network = format!("eel-{trial}");
        let mut session = Self {
            trial: trial.clone(),
            network: network.clone(),
            edge: String::new(),
            egress: spec.egress,
            egress_confirmed: false,
            volumes: vec![],
            services: vec![],
            engine,
            cleaned: false,
        };
        let mut create = vec!["network", "create", "--label", "eel.managed=1"];
        let label = format!("eel.trial={trial}");
        create.extend(["--label", &label]);
        if spec.egress == Egress::Blocked {
            create.push("--internal");
        }
        create.push(&network);
        docker_text(&create)?;
        if spec.egress == Egress::Blocked {
            let internal =
                docker_text(&["network", "inspect", "--format", "{{.Internal}}", &network]);
            session.egress_confirmed = internal.is_ok_and(|v| v == "true");
            if !session.egress_confirmed {
                let _ = session.cleanup();
                return Err(io::Error::other(
                    "試験用ネットワークが外部通信を遮断する設定になっていません。実行を止めました",
                ));
            }
            let edge = format!("{network}-edge");
            docker_text(&[
                "network",
                "create",
                "--label",
                "eel.managed=1",
                "--label",
                &label,
                &edge,
            ])?;
            session.edge = edge;
        }
        let mut volume_names = std::collections::BTreeSet::new();
        for service in &spec.services {
            for volume in &service.volumes {
                if volume_names.insert(volume.name.as_str()) {
                    let full = format!("eel-{trial}-{}", volume.name);
                    docker_text(&[
                        "volume",
                        "create",
                        "--label",
                        "eel.managed=1",
                        "--label",
                        &format!("eel.trial={trial}"),
                        &full,
                    ])?;
                    session.volumes.push(full);
                }
            }
        }
        let mut ordered: Vec<&Service> = spec.services.iter().collect();
        ordered.sort_by_key(|s| match s.role {
            ServiceRole::Database => 0,
            ServiceRole::Other => 1,
            ServiceRole::App => 2,
        });
        for service in ordered {
            match session.run(service) {
                Ok(()) => {
                    if session.services.last().is_some_and(|s| !s.ready) {
                        break;
                    }
                }
                Err(error) => {
                    let _ = session.cleanup();
                    return Err(error);
                }
            }
        }
        Ok(session)
    }

    fn run(&mut self, service: &Service) -> io::Result<()> {
        let assets = service
            .web_build
            .as_ref()
            .map(|build| {
                let root = std::env::var_os("EEL_PREPARED_ENVIRONMENTS").ok_or_else(|| {
                    invalid("ビルド済みファイルを使うにはEEL_PREPARED_ENVIRONMENTSが必要です")
                })?;
                crate::prepared_web::snapshot(Path::new(&root), build)
            })
            .transpose()?;
        let digest = pull(&service.image)?;
        let port = crate::server::free_port()?;
        let container = format!("eel-{}-{}", self.trial, service.name);
        let mut args = vec![
            "run".into(),
            "-d".into(),
            "--name".into(),
            container.clone(),
            "--label".into(),
            "eel.managed=1".into(),
            "--label".into(),
            format!("eel.trial={}", self.trial),
            "--label".into(),
            format!("eel.role={}", role_label(service.role)),
            "--network".into(),
            self.network.clone(),
            "--network-alias".into(),
            self.alias(&service.name),
            "--restart".into(),
            "no".into(),
        ];
        if self.egress == Egress::Allowed {
            args.push("--publish".into());
            args.push(format!("127.0.0.1:{port}:{}", service.expose));
        }
        harden(&mut args);
        for (key, value) in &service.env {
            args.push("--env".into());
            args.push(format!("{key}={value}"));
        }
        for volume in &service.volumes {
            args.push("--mount".into());
            args.push(format!(
                "type=volume,source=eel-{}-{},target={}",
                self.trial, volume.name, volume.path
            ));
        }
        args.push(service.image.clone());
        args.extend(service.command.iter().cloned());
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        if let Some(assets) = &assets {
            let mut create = vec!["create"];
            create.extend(arg_refs.iter().copied().skip(2));
            docker_text(&create)?;
            let copied = docker_text(&[
                "cp",
                &format!("{}/.", assets.directory.display()),
                &format!("{container}:/usr/share/nginx/html"),
            ])
            .and_then(|_| docker_text(&["start", &container]));
            if let Err(error) = copied {
                let _ = docker(&["rm", "-f", &container]);
                return Err(error);
            }
        } else {
            docker_text(&arg_refs)?;
        }
        let tap = if self.egress == Egress::Blocked {
            match self.start_tap(service, port) {
                Ok(name) => Some(name),
                Err(error) => {
                    let _ = docker(&["rm", "-f", &container]);
                    return Err(error);
                }
            }
        } else {
            None
        };
        let mut running = Running {
            spec: service.clone(),
            container,
            port,
            digest,
            started: true,
            ready: false,
            ready_after_ms: None,
            detail: String::new(),
            web_build: assets.as_ref().map(|s| s.basis.clone()),
            tap,
        };
        match wait_ready(&running) {
            Ok(ms) => {
                running.ready = true;
                running.ready_after_ms = Some(ms);
                running.detail = format!("127.0.0.1:{port} で準備できた（{ms}ms）");
            }
            Err(error) => {
                running.detail = error.to_string();
                self.services.push(running);
                return Ok(());
            }
        }
        self.services.push(running);
        Ok(())
    }

    /// 対象の本体のネットワーク別名。遮断時は中継が元の名前を受け持つ。
    fn alias(&self, name: &str) -> String {
        match self.egress {
            Egress::Blocked => format!("{name}-real"),
            Egress::Allowed => name.to_owned(),
        }
    }

    /// 対象の前に置く中継。ホストの127.0.0.1と対象、対象どうしの接続を通し、接続ごとの結果を標準出力へ残す。
    fn start_tap(&self, service: &Service, port: u16) -> io::Result<String> {
        pull(GATEWAY_IMAGE)?;
        let name = format!("eel-{}-{}-tap", self.trial, service.name);
        let config = format!(
            "user root;pid /tmp/tap.pid;error_log /dev/stderr info;events{{}}stream{{resolver 127.0.0.11 valid=1s;resolver_timeout 2s;log_format eel escape=json \"{{\\\"id\\\":$connection,\\\"t\\\":$msec,\\\"client\\\":\\\"$remote_addr\\\",\\\"local\\\":\\\"$server_addr\\\",\\\"status\\\":\\\"$status\\\",\\\"in\\\":$bytes_received,\\\"out\\\":$bytes_sent}}\";access_log /dev/stdout eel;server{{listen {expose};set $upstream {real}:{expose};proxy_half_close on;proxy_pass $upstream;}}}}",
            expose = service.expose,
            real = self.alias(&service.name),
        );
        let script = format!(
            "printf '%s' '{config}' > /tmp/tap.conf && exec nginx -c /tmp/tap.conf -g 'daemon off;'"
        );
        let mut args: Vec<String> = vec![
            "create".into(),
            "--name".into(),
            name.clone(),
            "--label".into(),
            "eel.managed=1".into(),
            "--label".into(),
            format!("eel.trial={}", self.trial),
            "--label".into(),
            "eel.role=tap".into(),
            "--network".into(),
            self.edge.clone(),
            "--publish".into(),
            format!("127.0.0.1:{port}:{}", service.expose),
            "--restart".into(),
            "no".into(),
        ];
        harden(&mut args);
        args.extend([
            "--entrypoint".into(),
            "sh".into(),
            GATEWAY_IMAGE.into(),
            "-c".into(),
            script,
        ]);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        docker_text(&refs)?;
        let started = docker_text(&[
            "network",
            "connect",
            "--alias",
            &service.name,
            &self.network,
            &name,
        ])
        .and_then(|_| docker_text(&["start", &name]));
        if let Err(error) = started {
            let _ = docker(&["rm", "-f", &name]);
            return Err(error);
        }
        Ok(name)
    }

    /// 中継が記録した接続を、接続元と接続先の対象ごとに集計する。
    fn traffic(&self, addresses: &Addresses) -> (Vec<TrafficSummary>, String) {
        if self.egress == Egress::Allowed {
            return (
                vec![],
                "外部通信を許可した試行では、対象どうしの通信を記録していない".into(),
            );
        }
        let clients = &addresses.clients;
        let mut rows: BTreeMap<(String, String), TrafficSummary> = BTreeMap::new();
        let mut unparsed = 0usize;
        let mut truncated = false;
        let mut unavailable = vec![];
        for service in &self.services {
            let Some(tap) = &service.tap else { continue };
            let edge_ip = addresses.edges.get(tap);
            let output = match docker(&["logs", tap]) {
                Ok(output) if output.status.success() => output,
                _ => {
                    unavailable.push(service.spec.name.as_str());
                    continue;
                }
            };
            let text = String::from_utf8_lossy(&output.stdout);
            let errors = String::from_utf8_lossy(&output.stderr);
            if text.len() > MAX_TRAFFIC_LOG || errors.len() > MAX_TRAFFIC_LOG {
                truncated = true;
            }
            let text: String = text.chars().take(MAX_TRAFFIC_LOG).collect();
            let errors: String = errors.chars().take(MAX_TRAFFIC_LOG).collect();
            let mut finished = std::collections::BTreeSet::new();
            for line in text.lines().filter(|l| l.starts_with('{')) {
                match serde_json::from_str::<TapLine>(line) {
                    Ok(entry) => {
                        finished.insert(entry.id);
                        let from = match clients.get(&entry.client) {
                            Some(name) => name.clone(),
                            None if edge_ip.is_some_and(|ip| *ip == entry.local) => {
                                HOST_CLIENT.to_owned()
                            }
                            None => format!("不明（{}）", entry.client),
                        };
                        record_connection(&mut rows, &from, &service.spec.name, &entry);
                    }
                    Err(_) => unparsed += 1,
                }
            }
            for (id, client) in started_connections(&errors) {
                if !finished.contains(&id) {
                    let from = clients
                        .get(&client)
                        .cloned()
                        .unwrap_or_else(|| format!("不明（{client}）"));
                    record_unfinished(&mut rows, &from, &service.spec.name);
                }
            }
        }
        let mut summary: Vec<TrafficSummary> = rows.into_values().collect();
        let omitted_rows = summary.len().saturating_sub(MAX_TRAFFIC_ROWS);
        summary.truncate(MAX_TRAFFIC_ROWS);
        let mut note = String::from(
            "中継を通った接続（接続元・接続先の対象、中継の状態、バイト数）を記録した。失敗件数は中継の状態が200以外の接続で、クライアントの切断や中継停止も含み、アプリの処理の成否ではない。閉じなかった接続は件数だけで、成否とバイト数は分からない。中継を通らない通信と、通信の中身は記録していない",
        );
        if !unavailable.is_empty() {
            note.push_str(&format!(
                "。中継ログを取得できず未観測の対象: {}",
                unavailable.join("、")
            ));
        }
        if omitted_rows > 0 {
            note.push_str(&format!("。集計の上限により{omitted_rows}組を省略した"));
        }
        if unparsed > 0 {
            note.push_str(&format!("。読み取れない行が{unparsed}件あった"));
        }
        if truncated {
            note.push_str("。ログが上限を超えたため後半を省略した");
        }
        (summary, note)
    }

    /// 中継と対象を止める前に、IPアドレスと名前の対応を読む。止めた後は読めない。
    fn addresses(&self) -> Addresses {
        Addresses {
            clients: self
                .services
                .iter()
                .filter_map(|s| {
                    container_ip(&s.container, &self.network).map(|ip| (ip, s.spec.name.clone()))
                })
                .collect(),
            edges: self
                .services
                .iter()
                .filter_map(|s| {
                    let tap = s.tap.as_ref()?;
                    Some((tap.clone(), container_ip(tap, &self.edge)?))
                })
                .collect(),
        }
    }

    pub fn ports(&self) -> BTreeMap<String, u16> {
        self.services
            .iter()
            .map(|s| (s.spec.name.clone(), s.port))
            .collect()
    }

    pub fn port_of(&self, name: &str) -> io::Result<u16> {
        self.services
            .iter()
            .find(|s| s.spec.name == name)
            .map(|s| s.port)
            .ok_or_else(|| invalid(format!("コンテナ {name} のポートがありません")))
    }

    pub fn all_ready(&self) -> bool {
        self.services.iter().all(|s| s.ready)
    }

    pub fn stop_database(&mut self) -> io::Result<bool> {
        let Some(index) = self
            .services
            .iter()
            .position(|s| s.spec.role == ServiceRole::Database)
        else {
            return Err(invalid("データベースの役割のコンテナがありません"));
        };
        docker_text(&["stop", "-t", "5", &self.services[index].container])?;
        let running = inspect_running(&self.services[index].container)?;
        self.services[index].detail = if running {
            "データベースのコンテナを止めたが、まだ動いている".into()
        } else {
            "データベースのコンテナを止めた".into()
        };
        Ok(!running)
    }

    pub fn disconnect_database(&mut self) -> io::Result<bool> {
        let Some(index) = self
            .services
            .iter()
            .position(|s| s.spec.role == ServiceRole::Database)
        else {
            return Err(invalid("データベースの役割のコンテナがありません"));
        };
        docker_text(&[
            "network",
            "disconnect",
            &self.network,
            &self.services[index].container,
        ])?;
        let connected = inspect_network(&self.services[index].container, &self.network)?;
        self.services[index].detail = if connected {
            "データベースを試験用ネットワークから外したが、まだ接続されている".into()
        } else {
            "データベースを試験用ネットワークから外した".into()
        };
        Ok(!connected)
    }

    pub fn remount_database_read_only(&mut self) -> io::Result<bool> {
        let index = self.recreate_database(true)?;
        let probe_path = format!(
            "{}/.eel-write-probe",
            self.services[index].spec.volumes[0].path
        );
        let writable = self.writable(index, &probe_path);
        self.services[index].detail = if writable {
            "保存先のボリュームを読み取り専用で載せ直したが、コンテナ内から書き込めた".into()
        } else {
            "保存先のボリュームを読み取り専用で載せ直した".into()
        };
        Ok(!writable)
    }

    fn writable(&self, index: usize, probe_path: &str) -> bool {
        docker(&["exec", &self.services[index].container, "touch", probe_path])
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// データベースのコンテナを消して、同じボリュームで作り直す。`read_only`なら読み取り専用で載せる。
    fn recreate_database(&mut self, read_only: bool) -> io::Result<usize> {
        let Some(index) = self
            .services
            .iter()
            .position(|s| s.spec.role == ServiceRole::Database)
        else {
            return Err(invalid("データベースの役割のコンテナがありません"));
        };
        if self.services[index].spec.volumes.is_empty() {
            return Err(invalid("読み取り専用にするボリュームがありません"));
        }
        let name = self.services[index].spec.name.clone();
        docker_text(&["rm", "-f", &self.services[index].container])?;
        let spec = self.services[index].spec.clone();
        let port = self.services[index].port;
        let container = self.services[index].container.clone();
        let mut args = vec![
            "run".into(),
            "-d".into(),
            "--name".into(),
            container.clone(),
            "--label".into(),
            "eel.managed=1".into(),
            "--label".into(),
            format!("eel.trial={}", self.trial),
            "--network".into(),
            self.network.clone(),
            "--network-alias".into(),
            self.alias(&name),
            "--restart".into(),
            "no".into(),
        ];
        if self.egress == Egress::Allowed {
            args.push("--publish".into());
            args.push(format!("127.0.0.1:{port}:{}", spec.expose));
        }
        harden(&mut args);
        for (key, value) in &spec.env {
            args.push("--env".into());
            args.push(format!("{key}={value}"));
        }
        for volume in &spec.volumes {
            args.push("--mount".into());
            args.push(format!(
                "type=volume,source=eel-{}-{},target={}{}",
                self.trial,
                volume.name,
                volume.path,
                if read_only { ",readonly" } else { "" }
            ));
        }
        args.push(spec.image.clone());
        args.extend(spec.command.iter().cloned());
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        docker_text(&arg_refs)?;
        Ok(index)
    }

    /// 故障を解除する。解除後の状態を読み出して確かめ、確かめられた場合だけ`true`を返す。
    pub fn restore_database(&mut self, fault: crate::trial::TrialFault) -> io::Result<bool> {
        use crate::trial::TrialFault;
        let Some(index) = self
            .services
            .iter()
            .position(|s| s.spec.role == ServiceRole::Database)
        else {
            return Err(invalid("データベースの役割のコンテナがありません"));
        };
        let (effective, detail) = match fault {
            TrialFault::DatabaseStopped => {
                docker_text(&["start", &self.services[index].container])?;
                let running = inspect_running(&self.services[index].container)?;
                (running, "データベースのコンテナを起動し直した")
            }
            TrialFault::DatabaseUnreachable => {
                let alias = self.alias(&self.services[index].spec.name);
                docker_text(&[
                    "network",
                    "connect",
                    "--alias",
                    &alias,
                    &self.network,
                    &self.services[index].container,
                ])?;
                let connected = inspect_network(&self.services[index].container, &self.network)?;
                (connected, "データベースを試験用ネットワークへ接続し直した")
            }
            TrialFault::VolumeReadOnly => {
                let index = self.recreate_database(false)?;
                let probe_path = format!(
                    "{}/.eel-write-probe",
                    self.services[index].spec.volumes[0].path
                );
                // 起動直後はまだ受け付けないことがあるため、書き込めるまで短く待つ。
                let writable = (0..20).any(|_| {
                    let ok = self.writable(index, &probe_path);
                    if !ok {
                        thread::sleep(Duration::from_millis(500));
                    }
                    ok
                });
                (writable, "保存先のボリュームを書き込み可能で載せ直した")
            }
            _ => return Err(invalid("この故障には復旧の操作がありません")),
        };
        self.services[index].detail = if effective {
            format!("{detail}。解除後の状態を確かめた")
        } else {
            format!("{detail}が、解除後の状態を確かめられなかった")
        };
        Ok(effective)
    }

    /// 復旧後に、各コンテナが準備できるまで待つ。準備できなかったことも記録する。
    pub fn settle(&mut self, timeout_ms: u64) -> Vec<crate::trial::RecoveredService> {
        let timeout = Duration::from_millis(timeout_ms);
        let mut out = vec![];
        for service in &mut self.services {
            let result = wait_ready_for(service, timeout);
            let (ready, ms, detail) = match result {
                Ok(ms) => (true, Some(ms), format!("復旧後に準備できた（{ms}ms）")),
                Err(error) => (false, None, error.to_string()),
            };
            service.ready = ready;
            out.push(crate::trial::RecoveredService {
                name: service.spec.name.clone(),
                ready,
                ready_after_ms: ms,
                detail,
            });
        }
        out
    }

    pub fn query_database(&self) -> Option<String> {
        let service = self
            .services
            .iter()
            .find(|s| s.spec.role == ServiceRole::Database)?;
        if !inspect_running(&service.container).ok()? {
            return Some("データベースのコンテナは動いていない".into());
        }
        let password = service
            .spec
            .env
            .get("MYSQL_ROOT_PASSWORD")
            .or_else(|| service.spec.env.get("MARIADB_ROOT_PASSWORD"));
        let env = format!("MYSQL_PWD={}", password.map(String::as_str).unwrap_or(""));
        let output = docker(&[
            "exec",
            "--env",
            &env,
            &service.container,
            "mariadb",
            "--user=root",
            "--connect-timeout=5",
            "-N",
            "-e",
            "SELECT 1",
        ])
        .ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Some(if output.status.success() {
            format!("基盤がデータベースへ SELECT 1 を送り、応答は {stdout}")
        } else {
            format!(
                "基盤がデータベースへ SELECT 1 を送ったが失敗した: {}",
                if stderr.is_empty() { stdout } else { stderr }
            )
        })
    }

    /// MariaDBの読取専用照会。対象の試験値に一致する行だけを読む。
    pub fn read_database(
        &self,
        probe: &crate::browser::DatabaseProbe,
        test_value: &str,
    ) -> crate::browser::DatabaseRead {
        let mut read = crate::browser::DatabaseRead {
            id: probe.id.clone(),
            performed: false,
            exit_code: None,
            text: String::new(),
            detail: "DBの行を読み出していない".into(),
        };
        let Some(service) = self
            .services
            .iter()
            .find(|s| s.spec.name == probe.service && s.spec.role == ServiceRole::Database)
        else {
            return read;
        };
        if !inspect_running(&service.container).unwrap_or(false) {
            read.detail = "DBが停止しており、保存した行を読めない".into();
            return read;
        }
        if ![&probe.database, &probe.table, &probe.match_column]
            .iter()
            .all(|s| crate::browser::sql_identifier(s))
            || probe.columns.is_empty()
            || !probe
                .columns
                .iter()
                .all(|s| crate::browser::sql_identifier(s))
        {
            read.detail = "DBの識別子が不正".into();
            return read;
        }
        let password = service
            .spec
            .env
            .get("MYSQL_ROOT_PASSWORD")
            .or_else(|| service.spec.env.get("MARIADB_ROOT_PASSWORD"));
        let env = format!("MYSQL_PWD={}", password.map(String::as_str).unwrap_or(""));
        let sql = crate::browser::query(probe, test_value);
        match docker(&[
            "exec",
            "--env",
            &env,
            &service.container,
            "mariadb",
            "--user=root",
            "--connect-timeout=5",
            "--batch",
            "--skip-column-names",
            "--database",
            &probe.database,
            "--execute",
            &sql,
        ]) {
            Ok(output) => {
                read.performed = true;
                read.exit_code = output.status.code();
                if output.status.success() && output.stdout.len() <= 8192 {
                    read.text = String::from_utf8_lossy(&output.stdout).into_owned();
                    read.detail = "基盤が試験値に一致するDBの行を直接読み出した".into();
                } else {
                    read.exit_code = read.exit_code.filter(|c| *c != 0);
                    read.detail =
                        "DBの読取が失敗したか出力上限に達した。行の不存在とは扱わない".into();
                }
            }
            Err(_) => read.detail = "DBの読取を起動できない".into(),
        }
        read
    }

    pub fn request(&self, op: &HttpOp, snapshot: &Path, timeout_ms: u64) -> io::Result<HttpResult> {
        let running = self
            .services
            .iter()
            .find(|s| s.spec.name == op.service)
            .ok_or_else(|| invalid(format!("操作先のコンテナ {} がありません", op.service)))?;
        let tls = matches!(running.spec.ready, ReadyCheck::Http { tls: true, .. });
        let body = match &op.body_file {
            Some(path) => Some(fs::read(snapshot.join(path))?),
            None => None,
        };
        if body.as_ref().is_some_and(|b| b.len() > MAX_BODY) {
            return Err(invalid("HTTPの本文が1MiBを超えています"));
        }
        http_request(
            running.port,
            tls,
            &op.method,
            &op.path,
            op.content_type.as_deref(),
            body.as_deref(),
            timeout_ms,
        )
    }

    pub fn snapshot(&self) -> ContainerPhase {
        let addresses = self.addresses();
        if self.egress == Egress::Blocked {
            for service in &self.services {
                if let Some(tap) = &service.tap {
                    let _ = docker(&["stop", "-t", "2", tap]);
                }
            }
        }
        let (traffic, traffic_note) = self.traffic(&addresses);
        for service in &self.services {
            let _ = docker(&["stop", "-t", "2", &service.container]);
        }
        let leftovers = labeled(&self.trial);
        ContainerPhase {
            engine: self.engine.clone(),
            network: self.network.clone(),
            egress: self.egress.label().into(),
            egress_confirmed: self.egress_confirmed,
            traffic,
            traffic_note,
            volumes: self.volumes.clone(),
            services: self
                .services
                .iter()
                .map(|s| ServicePhase {
                    name: s.spec.name.clone(),
                    role: s.spec.role,
                    image: s.spec.image.clone(),
                    digest: s.digest.clone(),
                    container: s.container.clone(),
                    port: s.port,
                    started: s.started,
                    ready: s.ready,
                    ready_after_ms: s.ready_after_ms,
                    running_after_stop: inspect_running(&s.container).unwrap_or(false),
                    log_excerpt: logs(&s.container),
                    detail: s.detail.clone(),
                    web_build: s.web_build.clone(),
                })
                .collect(),
            leftovers: leftovers.clone(),
            database_query: None,
            http: None,
            browser: None,
            database_reads: vec![],
            http_reads: vec![],
            detail: if leftovers.is_empty() {
                "試行用のコンテナ・ネットワーク・ボリュームをこの試行だけで使っている".into()
            } else {
                format!("基盤が管理する試験用の資源: {}", leftovers.join("、"))
            },
        }
    }

    pub fn cleanup(&mut self) -> io::Result<Vec<String>> {
        for service in self.services.iter().rev() {
            if let Some(tap) = &service.tap {
                let _ = docker(&["rm", "-f", tap]);
            }
            let _ = docker(&["rm", "-f", &service.container]);
        }
        if !self.edge.is_empty() {
            let _ = docker(&["network", "rm", &self.edge]);
        }
        if !self.network.is_empty() {
            let _ = docker(&["network", "rm", &self.network]);
        }
        for volume in self.volumes.iter().rev() {
            let _ = docker(&["volume", "rm", volume]);
        }
        self.cleaned = true;
        let leftovers = labeled(&self.trial);
        if leftovers.is_empty() {
            Ok(leftovers)
        } else {
            Err(io::Error::other(format!(
                "試行の後に試験用のコンテナまたはボリュームが残った: {}",
                leftovers.join("、")
            )))
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if !self.cleaned {
            let _ = self.cleanup();
        }
    }
}

/// 対象とゲートウェイのコンテナから、既定で付く権限を減らす。
fn harden(args: &mut Vec<String>) {
    args.extend(
        [
            "--security-opt",
            "no-new-privileges",
            "--cap-drop",
            "ALL",
            "--pids-limit",
            PIDS_LIMIT,
        ]
        .map(String::from),
    );
    for capability in KEPT_CAPABILITIES {
        args.push("--cap-add".into());
        args.push(capability.into());
    }
}

fn role_label(role: ServiceRole) -> &'static str {
    match role {
        ServiceRole::App => "app",
        ServiceRole::Database => "database",
        ServiceRole::Other => "other",
    }
}

fn sanitize_trial(trial: &str) -> String {
    let trimmed = trial.trim_start_matches("run-");
    trimmed
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .take(40)
        .collect()
}

fn pull(image: &str) -> io::Result<String> {
    docker_text(&["pull", "--quiet", image])?;
    let digest = docker_text(&[
        "image",
        "inspect",
        "--format",
        "{{if .RepoDigests}}{{index .RepoDigests 0}}{{else}}{{.Id}}{{end}}",
        image,
    ])?;
    if digest.is_empty() {
        return Err(io::Error::other("イメージのダイジェストを記録できません"));
    }
    Ok(digest)
}

fn wait_ready(running: &Running) -> io::Result<u64> {
    wait_ready_for(
        running,
        Duration::from_millis(running.spec.ready_timeout_ms),
    )
}

fn wait_ready_for(running: &Running, timeout: Duration) -> io::Result<u64> {
    let started = Instant::now();
    loop {
        if !inspect_running(&running.container)? {
            return Err(io::Error::other(format!(
                "コンテナ {} が準備の前に終了した: {}",
                running.spec.name,
                logs(&running.container)
            )));
        }
        let ok = match &running.spec.ready {
            ReadyCheck::Tcp => {
                crate::server::listening(running.port)
                    && (running.tap.is_none() || target_holds_connection(running.port))
            }
            ReadyCheck::Http { path, tls } => {
                http_request(running.port, *tls, "GET", path, None, None, 5000)
                    .ok()
                    .and_then(|r| r.status)
                    .is_some_and(|s| (200..400).contains(&s))
            }
            ReadyCheck::Exec { args } => {
                let mut cmd = vec!["exec", running.container.as_str()];
                cmd.extend(args.iter().map(String::as_str));
                docker(&cmd).map(|o| o.status.success()).unwrap_or(false)
            }
        };
        if ok {
            return Ok(started.elapsed().as_millis() as u64);
        }
        if started.elapsed() >= timeout {
            return Err(io::Error::other(format!(
                "コンテナ {} が {}ms 以内に準備できなかった: {}",
                running.spec.name,
                timeout.as_millis(),
                logs(&running.container)
            )));
        }
        thread::sleep(Duration::from_millis(200));
    }
}

/// 中継は常に接続を受け付けるため、対象へつなげないときは直後に閉じられる。直後に閉じられなければ到達できたとみなす。
fn target_holds_connection(port: u16) -> bool {
    let Ok(mut stream) = TcpStream::connect((Ipv4Addr::LOCALHOST, port)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(400)));
    let mut byte = [0u8; 1];
    match stream.read(&mut byte) {
        Ok(0) => false,
        Ok(_) => true,
        Err(e) => matches!(
            e.kind(),
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
        ),
    }
}

fn inspect_running(container: &str) -> io::Result<bool> {
    match docker_text(&["inspect", "--format", "{{.State.Running}}", container]) {
        Ok(value) => Ok(value == "true"),
        Err(_) => Ok(false),
    }
}

fn inspect_network(container: &str, network: &str) -> io::Result<bool> {
    let value = docker_text(&[
        "inspect",
        "--format",
        "{{json .NetworkSettings.Networks}}",
        container,
    ])
    .unwrap_or_default();
    Ok(value.contains(network))
}

fn logs(container: &str) -> String {
    docker_text(&["logs", "--tail", "80", container])
        .unwrap_or_default()
        .chars()
        .take(MAX_LOG)
        .collect()
}

fn labeled(trial: &str) -> Vec<String> {
    let filter = format!("label=eel.trial={trial}");
    let mut found = vec![];
    for (kind, args) in [
        ("container", &["ps", "-aq", "--filter", &filter][..]),
        ("network", &["network", "ls", "-q", "--filter", &filter][..]),
        ("volume", &["volume", "ls", "-q", "--filter", &filter][..]),
    ] {
        if let Ok(text) = docker_text(args) {
            for id in text.split_whitespace() {
                found.push(format!("{kind}:{id}"));
            }
        }
    }
    found
}

fn http_request(
    port: u16,
    tls: bool,
    method: &str,
    path: &str,
    content_type: Option<&str>,
    body: Option<&[u8]>,
    timeout_ms: u64,
) -> io::Result<HttpResult> {
    let url = format!(
        "{}://127.0.0.1:{port}{path}",
        if tls { "https" } else { "http" }
    );
    if tls {
        return curl_request(&url, method, content_type, body, timeout_ms);
    }
    let mut stream = TcpStream::connect_timeout(
        &std::net::SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
        Duration::from_millis(timeout_ms.min(5000)),
    )?;
    stream.set_read_timeout(Some(Duration::from_millis(timeout_ms.min(15000))))?;
    stream.set_write_timeout(Some(Duration::from_millis(5000)))?;
    let mut request =
        format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n");
    if let Some(kind) = content_type {
        request.push_str(&format!("Content-Type: {kind}\r\n"));
    }
    let payload = body.unwrap_or(b"");
    request.push_str(&format!("Content-Length: {}\r\n\r\n", payload.len()));
    stream.write_all(request.as_bytes())?;
    if !payload.is_empty() {
        stream.write_all(payload)?;
    }
    let _ = stream.shutdown(Shutdown::Write);
    let mut buf = Vec::new();
    stream.take(MAX_BODY as u64 + 8192).read_to_end(&mut buf)?;
    parse_http(&url, method, false, &buf)
}

fn curl_request(
    url: &str,
    method: &str,
    content_type: Option<&str>,
    body: Option<&[u8]>,
    timeout_ms: u64,
) -> io::Result<HttpResult> {
    let dir = tempfile()?;
    let header_path = dir.join("headers");
    let body_path = dir.join("body");
    let payload = dir.join("payload");
    if let Some(bytes) = body {
        fs::write(&payload, bytes)?;
    }
    let seconds = ((timeout_ms / 1000).max(1)).to_string();
    let mut args = vec![
        "-sS".into(),
        "-D".into(),
        header_path.to_string_lossy().into_owned(),
        "-o".into(),
        body_path.to_string_lossy().into_owned(),
        "--max-time".into(),
        seconds,
        "--connect-timeout".into(),
        "5".into(),
        "-k".into(),
        "-X".into(),
        method.to_owned(),
    ];
    if let Some(kind) = content_type {
        args.push("-H".into());
        args.push(format!("Content-Type: {kind}"));
    }
    if body.is_some() {
        args.push("--data-binary".into());
        args.push(format!("@{}", payload.display()));
    }
    args.push(url.to_owned());
    let mut command = Command::new("curl");
    command
        .args(&args)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .stdin(Stdio::null());
    let output = command.output()?;
    let headers = fs::read(&header_path).unwrap_or_default();
    let body = fs::read(&body_path).unwrap_or_default();
    let mut combined = headers;
    combined.extend_from_slice(&body);
    let mut result = parse_http(url, method, true, &combined)?;
    if !output.status.success() && result.status.is_none() {
        result.detail = format!(
            "curlが失敗した: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let _ = fs::remove_dir_all(dir);
    Ok(result)
}

fn tempfile() -> io::Result<PathBuf> {
    let path = std::env::temp_dir().join(format!(
        "eel-http-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    fs::create_dir(&path)?;
    Ok(path)
}

fn parse_http(url: &str, method: &str, tls_insecure: bool, raw: &[u8]) -> io::Result<HttpResult> {
    let text = String::from_utf8_lossy(raw);
    let status = text
        .lines()
        .find(|l| l.starts_with("HTTP/"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok());
    let body = text
        .split("\r\n\r\n")
        .nth(1)
        .unwrap_or(text.split("\n\n").nth(1).unwrap_or(""));
    let excerpt: String = body.chars().take(2000).collect();
    Ok(HttpResult {
        method: method.into(),
        url: url.into(),
        status,
        tls_insecure,
        body_excerpt: excerpt.clone(),
        detail: match status {
            Some(code) => format!(
                "基盤が {} {} へ要求し、HTTP {code} を受け取った{}",
                method,
                url,
                if tls_insecure {
                    "（試験用の自己署名証明書のため、証明書の検証はしていない）"
                } else {
                    ""
                }
            ),
            None => "HTTPの状態コードを読み取れなかった".into(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn egress_defaults_to_blocked_and_is_omitted_from_old_settings() {
        let spec: ContainerSpec = serde_json::from_str(r#"{"services":[]}"#).unwrap();
        assert_eq!(spec.egress, Egress::Blocked);
        assert!(!serde_json::to_string(&spec).unwrap().contains("egress"));
        let allowed: ContainerSpec =
            serde_json::from_str(r#"{"services":[],"egress":"allowed"}"#).unwrap();
        assert!(serde_json::to_string(&allowed).unwrap().contains("allowed"));
    }

    #[test]
    fn connections_are_summarized_by_client_and_target() {
        let line = |t: f64, status: &str, bytes_in: u64, out: u64| {
            serde_json::from_str::<TapLine>(&format!(
                r#"{{"id":1,"t":{t},"client":"10.0.0.2","local":"10.0.0.9","status":"{status}","in":{bytes_in},"out":{out}}}"#
            ))
            .unwrap()
        };
        let mut rows = BTreeMap::new();
        record_connection(&mut rows, "app", "db", &line(1000.5, "200", 10, 100));
        record_connection(&mut rows, "app", "db", &line(1002.0, "500", 0, 0));
        record_connection(&mut rows, HOST_CLIENT, "app", &line(999.0, "200", 5, 50));
        let app_to_db = &rows[&("app".to_owned(), "db".to_owned())];
        assert_eq!(app_to_db.connections, 2);
        assert_eq!(app_to_db.failed, 1);
        assert_eq!(app_to_db.bytes_from_client, 10);
        assert_eq!(app_to_db.bytes_to_client, 100);
        assert_eq!(app_to_db.first_at_ms, 1_000_500);
        assert_eq!(app_to_db.last_at_ms, 1_002_000);
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn unfinished_connections_are_read_from_the_error_log() {
        let log = "2026/10/25 15:26:28 [info] 7#7: *4 client 172.18.0.4:58172 connected to 0.0.0.0:80\n\
                   2026/10/25 15:26:28 [info] 7#7: *4 proxy 172.18.0.3:1 connected to 172.18.0.2:80\n\
                   2026/10/25 15:26:29 [notice] 1#1: start worker process 7\n";
        assert_eq!(started_connections(log), vec![(4, "172.18.0.4".to_owned())]);
        let mut rows = BTreeMap::new();
        record_unfinished(&mut rows, "web", "back");
        let row = &rows[&("web".to_owned(), "back".to_owned())];
        assert_eq!((row.connections, row.unfinished, row.failed), (1, 1, 0));
    }

    #[test]
    fn hardening_drops_all_capabilities_then_adds_the_kept_ones() {
        let mut args = vec![];
        harden(&mut args);
        let text = args.join(" ");
        assert!(text.contains("--cap-drop ALL"));
        assert!(text.contains("no-new-privileges"));
        assert!(!text.contains("--cap-add SYS_ADMIN"));
        assert_eq!(text.matches("--cap-add").count(), KEPT_CAPABILITIES.len());
    }

    #[test]
    fn digest_is_required() {
        assert!(image_digest("nginx:1.27.4-alpine").is_none());
        assert!(image_digest(
            "nginx:1.27.4-alpine@sha256:4ff102c5d78d254a6f0da062b3cf39eaf07f01eec0927fd21e219d0af8bc0591"
        )
        .is_some());
        let spec = ContainerSpec {
            services: vec![Service {
                name: "web".into(),
                image: "nginx:1.27.4-alpine".into(),
                role: ServiceRole::App,
                expose: 80,
                env: BTreeMap::new(),
                volumes: vec![],
                web_build: None,
                command: vec![],
                ready: ReadyCheck::Tcp,
                ready_timeout_ms: 5000,
            }],
            egress: Egress::default(),
        };
        assert!(validate_spec(&spec).is_err());
    }

    #[test]
    fn service_names_cannot_shadow_another_services_real_alias() {
        let mut spec: ContainerSpec = serde_json::from_str(
            r#"{"services":[{"name":"app","image":"nginx:1.27.4-alpine@sha256:4ff102c5d78d254a6f0da062b3cf39eaf07f01eec0927fd21e219d0af8bc0591","role":"app","expose":80,"ready":"tcp","ready_timeout_ms":5000}]}"#,
        ).unwrap();
        let mut other = spec.services[0].clone();
        other.name = "app-real".into();
        spec.services.push(other);
        assert!(
            validate_spec(&spec)
                .unwrap_err()
                .to_string()
                .contains("中継用")
        );
        spec.egress = Egress::Allowed;
        assert!(validate_spec(&spec).is_ok());
    }

    #[test]
    fn http_target_must_exist() {
        let spec = ContainerSpec {
            services: vec![Service {
                name: "web".into(),
                image: "nginx:1.27.4-alpine@sha256:4ff102c5d78d254a6f0da062b3cf39eaf07f01eec0927fd21e219d0af8bc0591".into(),
                role: ServiceRole::App,
                expose: 80,
                env: BTreeMap::new(),
                volumes: vec![],
            web_build: None,
                command: vec![],
                ready: ReadyCheck::Http { path: "/".into(), tls: false },
                ready_timeout_ms: 5000,
            }],
            egress: Egress::default(),
        };
        assert!(validate_spec(&spec).is_ok());
        assert!(
            validate_http(
                &HttpOp {
                    service: "missing".into(),
                    method: "GET".into(),
                    path: "/".into(),
                    body_file: None,
                    content_type: None,
                },
                &spec
            )
            .is_err()
        );
    }
}
