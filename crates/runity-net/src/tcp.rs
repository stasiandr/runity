//! A [`Transport`] built directly on `std::net::TcpStream`.

use std::io::{self, ErrorKind, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};

use crate::framing;
use crate::transport::Transport;

/// One end of a TCP connection, framing every read and write through
/// [`framing`].
pub struct Tcp {
    stream: TcpStream,
    buf: Vec<u8>,
}

impl Tcp {
    /// Connects to `addr` and wraps the resulting socket.
    pub fn connect(addr: impl ToSocketAddrs) -> io::Result<Tcp> {
        Tcp::from_stream(TcpStream::connect(addr)?)
    }

    /// Wraps an already-connected socket, such as one returned by
    /// `TcpListener::accept`.
    pub fn from_stream(stream: TcpStream) -> io::Result<Tcp> {
        stream.set_nodelay(true)?;
        Ok(Tcp {
            stream,
            buf: Vec::new(),
        })
    }
}

impl Transport for Tcp {
    fn send(&mut self, frame: &[u8]) -> io::Result<()> {
        let mut out = Vec::with_capacity(4 + frame.len());
        framing::encode(frame, &mut out)?;
        self.stream.write_all(&out)
    }

    fn drain(&mut self) -> io::Result<Vec<Vec<u8>>> {
        self.stream.set_nonblocking(true)?;
        let result = read_available(&mut self.stream, &mut self.buf);
        self.stream.set_nonblocking(false)?;
        result?;
        framing::decode_all(&mut self.buf)
    }
}

/// Reads whatever is already sitting in the socket's buffer into `buf`,
/// stopping the moment there is nothing left to read without blocking.
fn read_available(stream: &mut TcpStream, buf: &mut Vec<u8>) -> io::Result<()> {
    let mut chunk = [0u8; 4096];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => return Ok(()), // peer closed the connection
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == ErrorKind::WouldBlock => return Ok(()),
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    fn connected_pair() -> (Tcp, Tcp) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let client = Tcp::connect(addr).unwrap();
        let (server_stream, _) = listener.accept().unwrap();
        let server = Tcp::from_stream(server_stream).unwrap();
        (client, server)
    }

    fn drain_until_nonempty(t: &mut Tcp) -> Vec<Vec<u8>> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let frames = t.drain().unwrap();
            if !frames.is_empty() || Instant::now() > deadline {
                return frames;
            }
            std::thread::yield_now();
        }
    }

    #[test]
    fn round_trips_a_frame_over_a_real_socket() {
        let (mut client, mut server) = connected_pair();
        client.send(b"hello").unwrap();
        assert_eq!(drain_until_nonempty(&mut server), vec![b"hello".to_vec()]);
    }

    #[test]
    fn round_trips_several_frames_in_order() {
        let (mut client, mut server) = connected_pair();
        client.send(b"one").unwrap();
        client.send(b"two").unwrap();
        let mut got = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        while got.len() < 2 && Instant::now() < deadline {
            got.extend(server.drain().unwrap());
        }
        assert_eq!(got, vec![b"one".to_vec(), b"two".to_vec()]);
    }

    #[test]
    fn send_rejects_an_oversized_frame_before_writing_it() {
        let (mut client, _server) = connected_pair();
        let oversized = vec![0u8; framing::MAX_FRAME_LEN + 1];
        assert!(client.send(&oversized).is_err());
    }

    #[test]
    fn drain_rejects_an_oversized_declared_length_from_the_peer() {
        let (mut client, mut server) = connected_pair();
        // Write a header declaring an oversized frame directly, bypassing
        // `Transport::send`'s own check, and never send a matching body: a
        // real body this size should never be allocated on the way in.
        let bad_len = (framing::MAX_FRAME_LEN + 1) as u32;
        client.stream.write_all(&bad_len.to_le_bytes()).unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match server.drain() {
                Ok(frames) if frames.is_empty() && Instant::now() < deadline => {
                    std::thread::yield_now();
                }
                Ok(frames) => panic!("expected rejection, got {frames:?}"),
                Err(e) => {
                    assert_eq!(e.kind(), ErrorKind::InvalidData);
                    break;
                }
            }
        }
    }
}
