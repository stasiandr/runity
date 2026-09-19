use std::io;

/// Something that can carry length-prefixed frames between two ends.
///
/// [`send`](Transport::send) hands one frame off to the transport;
/// [`drain`](Transport::drain) returns every frame that has fully arrived
/// since the last call without blocking to wait for more. Two frames handed
/// to `send` in order are guaranteed to come back out of `drain` in that same
/// order, though not necessarily in the same batch.
pub trait Transport {
    /// Sends `frame` to the other end. Fails if `frame` is larger than
    /// [`crate::MAX_FRAME_LEN`] or the underlying transport fails.
    fn send(&mut self, frame: &[u8]) -> io::Result<()>;

    /// Returns the frames that have completely arrived so far, oldest first,
    /// leaving any not-yet-complete frame buffered for the next call.
    fn drain(&mut self) -> io::Result<Vec<Vec<u8>>>;
}
