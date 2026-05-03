use futures::FutureExt;
use std::fmt::Display;
use std::future::Future;

fn panic_payload_to_string(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

pub trait SpawnLoggedOutcome {
    fn log_if_error(self, task_name: &'static str);
}

impl SpawnLoggedOutcome for () {
    fn log_if_error(self, _task_name: &'static str) {}
}

impl<T, E> SpawnLoggedOutcome for std::result::Result<T, E>
where
    E: Display,
{
    fn log_if_error(self, task_name: &'static str) {
        if let Err(error) = self {
            tracing::error!(task = task_name, error = %error, "Background task failed");
        }
    }
}

pub fn spawn_logged<F>(task_name: &'static str, future: F) -> tokio::task::JoinHandle<()>
where
    F: Future + Send + 'static,
    F::Output: SpawnLoggedOutcome + Send + 'static,
{
    tokio::spawn(async move {
        match std::panic::AssertUnwindSafe(future).catch_unwind().await {
            Ok(output) => output.log_if_error(task_name),
            Err(payload) => {
                tracing::error!(
                    task = task_name,
                    panic = %panic_payload_to_string(payload),
                    "Background task panicked"
                );
            }
        }
    })
}

#[macro_export]
macro_rules! spawn_logged {
    ($task_name:expr, $future:expr) => {{ $crate::core::spawn::spawn_logged($task_name, $future) }};
}
