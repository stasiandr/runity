//! A zero-thread [`Transport`] for two ends in the same process.
//!
//! `Loopback` still runs every frame through [`framing::encode`] and
//! [`framing::decode_all`] on a plain byte buffer, rather than handing the
//! other end its `Vec<u8>` directly. That is the whole point: single-player
//! pays the same serialization path a networked game pays, just without a
//! socket or a thread underneath it.

use std::cell::RefCell;
use std::io;
use std::rc::Rc;

use crate::framing;
use crate::transport::Transport;

/// One end of an in-memory, single-threaded connection created by
/// [`Loopback::pair`].
pub struct Loopback {
    outgoing: Rc<RefCell<Vec<u8>>>,
    incoming: Rc<RefCell<Vec<u8>>>,
}

impl Loopback {
    /// Creates two connected ends. Nothing sent on one is visible to the
    /// other until [`Transport::drain`] is called on it.
    pub fn pair() -> (Loopback, Loopback) {
        let a_to_b = Rc::new(RefCell::new(Vec::new()));
        let b_to_a = Rc::new(RefCell::new(Vec::new()));
        let a = Loopback {
            outgoing: a_to_b.clone(),
            incoming: b_to_a.clone(),
        };
        let b = Loopback {
            outgoing: b_to_a,
            incoming: a_to_b,
        };
        (a, b)
    }
}

impl Transport for Loopback {
    fn send(&mut self, frame: &[u8]) -> io::Result<()> {
        framing::encode(frame, &mut self.outgoing.borrow_mut())
    }

    fn drain(&mut self) -> io::Result<Vec<Vec<u8>>> {
        framing::decode_all(&mut self.incoming.borrow_mut())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_frame_between_the_two_ends() {
        let (mut a, mut b) = Loopback::pair();
        a.send(b"hello").unwrap();
        assert_eq!(b.drain().unwrap(), vec![b"hello".to_vec()]);
    }

    #[test]
    fn is_a_two_way_pipe() {
        let (mut a, mut b) = Loopback::pair();
        a.send(b"ping").unwrap();
        b.send(b"pong").unwrap();
        assert_eq!(b.drain().unwrap(), vec![b"ping".to_vec()]);
        assert_eq!(a.drain().unwrap(), vec![b"pong".to_vec()]);
    }

    #[test]
    fn queues_frames_until_drained() {
        let (mut a, mut b) = Loopback::pair();
        a.send(b"one").unwrap();
        a.send(b"two").unwrap();
        assert_eq!(b.drain().unwrap(), vec![b"one".to_vec(), b"two".to_vec()]);
        assert!(b.drain().unwrap().is_empty());
    }

    #[test]
    fn rejects_an_oversized_frame_without_delivering_it() {
        let (mut a, mut b) = Loopback::pair();
        let oversized = vec![0u8; framing::MAX_FRAME_LEN + 1];
        assert!(a.send(&oversized).is_err());
        assert!(b.drain().unwrap().is_empty());
    }
}
