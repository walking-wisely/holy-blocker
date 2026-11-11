use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::backoff::Backoff;
use crate::frame::{self, Decoded};
use crate::message::{self, Outbound};
use crate::status::ConnectionState;

const WRITE_TIMEOUT: Duration = Duration::from_secs(2);
const READ_CHUNK: usize = 16 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum SendError {
    NotConnected,
    Io,
}

struct Shared {
    state: Mutex<ConnectionState>,
    writer: Mutex<Option<UnixStream>>,
    stop: AtomicBool,
}

pub struct Link {
    shared: Arc<Shared>,
}

impl Link {
    pub fn start(path: PathBuf, backoff: Backoff) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(ConnectionState::NotConnected),
            writer: Mutex::new(None),
            stop: AtomicBool::new(false),
        });
        let worker = Arc::clone(&shared);
        thread::spawn(move || run(&worker, &path, backoff));
        Self { shared }
    }

    pub fn state(&self) -> ConnectionState {
        *self.shared.state.lock().unwrap()
    }

    pub fn send(&self, request: Outbound) -> Result<(), SendError> {
        let mut writer = self.shared.writer.lock().unwrap();
        let stream = writer.as_mut().ok_or(SendError::NotConnected)?;
        stream
            .write_all(&request.to_frame())
            .map_err(|_| SendError::Io)
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        close(&self.shared);
    }
}

fn set_state(shared: &Shared, state: ConnectionState) {
    *shared.state.lock().unwrap() = state;
}

fn close(shared: &Shared) {
    if let Some(stream) = shared.writer.lock().unwrap().take() {
        let _ = stream.shutdown(Shutdown::Both);
    }
    set_state(shared, ConnectionState::NotConnected);
}

fn run(shared: &Shared, path: &PathBuf, mut backoff: Backoff) {
    while !shared.stop.load(Ordering::SeqCst) {
        match UnixStream::connect(path) {
            Ok(stream) => {
                if serve(shared, stream) {
                    backoff.reset();
                }
                close(shared);
            }
            Err(_) => set_state(shared, ConnectionState::NotConnected),
        }
        sleep_unless_stopped(shared, backoff.next_delay());
    }
}

fn sleep_unless_stopped(shared: &Shared, total: Duration) {
    let step = Duration::from_millis(50);
    let mut slept = Duration::ZERO;
    while slept < total && !shared.stop.load(Ordering::SeqCst) {
        thread::sleep(step.min(total - slept));
        slept += step;
    }
}

