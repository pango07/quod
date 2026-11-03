//! The listener while the server boots: a long start-up step (the one-time
//! SQLite import into Postgres and the bucket) must not look like a dead
//! instance. Until `boot` returns, `/healthz` answers 200
//! `{"status":"starting", phase, blobs_done, blobs_total}` and every other
//! path 503 `{"error":"starting"}`. It then lets go of the port and the
//! real server binds it.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// What boot is doing, for the warm-up `/healthz`.
pub static PHASE: Mutex<&'static str> = Mutex::new("starting");
/// Blobs the import has handled, and how many it will.
pub static BLOBS_DONE: AtomicU64 = AtomicU64::new(0);
pub static BLOBS_TOTAL: AtomicU64 = AtomicU64::new(0);

pub fn phase(p: &'static str) {
    *PHASE.lock().unwrap_or_else(|e| e.into_inner()) = p;
}

pub struct Warmup {
    stop: Arc<AtomicBool>,
    addr: std::net::SocketAddr,
    thread: Option<std::thread::JoinHandle<()>>,
}

fn answer(mut s: TcpStream) {
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buf = [0u8; 2048];
    let n = s.read(&mut buf).unwrap_or(0);
    let req = String::from_utf8_lossy(&buf[..n]);
    let path = req.split_whitespace().nth(1).unwrap_or("");
    let (code, body) = if path == "/healthz" {
        let phase = *PHASE.lock().unwrap_or_else(|e| e.into_inner());
        (
            "200 OK",
            serde_json::json!({
                "status": "starting",
                "phase": phase,
                "blobs_done": BLOBS_DONE.load(Ordering::Relaxed),
                "blobs_total": BLOBS_TOTAL.load(Ordering::Relaxed),
            })
            .to_string(),
        )
    } else {
        (
            "503 Service Unavailable",
            r#"{"error":"starting","message":"the server is starting (importing data); try again shortly"}"#
                .to_string(),
        )
    };
    let _ = write!(
        s,
        "HTTP/1.1 {code}\r\ncontent-type: application/json\r\nretry-after: 10\r\nquod-protocol: quod.v1\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
}

impl Warmup {
    /// Binds `bind:port` and answers until `stop`. `None` (with a log line)
    /// when the port cannot be bound: boot goes on without it.
    pub fn start(bind: &str, port: u16) -> Option<Warmup> {
        let l = match TcpListener::bind((bind, port)) {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!(target: "quod", error = %e, "warm-up listener not started");
                return None;
            }
        };
        let addr = l.local_addr().ok()?;
        let stop = Arc::new(AtomicBool::new(false));
        let st = stop.clone();
        let thread = std::thread::spawn(move || {
            for s in l.incoming() {
                if st.load(Ordering::Relaxed) {
                    break;
                }
                if let Ok(s) = s {
                    std::thread::spawn(move || answer(s));
                }
            }
        });
        tracing::info!(target: "quod", %addr, "warm-up listener: /healthz answers while the server starts");
        Some(Warmup {
            stop,
            addr,
            thread: Some(thread),
        })
    }

    /// Stops answering and frees the port.
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Wake the accept loop.
        let wake = if self.addr.ip().is_unspecified() {
            std::net::SocketAddr::from(([127, 0, 0, 1], self.addr.port()))
        } else {
            self.addr
        };
        let _ = TcpStream::connect_timeout(&wake, Duration::from_secs(2));
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
