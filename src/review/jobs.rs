//! 長い読取・AI処理の受付、状態、結果の保持。送信済みの外部処理は強制終了できない。
use axum::{body::to_bytes, response::Response};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Status {
    Queued,
    Running,
    Completed,
    Cancelled,
}
#[derive(Clone, Serialize)]
pub(super) struct Job {
    pub id: String,
    pub kind: &'static str,
    pub status: Status,
    pub response_status: Option<u16>,
    pub result: Option<Value>,
}
#[derive(Clone)]
pub(super) struct Queue {
    jobs: Arc<Mutex<BTreeMap<u64, Job>>>,
    sequence: Arc<AtomicU64>,
    workers: Arc<tokio::sync::Semaphore>,
    capacity: Arc<tokio::sync::Semaphore>,
}
impl Queue {
    pub fn new(workers: Arc<tokio::sync::Semaphore>) -> Self {
        Self {
            jobs: Default::default(),
            sequence: Default::default(),
            workers,
            capacity: Arc::new(tokio::sync::Semaphore::new(12)),
        }
    }
    pub fn get(&self, id: u64) -> io::Result<Job> {
        self.jobs
            .lock()
            .map_err(|_| io::Error::other("処理状態を読めません"))?
            .get(&id)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "処理の記録がありません"))
    }
    pub fn cancel(&self, id: u64) -> io::Result<Job> {
        let mut jobs = self
            .jobs
            .lock()
            .map_err(|_| io::Error::other("処理状態を読めません"))?;
        let job = jobs
            .get_mut(&id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "処理の記録がありません"))?;
        if job.status == Status::Completed {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "処理は完了しています",
            ));
        }
        job.status = Status::Cancelled;
        job.result = None;
        Ok(job.clone())
    }
    pub fn start(&self, task: impl FnOnce() -> Response + Send + 'static) -> io::Result<Job> {
        let capacity =
            self.capacity.clone().try_acquire_owned().map_err(|_| {
                io::Error::new(io::ErrorKind::WouldBlock, "読取・AI処理の受付上限です")
            })?;
        let id = self.sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let job = Job {
            id: format!("{id}"),
            kind: "request",
            status: Status::Queued,
            response_status: None,
            result: None,
        };
        {
            let mut jobs = self
                .jobs
                .lock()
                .map_err(|_| io::Error::other("処理状態を読めません"))?;
            while jobs.len() >= 16 {
                let finished = jobs
                    .iter()
                    .find(|(_, j)| matches!(j.status, Status::Completed | Status::Cancelled))
                    .map(|(id, _)| *id);
                if let Some(id) = finished {
                    jobs.remove(&id);
                } else {
                    break;
                }
            }
            jobs.insert(id, job.clone());
        }
        let queue = self.clone();
        tokio::spawn(async move {
            let _capacity = capacity;
            let Ok(_worker) = queue.workers.clone().acquire_owned().await else {
                return;
            };
            {
                let Ok(mut jobs) = queue.jobs.lock() else {
                    return;
                };
                let Some(job) = jobs.get_mut(&id) else { return };
                if job.status == Status::Cancelled {
                    return;
                }
                job.status = Status::Running;
            }
            let reply = match tokio::task::spawn_blocking(task).await {
                Ok(response) => {
                    let status = response.status().as_u16();
                    match to_bytes(response.into_body(), 8 * 1024 * 1024).await {
                        Ok(bytes) => serde_json::from_slice(&bytes)
                            .map(|value| (status, value))
                            .unwrap_or((
                                500,
                                serde_json::json!({"error":"処理結果の形式が不正です"}),
                            )),
                        Err(_) => (
                            500,
                            serde_json::json!({"error":"処理結果が上限を超えています"}),
                        ),
                    }
                }
                Err(_) => (
                    500,
                    serde_json::json!({"error":"処理を完了できませんでした"}),
                ),
            };
            if let Ok(mut jobs) = queue.jobs.lock()
                && let Some(job) = jobs.get_mut(&id)
                && job.status != Status::Cancelled
            {
                job.status = Status::Completed;
                job.response_status = Some(reply.0);
                job.result = Some(reply.1);
            }
        });
        Ok(job)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_cancelled_queued_job_never_runs_and_completed_results_keep_http_status() {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let queue = Queue::new(Arc::new(tokio::sync::Semaphore::new(1)));
                let (tx, rx) = std::sync::mpsc::channel();
                let first = queue
                    .start(move || {
                        rx.recv().unwrap();
                        super::super::transport::failure(400, "invalid")
                    })
                    .unwrap();
                while queue.get(first.id.parse().unwrap()).unwrap().status == Status::Queued {
                    tokio::task::yield_now().await;
                }
                let second = queue.start(|| panic!("cancelled job executed")).unwrap();
                queue.cancel(second.id.parse().unwrap()).unwrap();
                tx.send(()).unwrap();
                let result = tokio::time::timeout(std::time::Duration::from_secs(3), async {
                    loop {
                        let job = queue.get(first.id.parse().unwrap()).unwrap();
                        if job.status == Status::Completed {
                            break job;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                    }
                })
                .await
                .unwrap();
                assert_eq!(result.response_status, Some(400));
                assert_eq!(
                    queue.get(second.id.parse().unwrap()).unwrap().status,
                    Status::Cancelled
                );
            });
    }
}
