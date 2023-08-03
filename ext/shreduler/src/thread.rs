use magnus::{rb_sys::protect, Error, Value};
use tokio::time::Duration;

/// Task which checks for Ruby thread interrupts every so often.
pub async fn check_interrupts() -> Result<Value, Error> {
    let mut interval = tokio::time::interval(Duration::from_millis(1000));

    loop {
        interval.tick().await;

        let int = protect(|| unsafe {
            rb_sys::rb_thread_check_ints();
            rb_sys::Qnil as _
        });

        match int {
            Ok(_) => continue,
            Err(e) => return Err(e),
        }
    }
}
