//! Small synchronization primitives used by the parallel fetcher. These
//! exist so the crate doesn't need an async runtime just to coordinate
//! N concurrent libgit2 calls behind a concurrency cap.

use std::any::Any;
use std::sync::{Condvar, Mutex};

/// Counting semaphore. Threads call [`acquire`] and block until a permit
/// is available; the returned [`Permit`] releases the permit on drop.
pub struct Semaphore {
    permits: Mutex<usize>,
    cv: Condvar,
}

impl Semaphore {
    pub fn new(permits: usize) -> Self {
        Self {
            permits: Mutex::new(permits),
            cv: Condvar::new(),
        }
    }

    pub fn acquire(&self) -> Permit<'_> {
        let mut g = self.permits.lock().expect("semaphore poisoned");
        while *g == 0 {
            g = self.cv.wait(g).expect("semaphore poisoned");
        }
        *g -= 1;
        Permit { sem: self }
    }
}

pub struct Permit<'a> {
    sem: &'a Semaphore,
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        let mut g = self.sem.permits.lock().expect("semaphore poisoned");
        *g += 1;
        self.sem.cv.notify_one();
    }
}

/// Convert a panic payload (the `Err` value of `JoinHandle::join`) into a
/// printable string. Tries the two common panic payload types
/// (`&'static str` and `String`) before falling back to a placeholder.
pub fn panic_payload_to_string(payload: Box<dyn Any + Send + 'static>) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        return (*s).to_string();
    }
    if let Some(s) = payload.downcast_ref::<String>() {
        return s.clone();
    }
    "<non-string panic payload>".to_string()
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
        thread,
        time::Duration,
    };

    use super::Semaphore;

    #[test]
    fn payload_extraction_string_literal() {
        let h = thread::spawn(|| panic!("boom"));
        let err = h.join().unwrap_err();
        assert_eq!(super::panic_payload_to_string(err), "boom");
    }

    #[test]
    fn payload_extraction_owned_string() {
        let h = thread::spawn(|| panic!("dynamic: {}", 42));
        let err = h.join().unwrap_err();
        assert_eq!(super::panic_payload_to_string(err), "dynamic: 42");
    }

    #[test]
    fn payload_extraction_non_string_fallback() {
        let h = thread::spawn(|| std::panic::panic_any(123u32));
        let err = h.join().unwrap_err();
        assert_eq!(
            super::panic_payload_to_string(err),
            "<non-string panic payload>"
        );
    }

    #[test]
    fn permits_are_returned_on_drop() {
        let sem = Semaphore::new(2);
        let p1 = sem.acquire();
        let p2 = sem.acquire();
        drop(p1);
        let _p3 = sem.acquire(); // would block forever if drop didn't return
        drop(p2);
    }

    #[test]
    fn caps_concurrent_acquirers() {
        let sem = Arc::new(Semaphore::new(2));
        let in_flight = Arc::new(AtomicUsize::new(0));
        let max_in_flight = Arc::new(AtomicUsize::new(0));

        thread::scope(|s| {
            for _ in 0..16 {
                let sem = sem.clone();
                let in_flight = in_flight.clone();
                let max_in_flight = max_in_flight.clone();
                s.spawn(move || {
                    let _permit = sem.acquire();
                    let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                    max_in_flight.fetch_max(now, Ordering::SeqCst);
                    thread::sleep(Duration::from_millis(20));
                    in_flight.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });

        assert_eq!(in_flight.load(Ordering::SeqCst), 0);
        assert_eq!(max_in_flight.load(Ordering::SeqCst), 2);
    }
}
