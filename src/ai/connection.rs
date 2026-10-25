//! 運営者の設定を起動時に読み、秘密をレスポンスや通信エラーへ含めない。
use super::*;
use crate::error::invalid;
use std::time::Duration;
use std::{fs, io::Write, path::Path};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectionSettings {
    base_url: String,
    model: String,
    api_key: String,
}

impl Config {
    pub(super) fn validate_connection(&self) -> io::Result<()> {
        let url = url::Url::parse(&self.base_url)
            .map_err(|_| invalid("AI接続先のURLを確認してください"))?;
        if url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || !(url.scheme() == "https"
                || url.scheme() == "http"
                    && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")))
            || self.base_url.len() > 2048
            || self.model.trim().is_empty()
            || self.model.len() > 160
            || self.model.chars().any(char::is_control)
        {
            return Err(invalid(
                "AI接続先にはHTTPS、ローカル接続にはlocalhostのHTTPを指定し、モデル名を入力してください",
            ));
        }
        Ok(())
    }
    pub(super) fn key(&self) -> io::Result<String> {
        let key = match &self.inline_key {
            Some(key) => key.clone(),
            None => fs::read_to_string(&self.key_file)
                .map_err(|_| invalid("AIの認証情報を読めません"))?,
        };
        let key = key.trim();
        if key.is_empty() || key.len() > 4096 || !key.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(invalid("AIのAPIキーを確認してください"));
        }
        Ok(key.into())
    }
    pub fn load_connection(path: &Path) -> io::Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }
        if fs::metadata(path)?.len() > 16384 {
            return Err(invalid("AI接続設定のサイズを確認してください"));
        }
        let bytes = fs::read(path).map_err(|_| invalid("運営者のAI設定ファイルを読めません"))?;
        let settings: ConnectionSettings = serde_json::from_slice(&bytes)
            .map_err(|_| invalid("運営者のAI設定ファイルのJSON形式を確認してください"))?;
        let config = Self {
            base_url: settings.base_url.trim_end_matches('/').into(),
            model: settings.model.trim().into(),
            key_file: PathBuf::new(),
            inline_key: Some(settings.api_key),
        };
        config.validate_connection()?;
        config.key()?;
        Ok(Some(config))
    }
    pub fn test_connection(&self) -> io::Result<Value> {
        let response = self.complete_in(
            json!([
                {"role":"system", "content":"EEL_CONNECTION_TEST: Return only the JSON object {\"connected\":true}."},
                {"role":"user", "content":"接続確認です。指定されたJSONだけを返してください。"}
            ]),
            Duration::from_secs(30),
        )?;
        let content: Value = serde_json::from_str(response["content"].as_str().unwrap_or_default())
            .map_err(|_| invalid("接続先からJSON形式の応答を得られませんでした"))?;
        if content["connected"] != true {
            return Err(invalid("接続先から想定した形式の応答を得られませんでした"));
        }
        Ok(
            json!({"status":"ok", "model":response["model"], "message":"指定したモデルから応答を受け取りました。構成図を生成できます。"}),
        )
    }
    /// 生成ではNode.jsを必要としない。JSONの構造と参照の検査は呼び出し側が行う。
    pub(crate) fn complete(&self, mut messages: Value) -> io::Result<Value> {
        crate::writing::apply(&mut messages)?;
        self.complete_in(messages, Duration::from_secs(300))
    }
    fn complete_in(&self, messages: Value, timeout: Duration) -> io::Result<Value> {
        self.validate_connection()?;
        let key = self.key()?;
        let body = serde_json::to_vec(&json!({"model": self.model, "messages":messages, "response_format":{"type":"json_object"}})).map_err(io::Error::other)?;
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .tls_config(
                ureq::tls::TlsConfig::builder()
                    .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                    .build(),
            )
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_global(Some(timeout))
            .build()
            .into();
        let mut response = agent
            .post(format!("{}/chat/completions", self.base_url))
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {key}"))
            .send(body.as_slice())
            .map_err(map_connect_error)?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response
                .body_mut()
                .with_config()
                .limit(2048)
                .read_to_string()
                .unwrap_or_default();
            return Err(public_http_error(status, &body, &key));
        }
        let text = response.body_mut().with_config().limit(2 * 1024 * 1024).read_to_string().map_err(|_| invalid("AI応答を読み取れませんでした。待機時間またはサイズの上限に達した可能性があります"))?;
        let data: Value = serde_json::from_str(&text)
            .map_err(|_| invalid("AI接続先の応答をJSONとして読めませんでした"))?;
        let content = data["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| invalid("AIの応答に本文がありません"))?;
        if content.len() > 512 * 1024 {
            return Err(invalid("AIの生成結果が512KiBを超えています"));
        }
        Ok(json!({"content":content,"model":data["model"].as_str().unwrap_or(&self.model)}))
    }
}

