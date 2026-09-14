//! The control socket's listener, and the client that talks to it.
//!
//! The protocol itself lives in [`crate::control`], away from any sockets, so
//! it can be tested without one. This file is only plumbing.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc;

use crate::control::{socket_path, Request};

/// One request, and somewhere to put the answer.
///
/// The reply travels back on its own channel rather than being written by the
/// listener thread: only the main loop knows the current settings, and having
/// two threads answer would mean two sources of truth.
pub struct Call {
    pub request: Request,
    pub reply: mpsc::Sender<String>,
}

/// Bind the socket, replacing a stale one left by a process that is gone.
pub fn bind() -> std::io::Result<(UnixListener, PathBuf)> {
    let path = socket_path().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "XDG_RUNTIME_DIR is unset, so there is nowhere to put the control socket",
        )
    })?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // A socket file left behind by a crash would make bind fail forever. Only
    // remove one nothing is listening on, so two daemons cannot silently steal
    // the socket from each other.
    if path.exists() && UnixStream::connect(&path).is_err() {
        let _ = std::fs::remove_file(&path);
    }

    let listener = UnixListener::bind(&path)?;
    Ok((listener, path))
}

/// Serve the socket forever, handing each request to `sink` and writing back
/// whatever the main loop answers.
///
/// One connection, one request, one reply, then close. A persistent connection
/// would need a protocol for framing replies and gains nothing: every caller
/// here is a short-lived CLI invocation or a bar widget polling.
pub fn serve<F>(listener: UnixListener, mut sink: F)
where
    F: FnMut(Call),
{
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };

        let mut line = String::new();
        if BufReader::new(&stream).read_line(&mut line).is_err() {
            continue;
        }

        let answer = match Request::parse(line.trim()) {
            Ok(request) => {
                let (tx, rx) = mpsc::channel();
                sink(Call { request, reply: tx });
                // A main loop that never answers must not wedge the socket
                // thread; the caller gets an error and can try again.
                rx.recv_timeout(std::time::Duration::from_secs(2))
                    .unwrap_or_else(|_| crate::control::error_json("nimbus did not answer in time"))
            }
            Err(e) => crate::control::error_json(&e),
        };

        let _ = writeln!(stream, "{answer}");
        let _ = stream.flush();
    }
}

/// Send one request to a running daemon and return its reply.
pub fn ask(request: &Request) -> Result<String, String> {
    let path = socket_path().ok_or("XDG_RUNTIME_DIR is unset")?;
    let mut stream = UnixStream::connect(&path).map_err(|e| {
        format!(
            "no nimbus is running ({e}).\n\
             Start one with: nimbus-wayland &"
        )
    })?;
    writeln!(stream, "{}", request.to_json()).map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())?;

    let mut reply = String::new();
    stream.read_to_string(&mut reply).map_err(|e| e.to_string())?;
    Ok(reply.trim().to_string())
}
