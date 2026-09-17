//! Prepares native graphics resources without blocking the event loop.

use std::sync::mpsc::{self, Receiver, TryRecvError};

/// A single preparation job, scheduled only after the window's first frame.
pub(crate) struct RendererStartup<T> {
    job: Option<Box<dyn FnOnce() -> T + Send>>,
    receiver: Option<Receiver<T>>,
    first_frame: bool,
}

impl<T: Send + 'static> RendererStartup<T> {
    pub(crate) fn new(job: impl FnOnce() -> T + Send + 'static) -> Self {
        Self {
            job: Some(Box::new(job)),
            receiver: None,
            first_frame: false,
        }
    }

    pub(crate) fn after_frame(&mut self) {
        self.first_frame = true;
    }

    /// Starts or polls preparation; never waits for the worker.
    pub(crate) fn tick(&mut self) -> Result<Option<T>, String> {
        if !self.first_frame {
            return Ok(None);
        }
        if let Some(job) = self.job.take() {
            let (sender, receiver) = mpsc::sync_channel(1);
            std::thread::Builder::new()
                .name("viewport-prepare".into())
                .spawn(move || {
                    // If the window was closed or recreated, drop the obsolete result.
                    let _ = sender.send(job());
                })
                .map_err(|error| format!("Could not start graphics preparation: {error}"))?;
            self.receiver = Some(receiver);
        }
        let Some(receiver) = &self.receiver else {
            return Ok(None);
        };
        match receiver.try_recv() {
            Ok(value) => {
                self.receiver = None;
                Ok(Some(value))
            }
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => {
                self.receiver = None;
                Err("Graphics preparation failed; see the application log.".into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn waits_for_frame_and_prepares_once_without_blocking() {
        let (entered, started) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let mut startup = RendererStartup::new(move || {
            entered.send(()).unwrap();
            gate.recv().unwrap();
            42
        });
        assert_eq!(startup.tick().unwrap(), None);
        assert!(started.try_recv().is_err());
        startup.after_frame();
        assert_eq!(startup.tick().unwrap(), None);
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        startup.after_frame();
        assert_eq!(startup.tick().unwrap(), None);
        release.send(()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(value) = startup.tick().unwrap() {
                assert_eq!(value, 42);
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(startup.tick().unwrap(), None);
    }

    #[test]
    fn close_does_not_wait_for_running_preparation() {
        let (release, gate) = mpsc::channel();
        let (done, completed) = mpsc::channel();
        let mut startup = RendererStartup::new(move || {
            gate.recv().unwrap();
            done.send(()).unwrap();
        });
        startup.after_frame();
        startup.tick().unwrap();
        drop(startup);
        release.send(()).unwrap();
        completed.recv_timeout(Duration::from_secs(5)).unwrap();
    }

    #[test]
    fn cancelled_worker_destroys_unclaimed_result() {
        struct ResultGuard(mpsc::Sender<()>);
        impl Drop for ResultGuard {
            fn drop(&mut self) {
                self.0.send(()).unwrap();
            }
        }
        let (release, gate) = mpsc::channel();
        let (destroyed, dropped) = mpsc::channel();
        let mut startup = RendererStartup::new(move || {
            gate.recv().unwrap();
            ResultGuard(destroyed)
        });
        startup.after_frame();
        assert!(startup.tick().unwrap().is_none());
        drop(startup);
        release.send(()).unwrap();
        dropped.recv_timeout(Duration::from_secs(5)).unwrap();
    }

    #[test]
    fn worker_failure_is_reported_once() {
        let mut startup = RendererStartup::<()>::new(|| panic!("injected preparation failure"));
        startup.after_frame();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while startup.tick().is_ok() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(startup.tick().unwrap(), None);
    }
}
