//! A tiny loopback HTTP server for integration tests (no external dependency).

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

/// One canned answer: request path, status, extra headers, body.
pub type Route = (&'static str, u16, Vec<(&'static str, String)>, Vec<u8>);

/// Starts a server on `127.0.0.1:0` and returns its base URL (`http://127.0.0.1:<port>`).
///
/// Unknown paths get 404. The server lives as long as the test process.
pub fn serve(routes: Vec<Route>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let routes = routes.clone();
            thread::spawn(move || handle(stream, &routes));
        }
    });
    format!("http://127.0.0.1:{port}")
}

fn handle(stream: TcpStream, routes: &[Route]) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    // Drain the headers.
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) if line == "\r\n" || line == "\n" => break,
            Ok(_) => {}
        }
    }
    let path = request_line.split_whitespace().nth(1).unwrap_or("/");
    let path = path.split('?').next().unwrap_or(path);
    let (status, headers, body) = routes
        .iter()
        .find(|r| r.0 == path)
        .map(|r| (r.1, r.2.clone(), r.3.clone()))
        .unwrap_or((404, Vec::new(), b"not found".to_vec()));
    let mut out = format!(
        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (k, v) in headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str("\r\n");
    let mut stream = stream;
    let _ = stream.write_all(out.as_bytes());
    let _ = stream.write_all(&body);
    let _ = stream.flush();
}
