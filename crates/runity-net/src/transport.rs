//! Getting bytes to the other machine.
//!
//! The protocol above this does not care how: [`Connection`](crate::Connection)
//! takes bytes and returns bytes, so the socket is a detail that can be
//! swapped for a test double. That is not architectural neatness for its own
//! sake — it is the difference between testing reliability against a link that
//! loses a third of everything and testing it against whatever the machine's
//! loopback happened to do that afternoon.

use std::io;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};

/// Somewhere to send datagrams.
pub trait Transport {
    /// How a peer is named.
    type Address: Copy + Eq + Ord + core::fmt::Debug;

    /// Send one datagram. Failure to deliver is normal and not an error.
    fn send(&mut self, to: Self::Address, bytes: &[u8]) -> io::Result<()>;

    /// Take one datagram if there is one waiting. Never blocks.
    fn receive(&mut self, buffer: &mut [u8]) -> io::Result<Option<(Self::Address, usize)>>;
}

/// A UDP socket.
///
/// UDP and not TCP because a game needs to choose, per message, between "must
/// arrive" and "must be recent". TCP offers only the first and makes the
/// second wait behind it.
#[derive(Debug)]
pub struct UdpTransport {
    socket: UdpSocket,
}

impl UdpTransport {
    /// Bind a non-blocking socket.
    ///
    /// Use port 0 to let the operating system choose one — which is what a
    /// client wants, and what a test wants so that two of them can run at
    /// once.
    pub fn bind(address: impl ToSocketAddrs) -> io::Result<Self> {
        let socket = UdpSocket::bind(address)?;
        socket.set_nonblocking(true)?;
        Ok(Self { socket })
    }

    /// The address actually bound.
    pub fn local_address(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }
}

impl Transport for UdpTransport {
    type Address = SocketAddr;

    fn send(&mut self, to: SocketAddr, bytes: &[u8]) -> io::Result<()> {
        match self.socket.send_to(bytes, to) {
            Ok(_) => Ok(()),
            // A full send buffer is congestion, not a failure: the packet is
            // dropped, which is what the protocol above already handles.
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(()),
            // Some systems report an earlier ICMP rejection on the next send.
            // A peer that has gone is the timeout's business, not ours.
            Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => Ok(()),
            Err(error) => Err(error),
        }
    }

    fn receive(&mut self, buffer: &mut [u8]) -> io::Result<Option<(SocketAddr, usize)>> {
        match self.socket.recv_from(buffer) {
            Ok((length, from)) => Ok(Some((from, length))),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => Ok(None),
            Err(error) => Err(error),
        }
    }
}
