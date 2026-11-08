//! Overlap the independent environment pin and checkout import. No task
//! outlives preparation: errors and panics retain their original meaning,
//! and the caller still validates the pin before using the imported tree.

use std::time::{Duration, Instant};

pub struct Prepared<P, I> {
    pub pin: P,
    pub checkout: Option<I>,
    pub pin_time: Duration,
    pub checkout_time: Duration,
}

fn timed<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let start = Instant::now();
    let value = f();
    (value, start.elapsed())
}

/// `serial` is a diagnostic control: identical work, in the old order.
/// Network steps need no checkout import at all.
pub fn inputs<P, I: Send>(
    needs_checkout: bool,
    serial: bool,
    pin: impl FnOnce() -> P,
    import: impl FnOnce() -> I + Send,
) -> Prepared<P, I> {
    if needs_checkout && !serial {
        std::thread::scope(|scope| {
            let checkout = scope.spawn(|| timed(import));
            let (pin, pin_time) = timed(pin);
            let (checkout, checkout_time) = checkout
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
            Prepared {
                pin,
                checkout: Some(checkout),
                pin_time,
                checkout_time,
            }
        })
    } else {
        let (pin, pin_time) = timed(pin);
        let (checkout, checkout_time) = if needs_checkout {
            let (checkout, elapsed) = timed(import);
            (Some(checkout), elapsed)
        } else {
            (None, Duration::ZERO)
        };
        Prepared {
            pin,
            checkout,
            pin_time,
            checkout_time,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, mpsc};

    use super::*;

    #[test]
    fn checkout_can_progress_while_the_pin_is_waiting() {
        let (send, receive) = mpsc::sync_channel(0);
        let prepared = inputs(
            true,
            false,
            || receive.recv_timeout(Duration::from_secs(5)).unwrap(),
            || {
                send.send(17).unwrap();
                "whole checkout"
            },
        );
        assert_eq!(prepared.pin, 17);
        assert_eq!(prepared.checkout, Some("whole checkout"));
    }

    #[test]
    fn serial_control_keeps_the_original_order() {
        let order = Mutex::new(Vec::new());
        let prepared = inputs(
            true,
            true,
            || order.lock().unwrap().push("pin"),
            || order.lock().unwrap().push("checkout"),
        );
        assert_eq!(*order.lock().unwrap(), ["pin", "checkout"]);
        assert_eq!(prepared.checkout, Some(()));
    }

    #[test]
    fn network_steps_never_import_the_checkout() {
        for serial in [false, true] {
            let prepared = inputs(false, serial, || 23, || -> usize { panic!("unneeded import") });
            assert_eq!(prepared.pin, 23);
            assert!(prepared.checkout.is_none());
            assert_eq!(prepared.checkout_time, Duration::ZERO);
        }
    }

    #[test]
    fn failed_import_is_preserved_and_joined() {
        let prepared = inputs(true, false, || 23, || Err::<(), _>("unreadable checkout"));
        assert_eq!(prepared.pin, 23);
        assert_eq!(prepared.checkout, Some(Err("unreadable checkout")));
    }

    #[test]
    #[should_panic(expected = "import panic")]
    fn an_import_panic_is_not_disguised_as_a_cache_miss() {
        inputs(true, false, || 23, || -> usize { panic!("import panic") });
    }
}