fn map_connect_error(error: ureq::Error) -> io::Error {
    let (kind, message) = match error {
        ureq::Error::Timeout(_) => (
            io::ErrorKind::TimedOut,
            "構成図の生成が待機上限に達しました。時間をおいて再度試してください",
        ),
        ureq::Error::HostNotFound => (
            io::ErrorKind::InvalidInput,
            "AI接続先のホスト名を解決できませんでした。接続設定とネットワークを確認してください",
        ),
        ureq::Error::Tls(_) | ureq::Error::TlsRequired => (
            io::ErrorKind::InvalidInput,
            "AI接続先の暗号化通信を確立できませんでした。接続設定を確認してください",
        ),
        _ => (
            io::ErrorKind::InvalidInput,
            "AI接続先へ接続できませんでした。接続設定とネットワークを確認してください",
        ),
    };
    io::Error::new(kind, message)
}

fn public_http_error(status: u16, body: &str, secret: &str) -> io::Error {
    let detail = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_owned))
        .filter(|message| {
            !message.is_empty()
                && message.len() <= 300
                && message.bytes().all(|b| b >= 32)
                && !message.contains(secret)
        });
    invalid(&match detail {
        Some(message) => format!("AI接続先がHTTP {status}を返しました。{message}"),
        None => {
            format!("AI接続先がHTTP {status}を返しました。認証・モデル名・利用枠を確認してください")
        }
    })
}

/// 開発者が編集するひな形を作る。既存のキーは上書きしない。
pub fn initialize_connection(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(include_bytes!("../../ai.example.json"))?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::Duration;

    fn serve(status: &str, body: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let header = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0; 4096];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(header.as_bytes());
        });
        format!("http://127.0.0.1:{}", address.port())
    }

    fn config(url: &str) -> Config {
        Config {
            base_url: url.trim_end_matches('/').into(),
            model: "synthetic".into(),
            key_file: PathBuf::new(),
            inline_key: Some("synthetic-private-key".into()),
        }
    }

    #[test]
    fn http_error_shows_provider_message_without_secrets() {
        let error = public_http_error(
            400,
            r#"{"error":{"message":"context length exceeded","type":"invalid_request_error"}}"#,
            "synthetic-private-key",
        );
        assert!(error.to_string().contains("context length exceeded"));
        assert!(!error.to_string().contains("synthetic-private-key"));
        let hidden = public_http_error(
            400,
            r#"{"error":{"message":"key synthetic-private-key rejected"}}"#,
            "synthetic-private-key",
        );
        assert!(!hidden.to_string().contains("synthetic-private-key"));
        assert!(hidden.to_string().contains("認証・モデル名・利用枠"));
    }

    #[test]
    fn rejected_completion_includes_status_and_drops_secrets() {
        let url = serve(
            "400 Bad Request",
            r#"{"error":{"message":"model does not exist"}}"#,
        );
        let error = config(&url)
            .complete_in(
                json!([{"role":"user","content":"JSON"}]),
                Duration::from_secs(2),
            )
            .unwrap_err();
        assert!(error.to_string().contains("HTTP 400"));
        assert!(error.to_string().contains("model does not exist"));
        assert!(!error.to_string().contains("synthetic-private-key"));
    }

    #[test]
    fn stalled_completion_is_a_timeout_not_a_missing_key() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            let _stream = listener.accept().unwrap();
            thread::sleep(Duration::from_secs(3));
        });
        let error = config(&format!("http://127.0.0.1:{}", address.port()))
            .complete_in(
                json!([{"role":"user","content":"JSON"}]),
                Duration::from_millis(200),
            )
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(error.to_string().contains("待機上限"));
        assert!(!error.to_string().contains("synthetic-private-key"));
    }
}