fn serve(shared: &Shared, stream: UnixStream) -> bool {
    let Ok(mut reader) = stream.try_clone() else {
        return false;
    };
    let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
    *shared.writer.lock().unwrap() = Some(stream);
    set_state(shared, ConnectionState::Connected);

    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; READ_CHUNK];
    let mut received_a_frame = false;
    loop {
        match reader.read(&mut chunk) {
            Ok(0) | Err(_) => return received_a_frame,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
        loop {
            match frame::decode(&buf) {
                Decoded::NeedMore => break,
                Decoded::Oversized(_) => return received_a_frame,
                Decoded::Frame { body, consumed } => {
                    buf.drain(..consumed);
                    if message::parse_inbound(&body).is_err() {
                        return received_a_frame;
                    }
                    received_a_frame = true;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::time::Instant;

    struct Sock(PathBuf);

    impl Sock {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("hb-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_file(&dir);
            Sock(dir)
        }
    }

    impl Drop for Sock {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn fast() -> Backoff {
        Backoff::new(Duration::from_millis(20), Duration::from_millis(80))
    }

    fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ok() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn read_frame(stream: &mut UnixStream) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            if let Decoded::Frame { body, .. } = frame::decode(&buf) {
                return body;
            }
            stream.read_exact(&mut byte).unwrap();
            buf.push(byte[0]);
        }
    }

    #[test]
    fn is_not_connected_while_no_daemon_listens() {
        let sock = Sock::new("absent");
        let link = Link::start(sock.0.clone(), fast());
        thread::sleep(Duration::from_millis(100));
        assert_eq!(link.state(), ConnectionState::NotConnected);
        assert_eq!(link.send(Outbound::ListApps), Err(SendError::NotConnected));
    }

    #[test]
    fn connects_once_the_daemon_appears_and_sends_a_framed_request() {
        let sock = Sock::new("appears");
        let link = Link::start(sock.0.clone(), fast());
        thread::sleep(Duration::from_millis(100));
        let listener = UnixListener::bind(&sock.0).unwrap();
        let (mut server, _) = listener.accept().unwrap();
        wait_for("connected", || link.state() == ConnectionState::Connected);

        link.send(Outbound::RequestDisarm).unwrap();
        assert_eq!(read_frame(&mut server), br#"{"type":"request_disarm"}"#);
    }

    #[test]
    fn reconnects_after_the_daemon_drops_the_connection() {
        let sock = Sock::new("reconnect");
        let listener = UnixListener::bind(&sock.0).unwrap();
        let link = Link::start(sock.0.clone(), fast());
        let (first, _) = listener.accept().unwrap();
        wait_for("connected", || link.state() == ConnectionState::Connected);

        drop(first);
        wait_for("disconnect seen", || {
            link.state() == ConnectionState::NotConnected
        });
        let (mut second, _) = listener.accept().unwrap();
        wait_for("reconnected", || link.state() == ConnectionState::Connected);
        link.send(Outbound::CancelDisarm).unwrap();
        assert_eq!(read_frame(&mut second), br#"{"type":"cancel_disarm"}"#);
    }

    #[test]
    fn drops_the_connection_on_a_frame_that_is_not_a_typed_message() {
        let sock = Sock::new("garbage");
        let listener = UnixListener::bind(&sock.0).unwrap();
        let link = Link::start(sock.0.clone(), fast());
        let (mut server, _) = listener.accept().unwrap();
        wait_for("connected", || link.state() == ConnectionState::Connected);

        server
            .write_all(&frame::encode(b"not json").unwrap())
            .unwrap();
        let _second = listener.accept().unwrap();
        wait_for("second connection", || {
            link.state() == ConnectionState::Connected
        });
    }

    #[test]
    fn drops_the_connection_on_an_oversized_length_prefix() {
        let sock = Sock::new("oversized");
        let listener = UnixListener::bind(&sock.0).unwrap();
        let link = Link::start(sock.0.clone(), fast());
        let (mut server, _) = listener.accept().unwrap();
        wait_for("connected", || link.state() == ConnectionState::Connected);

        server
            .write_all(&((frame::MAX_BODY as u32) + 1).to_be_bytes())
            .unwrap();
        let _second = listener.accept().unwrap();
        wait_for("second connection", || {
            link.state() == ConnectionState::Connected
        });
    }

    #[test]
    fn a_valid_message_split_across_writes_keeps_the_connection() {
        let sock = Sock::new("split");
        let listener = UnixListener::bind(&sock.0).unwrap();
        let link = Link::start(sock.0.clone(), fast());
        let (mut server, _) = listener.accept().unwrap();
        wait_for("connected", || link.state() == ConnectionState::Connected);

        let frame = frame::encode(br#"{"type":"status"}"#).unwrap();
        server.write_all(&frame[..3]).unwrap();
        thread::sleep(Duration::from_millis(50));
        server.write_all(&frame[3..]).unwrap();
        thread::sleep(Duration::from_millis(100));
        assert_eq!(link.state(), ConnectionState::Connected);
    }

    #[test]
    fn dropping_the_link_closes_the_connection() {
        let sock = Sock::new("drop");
        let listener = UnixListener::bind(&sock.0).unwrap();
        let link = Link::start(sock.0.clone(), fast());
        let (mut server, _) = listener.accept().unwrap();
        wait_for("connected", || link.state() == ConnectionState::Connected);

        drop(link);
        let mut byte = [0u8; 1];
        assert_eq!(server.read(&mut byte).unwrap(), 0);
    }
}
