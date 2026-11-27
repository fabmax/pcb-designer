//! Work off the frame: a thread on the desktop. The browser has no threads (without
//! special page headers), so there a job runs to completion when it's started.

/// A started job: [`Job::is_finished`] polls it, [`Job::join`] waits for its result.
pub struct Job<T> {
    #[cfg(not(target_arch = "wasm32"))]
    handle: std::thread::JoinHandle<T>,
    #[cfg(target_arch = "wasm32")]
    result: T,
}

/// Runs `f` in the background.
pub fn spawn<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Job<T> {
    #[cfg(not(target_arch = "wasm32"))]
    return Job { handle: std::thread::spawn(f) };
    #[cfg(target_arch = "wasm32")]
    return Job { result: f() };
}

impl<T> Job<T> {
    pub fn is_finished(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        return self.handle.is_finished();
        #[cfg(target_arch = "wasm32")]
        return true;
    }

    /// The result, or `Err` when the job panicked.
    pub fn join(self) -> std::thread::Result<T> {
        #[cfg(not(target_arch = "wasm32"))]
        return self.handle.join();
        #[cfg(target_arch = "wasm32")]
        return Ok(self.result);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_job_hands_its_result_back() {
        let job = super::spawn(|| 6 * 7);
        assert_eq!(job.join().ok(), Some(42));
    }

    #[test]
    fn a_panicking_job_is_an_error() {
        let job = super::spawn(|| -> u32 { panic!("test") });
        assert!(job.join().is_err());
    }
}
