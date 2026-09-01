//! One running tool call, and everything needed to end it.
//!
//! Every tool call the agent makes is a job: it is registered when it starts,
//! it can be cancelled from the UI while it runs, and it is removed however it
//! ends — success, failure, timeout or cancellation. The registry is the only
//! thing that knows a call is still running, so Stop has exactly one place to
//! look and nothing can be left behind untracked.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

struct Inner {
    cancel: watch::Sender<bool>,
}

/// A handle to one running tool call. Cheap to clone; every clone refers to the
/// same job.
#[derive(Clone)]
pub struct Job(Arc<Inner>);

impl Job {
    fn new() -> Self {
        let (cancel, _) = watch::channel(false);
        Job(Arc::new(Inner { cancel }))
    }

    /// Ask this job to stop. Idempotent: pressing Stop twice is not an error.
    ///
    /// `send_replace` rather than `send`, deliberately: `send` fails and leaves
    /// the value untouched when nothing is subscribed yet, which would quietly
    /// lose a Stop pressed in the moment between a call starting and the runner
    /// beginning to watch it.
    pub fn cancel(&self) {
        self.0.cancel.send_replace(true);
    }

    pub fn is_cancelled(&self) -> bool {
        *self.0.cancel.borrow()
    }

    /// Resolves when Stop is pressed, and never otherwise. Safe to select on:
    /// a job that is never cancelled simply leaves this branch pending.
    pub async fn cancelled(&self) {
        let mut rx = self.0.cancel.subscribe();
        loop {
            if *rx.borrow_and_update() {
                return;
            }
            if rx.changed().await.is_err() {
                // The job outlived its registry entry, which means nothing can
                // cancel it any more.
                std::future::pending::<()>().await;
            }
        }
    }
}

/// Every tool call currently running, by the id the UI knows it as.
#[derive(Default)]
pub struct Jobs {
    running: Mutex<HashMap<String, Job>>,
}

impl Jobs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a call as running. The guard removes it however the call ends,
    /// including on an early return or a panic, so the registry can never
    /// report work that has already finished.
    pub fn start(&self, call_id: &str) -> (Job, Guard<'_>) {
        let job = Job::new();
        if let Ok(mut running) = self.running.lock() {
            running.insert(call_id.to_string(), job.clone());
        }
        (
            job,
            Guard {
                jobs: self,
                call_id: call_id.to_string(),
            },
        )
    }

    /// True when there was something to stop.
    pub fn cancel(&self, call_id: &str) -> bool {
        let job = self
            .running
            .lock()
            .ok()
            .and_then(|running| running.get(call_id).cloned());
        match job {
            Some(job) => {
                job.cancel();
                true
            }
            None => false,
        }
    }

    /// The read side of the registry. Tests use it to wait for a command to
    /// register before stopping it; nothing in the app needs to ask.
    #[cfg(test)]
    pub fn is_running(&self, call_id: &str) -> bool {
        self.running
            .lock()
            .map(|running| running.contains_key(call_id))
            .unwrap_or(false)
    }

    fn finish(&self, call_id: &str) {
        if let Ok(mut running) = self.running.lock() {
            running.remove(call_id);
        }
    }
}

pub struct Guard<'a> {
    jobs: &'a Jobs,
    call_id: String,
}

impl Drop for Guard<'_> {
    fn drop(&mut self) {
        self.jobs.finish(&self.call_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_job_is_registered_while_it_runs_and_gone_after() {
        let jobs = Jobs::new();
        {
            let (_job, _guard) = jobs.start("call-1");
            assert!(jobs.is_running("call-1"));
        }
        assert!(!jobs.is_running("call-1"), "the guard must clear the entry");
        assert!(!jobs.cancel("call-1"), "nothing left to cancel");
    }

    #[tokio::test]
    async fn stop_is_not_lost_when_it_arrives_before_anyone_is_watching() {
        let jobs = Jobs::new();
        let (job, _guard) = jobs.start("call-1");
        assert!(jobs.cancel("call-1"));
        assert!(job.is_cancelled());
        // Already cancelled before anyone waited: still resolves immediately.
        tokio::time::timeout(std::time::Duration::from_secs(1), job.cancelled())
            .await
            .expect("a cancelled job must not keep anyone waiting");
    }

    #[tokio::test]
    async fn a_wait_that_is_already_running_wakes_on_cancel() {
        let jobs = Jobs::new();
        let (job, _guard) = jobs.start("call-1");
        let waiting = job.clone();
        let task = tokio::spawn(async move { waiting.cancelled().await });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(jobs.cancel("call-1"));
        tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .expect("Stop must reach a waiter")
            .unwrap();
    }

    #[tokio::test]
    async fn a_job_nobody_cancels_never_resolves() {
        let jobs = Jobs::new();
        let (job, _guard) = jobs.start("call-1");
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), job.cancelled())
                .await
                .is_err(),
            "an uncancelled job must leave the select branch pending"
        );
    }

}
