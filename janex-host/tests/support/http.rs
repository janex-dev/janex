// Copyright (c) 2026 Glavo
// SPDX-License-Identifier: MPL-2.0

//! Loopback HTTP fixture with explicit responses and observable request counts.

use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

/// Owns a loopback server which never depends on external connectivity.
pub struct Server {
    /// Base URL with an ephemeral port.
    pub url: String,
    /// Recorded request targets in arrival order.
    pub requests: Arc<Mutex<Vec<String>>>,
    /// Complete response bytes, indexed by request target.
    replies: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    /// Requests termination when the fixture is dropped.
    stop: Arc<AtomicBool>,
    /// Background server thread, joined during cleanup.
    thread: Option<JoinHandle<()>>,
}

impl Server {
    /// Starts a server on loopback; unregistered paths return HTTP 404.
    pub fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let replies = Arc::new(Mutex::new(BTreeMap::<String, Vec<u8>>::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (seen, responses, stopping) = (requests.clone(), replies.clone(), stop.clone());
        let thread = thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(error) => panic!("HTTP fixture accept: {error}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut input = BufReader::new(&mut stream);
                let mut line = String::new();
                input.read_line(&mut line).unwrap();
                let target = line.split_whitespace().nth(1).unwrap_or("").to_owned();
                loop {
                    line.clear();
                    if input.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                        break;
                    }
                }
                seen.lock().unwrap().push(target.clone());
                let response = responses
                    .lock()
                    .unwrap()
                    .get(&target)
                    .cloned()
                    .unwrap_or_else(|| {
                        b"HTTP/1.1 404 Missing\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_vec()
                    });
                let _ = stream.write_all(&response);
            }
        });
        Self {
            url,
            requests,
            replies,
            stop,
            thread: Some(thread),
        }
    }

    /// Publishes one complete success response.
    pub fn file(&self, path: &str, bytes: &[u8]) {
        let mut response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            bytes.len()
        )
        .into_bytes();
        response.extend(bytes);
        self.raw(path, response);
    }

    /// Publishes explicit response bytes for protocol-boundary tests.
    pub fn raw(&self, path: &str, response: Vec<u8>) {
        self.replies.lock().unwrap().insert(path.into(), response);
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let result = self.thread.take().unwrap().join();
        if !thread::panicking() {
            result.unwrap();
        }
    }
}
