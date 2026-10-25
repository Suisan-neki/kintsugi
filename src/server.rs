//! 試行の中で先に起動して待つプロセス（例：画像の受信サーバー）。
//! 準備の完了は、基盤が割り当てた 127.0.0.1 のポートへ接続できるかで確かめる。
//! 操作するプロセスが終わった後に基盤が止め、止めたことを待つプロセスの失敗として扱わない。
use crate::error::invalid;
use crate::python_env::Launch;
use serde::{Deserialize, Serialize};
use std::{
    io,
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    process::{Child, ExitStatus},
    thread,
    time::{Duration, Instant},
};

pub const PORT: &str = "{{port}}";
pub const MAX_READY_MS: u64 = 60_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerSpec {
    #[serde(default, skip_serializing_if = "Launch::is_file")]
    pub launch: Launch,
    pub entrypoint: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub ready_timeout_ms: u64,
}

/// 待つプロセスの記録。対象の報告・基盤の読み出しとは別に残す。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerPhase {
    pub command: Vec<String>,
    pub port: u16,
    pub started: bool,
    pub ready: bool,
    pub ready_after_ms: Option<u64>,
    /// 基盤が止める前に、自分で終了していたか。
    pub exited_before_stop: bool,
    pub exit_code: Option<i32>,
    pub stopped_by_platform: bool,
    pub output_limit: bool,
    pub stdout_excerpt: String,
    pub stderr_excerpt: String,
    pub detail: String,
}

pub fn validate(spec: &ServerSpec) -> io::Result<()> {
    if !(1..=MAX_READY_MS).contains(&spec.ready_timeout_ms)
        || spec.args.len() > 64
        || spec
            .args
            .iter()
            .any(|a| a.len() > 16384 || a.contains('\0'))
    {
        return Err(invalid(
            "待つプロセスの引数と、準備の待機上限（1〜60000ミリ秒）を確認してください",
        ));
    }
    if !spec.args.iter().any(|a| a.contains(PORT)) {
        return Err(invalid(format!(
            "待つプロセスの引数に、基盤が割り当てるポート {PORT} を入れてください"
        )));
    }
    Ok(())
}

/// 試行の中だけで使うループバックのポートを選ぶ。選んでから起動までの間に他が使う可能性は残る。
pub fn free_port() -> io::Result<u16> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    Ok(listener.local_addr()?.port())
}

pub fn listening(port: u16) -> bool {
    TcpStream::connect_timeout(
        &SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
        Duration::from_millis(200),
    )
    .is_ok()
}

pub enum Ready {
    Ready(u64),
    Exited(ExitStatus),
    TimedOut,
}

/// 待つプロセスが接続を受け付けるまで待つ。先に終了した場合はその状態を返す。
pub fn wait_ready(child: &mut Child, port: u16, timeout_ms: u64) -> io::Result<Ready> {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Ready::Exited(status));
        }
        if listening(port) {
            return Ok(Ready::Ready(started.elapsed().as_millis() as u64));
        }
        if started.elapsed() >= Duration::from_millis(timeout_ms) {
            return Ok(Ready::TimedOut);
        }
        thread::sleep(Duration::from_millis(50));
    }
}

/// 操作の後に待つプロセスを止める。自分で終了していたかどうかと終了コードを返す。
pub fn stop(child: &mut Child) -> io::Result<(bool, Option<i32>)> {
    let before = child.try_wait()?;
    if let Some(status) = before {
        return Ok((true, status.code()));
    }
    crate::process::kill_group(child)?;
    // 公開APIには専用グループを作っていないChildも渡せる。
    child.kill()?;
    Ok((false, child.wait()?.code()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_waiting_process_needs_the_assigned_port() {
        let mut spec = ServerSpec {
            launch: Launch::Module,
            entrypoint: "app".into(),
            args: vec!["serve".into()],
            ready_timeout_ms: 1000,
        };
        assert!(validate(&spec).is_err());
        spec.args.push(PORT.into());
        assert!(validate(&spec).is_ok());
        spec.ready_timeout_ms = MAX_READY_MS + 1;
        assert!(validate(&spec).is_err());
    }

    #[test]
    fn an_assigned_port_is_not_listening_until_bound() {
        let port = free_port().expect("ポート");
        assert!(!listening(port));
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).expect("待ち受け");
        assert!(listening(port));
        drop(listener);
    }
}

#[cfg(test)]
mod stop_tests {
    #[test]
    fn stop_accepts_a_child_without_a_dedicated_group() {
        let child = std::process::Command::new("sh")
            .args(["-c", "exec sleep 30"])
            .spawn()
            .unwrap();
        let pid = child.id();
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            let mut child = child;
            tx.send(super::stop(&mut child)).unwrap();
        });
        let result = rx.recv_timeout(std::time::Duration::from_secs(2));
        if result.is_err() {
            // SAFETY: 待機中の所有する子を、失敗した試験でも残さない。
            unsafe {
                libc::kill(pid as i32, libc::SIGKILL);
            }
        }
        handle.join().unwrap();
        assert!(!result.unwrap().unwrap().0);
    }
}
