//! `Server-Timing` on every answer: where a request's time went, for the
//! browser's developer tools. `db` is the time a Postgres connection was
//! held, `s3` the time in bucket requests, `app` the blocking work (which
//! includes both), `total` the whole request.
//!
//! The middleware gives each request a `Timing` (a tokio task-local);
//! `blocking` hands it to the worker thread (a thread-local for the
//! closure's duration), where the database and bucket code add to it.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use axum::extract::Request;
use axum::http::HeaderValue;
use axum::middleware::Next;
use axum::response::Response;

#[derive(Default, Debug)]
pub struct Timing {
    db: AtomicU64,
    s3: AtomicU64,
    app: AtomicU64,
}

tokio::task_local! {
    static REQUEST: Arc<Timing>;
}

thread_local! {
    static CURRENT: RefCell<Option<Arc<Timing>>> = const { RefCell::new(None) };
}

fn add(f: impl Fn(&Timing) -> &AtomicU64, d: Duration) {
    CURRENT.with(|c| {
        if let Some(t) = c.borrow().as_ref() {
            f(t).fetch_add(d.as_micros() as u64, Ordering::Relaxed);
        }
    });
}

pub fn add_db(d: Duration) {
    add(|t| &t.db, d);
}

pub fn add_s3(d: Duration) {
    add(|t| &t.s3, d);
}

/// The request's timing, from async code (for `blocking`).
pub fn current() -> Option<Arc<Timing>> {
    REQUEST.try_with(Arc::clone).ok()
}

/// Runs `f` with `t` as this thread's timing, counting it as `app`.
pub fn with<T>(t: Option<Arc<Timing>>, f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let prev = CURRENT.with(|c| c.replace(t.clone()));
    let out = f();
    CURRENT.with(|c| *c.borrow_mut() = prev);
    if let Some(t) = t {
        t.app
            .fetch_add(start.elapsed().as_micros() as u64, Ordering::Relaxed);
    }
    out
}

/// Measures a span that adds to `db` when dropped (a held connection).
pub struct DbSpan(Instant);

impl DbSpan {
    pub fn start() -> DbSpan {
        DbSpan(Instant::now())
    }
}

impl Drop for DbSpan {
    fn drop(&mut self) {
        add_db(self.0.elapsed());
    }
}

/// The middleware: a `Timing` per request, and the header on the answer.
pub async fn layer(req: Request, next: Next) -> Response {
    let t = Arc::new(Timing::default());
    let start = Instant::now();
    let mut resp = REQUEST.scope(t.clone(), next.run(req)).await;
    let ms = |v: &AtomicU64| v.load(Ordering::Relaxed) as f64 / 1000.0;
    let h = format!(
        "db;dur={:.1}, s3;dur={:.1}, app;dur={:.1}, total;dur={:.1}",
        ms(&t.db),
        ms(&t.s3),
        ms(&t.app),
        start.elapsed().as_secs_f64() * 1000.0
    );
    if let Ok(v) = HeaderValue::from_str(&h) {
        resp.headers_mut().insert("server-timing", v);
    }
    resp
}
