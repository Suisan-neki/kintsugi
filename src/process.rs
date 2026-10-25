//! ホスト上の試行プロセスを専用のグループで起動し、終了後に同じグループを止める。
//! 対象が自分でセッションを分ける場合まで封じるOSの隔離ではない。
use std::{
    io,
    ops::{Deref, DerefMut},
    os::unix::process::CommandExt,
    process::{Child, Command},
};

pub(crate) struct ChildGuard(pub(crate) Child);

impl ChildGuard {
    pub(crate) fn spawn(command: &mut Command) -> io::Result<Self> {
        command.process_group(0).spawn().map(Self)
    }
}

impl Deref for ChildGuard {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.0
    }
}
impl DerefMut for ChildGuard {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.0
    }
}

pub(crate) fn kill_group(child: &mut Child) -> io::Result<()> {
    let pid = i32::try_from(child.id()).map_err(io::Error::other)?;
    // SAFETY: spawnで専用グループを作った子の正のPIDだけを使う。
    // 負のPIDは、その子と同じプロセスグループを指定する。
    if unsafe { libc::kill(-pid, libc::SIGKILL) } == -1 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            return Err(io::Error::new(
                error.kind(),
                format!("試行のプロセスグループ{pid}を停止できません: {error}"),
            ));
        }
    }
    Ok(())
}

/// Playwright等の管理プロセスに、分離した子の終了処理を行わせる。
pub(crate) fn terminate(child: &mut Child) -> io::Result<()> {
    if child.try_wait()?.is_some() {
        return Ok(());
    }
    let pid = i32::try_from(child.id()).map_err(io::Error::other)?;
    // SAFETY: まだ回収していない所有する子の正のPIDだけを指定する。
    if unsafe { libc::kill(pid, libc::SIGTERM) } == -1 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            return Err(error);
        }
    }
    Ok(())
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        // 親が先に正常終了していても、同じグループの子孫を止める。
        let _ = kill_group(&mut self.0);
        let _ = self.0.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, thread, time::Duration};

    #[test]
    fn descendants_are_stopped_after_normal_exit_and_early_drop() {
        let temp = std::env::temp_dir().join(format!("mugiurushi-process-{}", std::process::id()));
        fs::create_dir_all(&temp).unwrap();
        for normal_exit in [true, false] {
            let marker = temp.join(if normal_exit { "normal" } else { "timeout" });
            let mut command = Command::new("sh");
            command
                .arg("-c")
                .arg(if normal_exit {
                    "(sleep 0.3; printf leaked > \"$1\") & exit 0"
                } else {
                    "(sleep 0.3; printf leaked > \"$1\") & wait"
                })
                .arg("trial")
                .arg(&marker);
            let mut child = ChildGuard::spawn(&mut command).unwrap();
            if normal_exit {
                assert!(child.wait().unwrap().success());
            }
            drop(child);
            thread::sleep(Duration::from_millis(450));
            assert!(!marker.exists(), "a descendant survived group cleanup");
        }
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn cleanup_keeps_other_process_groups_alive() {
        let mut unrelated = Command::new("sh").args(["-c", "sleep 2"]).spawn().unwrap();
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 2"]);
        drop(ChildGuard::spawn(&mut command).unwrap());
        assert!(unrelated.try_wait().unwrap().is_none());
        unrelated.kill().unwrap();
        unrelated.wait().unwrap();
    }
}
