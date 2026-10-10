//! Loopback-only test server (127.0.0.1, random port). Serves the local test page so that two panes can load
//! the SAME origin (the cookie-isolation test needs that) without touching any third-party site.
use std::io::{Read, Write};
use std::net::TcpListener;

const TESTPAGE: &str = include_str!("../ui/testpage.html");
const POPUP: &str = include_str!("../ui/popup.html");

pub fn start() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || {
                let mut s = stream;
                let mut buf = [0u8; 2048];
                let n = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let path = req
                    .lines()
                    .next()
                    .and_then(|l| l.split_whitespace().nth(1))
                    .unwrap_or("/");
                let (status, body) = if path.starts_with("/popup") {
                    ("200 OK", POPUP)
                } else if path == "/" || path.starts_with("/?") {
                    ("200 OK", TESTPAGE)
                } else {
                    ("404 Not Found", "not found")
                };
                let resp = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = s.write_all(resp.as_bytes());
            });
        }
    });
    port
}
