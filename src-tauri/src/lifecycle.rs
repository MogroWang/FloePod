//! Owned, serial background work. Stopping wakes the timer and joins in-flight work.
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[derive(Default)]
struct ActiveTasks {
    closed: bool,
    count: usize,
}

/// Reject new work at shutdown and wait for effects already accepted, even if their IPC future is dropped.
#[derive(Clone, Default)]
pub struct TaskGate(Arc<(Mutex<ActiveTasks>, Condvar)>);
pub struct TaskPermit(TaskGate);

impl TaskGate {
    pub fn enter(&self) -> Result<TaskPermit, String> {
        let mut tasks = self.0 .0.lock().unwrap();
        if tasks.closed {
            return Err("应用正在退出，请等待当前操作完成".into());
        }
        tasks.count += 1;
        Ok(TaskPermit(self.clone()))
    }
    pub fn close(&self) {
        self.0 .0.lock().unwrap().closed = true;
    }
    pub fn wait(&self) {
        let tasks = self.0 .0.lock().unwrap();
        drop(
            self.0
                 .1
                .wait_while(tasks, |tasks| tasks.count != 0)
                .unwrap(),
        );
    }
}

impl Drop for TaskPermit {
    fn drop(&mut self) {
        let mut tasks = self.0 .0 .0.lock().unwrap();
        tasks.count -= 1;
        if tasks.count == 0 {
            self.0 .0 .1.notify_all();
        }
    }
}

#[derive(Clone, Default)]
pub struct StopToken(Arc<(Mutex<bool>, Condvar)>);

impl StopToken {
    pub fn is_stopped(&self) -> bool {
        *self.0 .0.lock().unwrap()
    }

    fn stop(&self) {
        *self.0 .0.lock().unwrap() = true;
        self.0 .1.notify_all();
    }

    fn wait(&self, duration: Duration) -> bool {
        let stopped = self.0 .0.lock().unwrap();
        *self
            .0
             .1
            .wait_timeout_while(stopped, duration, |stopped| !*stopped)
            .unwrap()
            .0
    }
}

struct Worker {
    stop: StopToken,
    thread: JoinHandle<()>,
}

#[derive(Default)]
pub struct PeriodicTask(Mutex<Option<Worker>>);

/// Time is passed in explicitly so long-running and delayed cycles need no sleeps in tests.
struct Cadence {
    period: Duration,
    next: Duration,
}

impl Cadence {
    fn new(period: Duration) -> Self {
        Self {
            period,
            next: period,
        }
    }

    fn remaining(&self, now: Duration) -> Duration {
        self.next.saturating_sub(now)
    }

    fn completed(&mut self, now: Duration) {
        // No catch-up burst after suspend or a slow operation; failures retry next period.
        self.next = now.saturating_add(self.period);
    }
}

impl PeriodicTask {
    /// Starts at most one worker. The first periodic run is after `period`;
    /// application startup work can finish before watchers begin observing files.
    pub fn start(
        &self,
        name: &str,
        period: Duration,
        action: impl FnMut(&StopToken) -> Result<(), String> + Send + 'static,
    ) -> Result<bool, String> {
        self.start_initialized(name, period, |_| Ok(()), action)
    }

    /// Initialization is serialized with start/stop, so a duplicate start cannot repeat startup I/O.
    pub fn start_initialized(
        &self,
        name: &str,
        period: Duration,
        initialize: impl FnOnce(&StopToken) -> Result<(), String>,
        mut action: impl FnMut(&StopToken) -> Result<(), String> + Send + 'static,
    ) -> Result<bool, String> {
        if period.is_zero() {
            return Err("后台任务周期必须大于零".into());
        }
        let mut worker = self.0.lock().unwrap();
        if worker.is_some() {
            return Ok(false);
        }
        let stop = StopToken::default();
        if let Err(error) = initialize(&stop) {
            crate::logging::write(&format!("[{name}] 启动任务失败: {error}；将按周期重试"));
        }
        let signal = stop.clone();
        let task_name = name.to_string();
        let thread = thread::Builder::new()
            .name(task_name.clone())
            .spawn(move || {
                let clock = Instant::now();
                let mut cadence = Cadence::new(period);
                while !signal.wait(cadence.remaining(clock.elapsed())) {
                    if let Err(error) = action(&signal) {
                        crate::logging::write(&format!("[{task_name}] {error}；下一周期重试"));
                    }
                    cadence.completed(clock.elapsed());
                }
            })
            .map_err(|error| format!("无法启动后台任务 {name}: {error}"))?;
        *worker = Some(Worker { stop, thread });
        Ok(true)
    }

    /// Nonblocking cancellation is safe on the UI thread; join while the event loop is still alive.
    pub fn request_stop(&self) {
        if let Some(worker) = self.0.lock().unwrap().as_ref() {
            worker.stop.stop();
        }
    }

    pub fn stop(&self) {
        // Keep the lifecycle lock while joining so restart cannot overlap the old worker.
        let mut worker = self.0.lock().unwrap();
        if let Some(worker) = worker.take() {
            worker.stop.stop();
            if worker.thread.join().is_err() {
                crate::logging::write("[lifecycle] 后台任务异常退出");
            }
        }
    }
}

impl Drop for PeriodicTask {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn duplicate_start_does_not_repeat_startup_cleanup() {
        let task = PeriodicTask::default();
        let count = std::cell::Cell::new(0);
        for _ in 0..3 {
            task.start_initialized(
                "startup",
                Duration::from_secs(86_400),
                |_| {
                    count.set(count.get() + 1);
                    Ok(())
                },
                |_| Ok(()),
            )
            .unwrap();
        }
        assert_eq!(count.get(), 1);
        task.request_stop();
        task.stop();
    }

    #[test]
    fn elapsed_days_do_not_disable_periodic_work_or_cause_catch_up_bursts() {
        let mut cadence = Cadence::new(Duration::from_secs(300));
        for now in [300, 600, 86400, 365 * 86400] {
            assert!(cadence.remaining(Duration::from_secs(now)).is_zero());
            cadence.completed(Duration::from_secs(now));
            assert_eq!(
                cadence.remaining(Duration::from_secs(now)),
                Duration::from_secs(300)
            );
        }
    }

    #[test]
    fn failures_retry_and_duplicate_start_does_not_create_another_worker() {
        let task = PeriodicTask::default();
        let (tx, rx) = mpsc::channel();
        assert!(task
            .start("test-retry", Duration::from_millis(1), move |_| {
                tx.send(()).unwrap();
                Err("模拟临时清理失败".into())
            })
            .unwrap());
        assert!(!task
            .start("duplicate", Duration::from_millis(1), |_| panic!(
                "duplicate worker"
            ))
            .unwrap());
        for _ in 0..3 {
            rx.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        task.stop();
        while rx.try_recv().is_ok() {}
        assert!(rx.recv_timeout(Duration::from_millis(20)).is_err());
    }

    #[test]
    fn stop_wakes_a_long_timer_and_restart_is_safe() {
        let task = PeriodicTask::default();
        task.start("long-timer", Duration::from_secs(86400), |_| {
            panic!("timer should stop")
        })
        .unwrap();
        task.stop();
        task.stop();
        let (tx, rx) = mpsc::channel();
        task.start("restart", Duration::from_millis(1), move |_| {
            tx.send(()).unwrap();
            Ok(())
        })
        .unwrap();
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        task.stop();
    }

    #[test]
    fn stop_joins_in_flight_work_after_cooperative_cancellation() {
        let task = PeriodicTask::default();
        let (tx, rx) = mpsc::channel();
        task.start("in-flight", Duration::from_millis(1), move |stop| {
            tx.send("started").unwrap();
            stop.wait(Duration::from_secs(86400));
            tx.send("finished").unwrap();
            Ok(())
        })
        .unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), "started");
        task.stop();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), "finished");
    }
}

#[cfg(test)]
mod task_gate_tests {
    use super::*;
    #[test]
    fn shutdown_rejects_new_work_and_waits_for_accepted_effects() {
        let gate = TaskGate::default();
        let first = gate.enter().unwrap();
        let second = gate.enter().unwrap();
        gate.close();
        assert!(gate.enter().is_err());
        let (done, result) = std::sync::mpsc::channel();
        let waiter = std::thread::spawn(move || {
            gate.wait();
            done.send(()).unwrap();
        });
        drop(first);
        assert!(result.try_recv().is_err());
        drop(second);
        result.recv_timeout(Duration::from_secs(2)).unwrap();
        waiter.join().unwrap();
    }
}
