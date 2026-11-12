//! A long sound streamed without a thread: decoded a little ahead on the
//! game's own thread, each frame ([`crate::audio::Audio::pump`]), into a ring
//! the mixer plays from.
//!
//! The browser has no thread to stream on, and decoding a whole track when it
//! starts is a stall and a lot of memory (two minutes of stereo at 48 kHz is
//! 46 MB as the mixer's frames). Pumped, a track costs its compressed bytes
//! and two seconds of frames, and a frame decodes about as much as it plays.
//! Loops wrap where the stream ends, sample-exact: the decoder trims the
//! encoder's delay and padding (gapless), so the frame after the last is the
//! first.
//!
//! Desktop and phone builds stream on kira's own thread instead; this is the
//! same sound played the other way, and a test plays both.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use kira::info::Info;
use kira::sound::Sound;
use kira::{Frame, Panning};
use rtrb::{Consumer, Producer, RingBuffer};
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

/// Seconds decoded ahead of what plays.
pub const AHEAD_SECONDS: f32 = 2.0;
/// At most this much decoded in one pump: a track starting does not stall
/// the frame it starts on (the rest follows over the next frames).
pub const PUMP_SECONDS: f32 = 0.5;

/// What the game and the mixer share about one pumped sound.
#[derive(Debug)]
pub(crate) struct Shared {
    /// Linear gain, as f32 bits.
    volume: AtomicU32,
    /// −1..1, as f32 bits.
    panning: AtomicU32,
    /// Asked to stop: fades out over a few milliseconds, then is finished.
    stop: AtomicBool,
    /// Played out (a one-shot past its end) or stopped and silent.
    finished: AtomicBool,
    /// Frames the mixer found the ring empty while it should have played:
    /// a pump that came too late.
    starved: AtomicU64,
    /// Frames played so far, of the sound's own rate.
    played: AtomicU64,
}

impl Shared {
    fn new(volume: f32) -> Self {
        Self {
            volume: AtomicU32::new(volume.to_bits()),
            panning: AtomicU32::new(0f32.to_bits()),
            stop: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            starved: AtomicU64::new(0),
            played: AtomicU64::new(0),
        }
    }
}

/// The game's hold on a pumped sound.
#[derive(Debug, Clone)]
pub struct PumpHandle(pub(crate) Arc<Shared>);

impl PumpHandle {
    pub fn set_volume(&self, gain: f32) {
        let gain = if gain.is_finite() { gain.max(0.0) } else { 0.0 };
        self.0.volume.store(gain.to_bits(), Ordering::Relaxed);
    }

    pub fn set_panning(&self, pan: f32) {
        self.0.panning.store(pan.clamp(-1.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    pub fn stop(&self) {
        self.0.stop.store(true, Ordering::Relaxed);
    }

    pub fn is_playing(&self) -> bool {
        !self.0.finished.load(Ordering::Relaxed)
    }

    /// Frames the mixer wanted and the ring did not have yet.
    pub fn starved(&self) -> u64 {
        self.0.starved.load(Ordering::Relaxed)
    }

    /// Frames played, at the sound's own rate.
    pub fn played(&self) -> u64 {
        self.0.played.load(Ordering::Relaxed)
    }
}

/// The decoding side, kept by the game and pumped each frame.
pub(crate) struct Feed {
    bytes: Arc<[u8]>,
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track: u32,
    producer: Producer<Frame>,
    looped: bool,
    /// Decoded and not yet in the ring.
    pending: VecDeque<Frame>,
    scratch: Vec<f32>,
    /// Every frame decoded (a one-shot at its end): the ring is all there is.
    pub(crate) done: Arc<AtomicBool>,
    shared: Arc<Shared>,
    rate: u32,
}

/// A stream opened: its reader, its decoder, its track and its rate.
type Opened = (Box<dyn FormatReader>, Box<dyn AudioDecoder>, u32, u32);

/// A stream's reader and decoder from its start.
fn open(bytes: &Arc<[u8]>) -> Result<Opened, String> {
    let source = MediaSourceStream::new(Box::new(std::io::Cursor::new(ArcBytes(bytes.clone()))), Default::default());
    let reader = symphonia::default::get_probe()
        .probe(&Hint::new(), source, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| e.to_string())?;
    let track = reader.default_track(TrackType::Audio).ok_or("no sound in it")?;
    let id = track.id;
    let params = track.codec_params.as_ref().and_then(|p| p.audio()).ok_or("not a sound track")?.clone();
    let rate = params.sample_rate.unwrap_or(44_100);
    let decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|e| e.to_string())?;
    Ok((reader, decoder, id, rate))
}

/// Shared bytes, readable as a cursor's.
struct ArcBytes(Arc<[u8]>);

impl AsRef<[u8]> for ArcBytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// A compressed sound (Ogg Vorbis, MP3, FLAC, WAV) to pump: its feed, its
/// sound for the mixer, its handle.
pub(crate) fn start(bytes: Arc<[u8]>, gain: f32, looped: bool) -> Result<(Feed, PumpedSound, PumpHandle), String> {
    let (reader, decoder, track, rate) = open(&bytes)?;
    let capacity = (rate as f32 * AHEAD_SECONDS) as usize + 1;
    let (producer, consumer) = RingBuffer::new(capacity);
    let shared = Arc::new(Shared::new(gain));
    let done = Arc::new(AtomicBool::new(false));
    let mut feed = Feed { bytes, reader, decoder, track, producer, looped, pending: VecDeque::new(), scratch: Vec::new(), done: done.clone(), shared: shared.clone(), rate };
    feed.pump();
    let sound = PumpedSound { consumer, rate, frac: 0.0, a: Frame::ZERO, b: Frame::ZERO, primed: false, gain: gain.max(0.0), pan: 0.0, shared: shared.clone(), done };
    Ok((feed, sound, PumpHandle(shared)))
}

impl Feed {
    /// Whether the mixer has let it go (finished or dropped): the feed can go.
    pub(crate) fn is_over(&self) -> bool {
        self.shared.finished.load(Ordering::Relaxed) || self.producer.is_abandoned()
    }

    /// Decode ahead into the ring: until it is full, or [`PUMP_SECONDS`] of
    /// sound this call. A loop starts over where the stream ends.
    pub(crate) fn pump(&mut self) {
        if self.done.load(Ordering::Relaxed) || self.is_over() {
            return;
        }
        let budget = (self.rate as f32 * PUMP_SECONDS) as usize;
        let mut moved = 0;
        while moved < budget {
            while let Some(&frame) = self.pending.front() {
                if self.producer.push(frame).is_err() {
                    return;
                }
                self.pending.pop_front();
                moved += 1;
            }
            if self.producer.slots() == 0 {
                return;
            }
            if !self.decode_packet() {
                if self.looped {
                    match open(&self.bytes) {
                        Ok((reader, decoder, track, _)) => {
                            self.reader = reader;
                            self.decoder = decoder;
                            self.track = track;
                        }
                        Err(_) => {
                            self.done.store(true, Ordering::Relaxed);
                            return;
                        }
                    }
                } else {
                    self.done.store(true, Ordering::Relaxed);
                    return;
                }
            }
        }
    }

    /// The next packet's frames into `pending`; false at the stream's end
    /// (or a stream that cannot be read on).
    fn decode_packet(&mut self) -> bool {
        use symphonia::core::audio::sample::Sample;
        use symphonia::core::errors::Error;
        loop {
            let packet = match self.reader.next_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) | Err(_) => return false,
            };
            if packet.track_id != self.track {
                continue;
            }
            match self.decoder.decode(&packet) {
                Ok(buffer) => {
                    let channels = buffer.spec().channels().count().max(1);
                    self.scratch.resize(buffer.samples_interleaved(), f32::MID);
                    buffer.copy_to_slice_interleaved(&mut self.scratch);
                    for frame in self.scratch.chunks(channels) {
                        let left = frame[0];
                        self.pending.push_back(Frame::new(left, frame.get(1).copied().unwrap_or(left)));
                    }
                    return true;
                }
                Err(Error::DecodeError(_)) => continue,
                Err(_) => return false,
            }
        }
    }
}

/// The mixer's side: frames out of the ring, resampled to the device's
/// rate (linear), at the game's volume and panning, smoothed.
pub(crate) struct PumpedSound {
    consumer: Consumer<Frame>,
    rate: u32,
    frac: f64,
    a: Frame,
    b: Frame,
    primed: bool,
    gain: f32,
    pan: f32,
    shared: Arc<Shared>,
    done: Arc<AtomicBool>,
}

impl PumpedSound {
    /// The next frame of the sound's own rate; `None` when there is none
    /// yet (starved) or none ever (played out).
    fn next(&mut self) -> Option<Frame> {
        match self.consumer.pop() {
            Ok(frame) => {
                self.shared.played.fetch_add(1, Ordering::Relaxed);
                Some(frame)
            }
            Err(_) => None,
        }
    }

    fn played_out(&self) -> bool {
        self.done.load(Ordering::Relaxed) && self.consumer.is_empty()
    }
}

impl Sound for PumpedSound {
    fn process(&mut self, out: &mut [Frame], dt: f64, _info: &Info) {
        if self.shared.finished.load(Ordering::Relaxed) {
            out.fill(Frame::ZERO);
            return;
        }
        let step = self.rate as f64 * dt;
        let stopping = self.shared.stop.load(Ordering::Relaxed);
        let target = if stopping { 0.0 } else { f32::from_bits(self.shared.volume.load(Ordering::Relaxed)) };
        let pan_target = f32::from_bits(self.shared.panning.load(Ordering::Relaxed));
        // About 5 ms to reach a new volume: a slider or a fade stepped once
        // a frame is a slope, not a staircase of clicks.
        let smooth = (1.0 - (-(dt as f32) / 0.005).exp()).clamp(0.0, 1.0);
        if !self.primed {
            self.b = self.next().unwrap_or(Frame::ZERO);
            self.primed = true;
        }
        for slot in out.iter_mut() {
            while self.frac >= 1.0 {
                self.frac -= 1.0;
                self.a = self.b;
                self.b = match self.next() {
                    Some(f) => f,
                    None => {
                        if !self.played_out() {
                            self.shared.starved.fetch_add(1, Ordering::Relaxed);
                        }
                        Frame::ZERO
                    }
                };
            }
            self.gain += (target - self.gain) * smooth;
            self.pan += (pan_target - self.pan) * smooth;
            let t = self.frac as f32;
            let frame = self.a + (self.b - self.a) * t;
            *slot = (frame * self.gain).panned(Panning(self.pan));
            self.frac += step;
        }
        if (stopping && self.gain < 1.0e-4) || (self.played_out() && self.a == Frame::ZERO && self.b == Frame::ZERO) {
            self.shared.finished.store(true, Ordering::Relaxed);
        }
    }

    fn finished(&self) -> bool {
        self.shared.finished.load(Ordering::Relaxed)
    }
}

/// Played through kira: hands the sound to the mixer.
pub(crate) struct PumpedData(pub(crate) PumpedSound);

impl kira::sound::SoundData for PumpedData {
    type Error = ();
    type Handle = ();

    fn into_sound(self) -> Result<(Box<dyn Sound>, Self::Handle), Self::Error> {
        Ok((Box::new(self.0), ()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One second of a 440 Hz tone in Ogg Vorbis, 48 kHz stereo (made with
    /// ffmpeg's encoder): 48 000 frames.
    const TONE: &[u8] = include_bytes!("../tests/tone_48k.ogg");

    fn all_frames(bytes: &[u8]) -> Vec<Frame> {
        let bytes: Arc<[u8]> = bytes.into();
        let (reader, decoder, track, rate) = open(&bytes).unwrap();
        let (producer, _consumer) = RingBuffer::new(1);
        let mut feed = Feed { bytes, reader, decoder, track, producer, looped: false, pending: VecDeque::new(), scratch: Vec::new(), done: Arc::new(AtomicBool::new(false)), shared: Arc::new(Shared::new(1.0)), rate };
        while feed.decode_packet() {}
        feed.pending.into_iter().collect()
    }

    fn info() -> Info<'static> {
        kira::info::MockInfoBuilder::new().build()
    }

    #[test]
    fn the_stream_decodes_to_exactly_its_length() {
        let frames = all_frames(TONE);
        assert_eq!(frames.len(), 48_000, "the encoder's delay and padding are trimmed");
        assert!(frames.iter().any(|f| f.left.abs() > 0.03), "the tone is there (−27 dBFS)");
    }

    #[test]
    fn a_pumped_sound_plays_what_the_stream_holds_and_ends() {
        let want = all_frames(TONE);
        let (mut feed, mut sound, handle) = start(TONE.into(), 1.0, false).unwrap();
        let mut out = vec![Frame::ZERO; 512];
        let mut got = Vec::new();
        // The device at the sound's own rate: frame for frame (one behind:
        // the first output is the frame before the first).
        for _ in 0..200 {
            feed.pump();
            sound.process(&mut out, 1.0 / 48_000.0, &info());
            got.extend_from_slice(&out);
            if sound.finished() {
                break;
            }
        }
        assert!(sound.finished(), "a one-shot ends");
        assert!(!handle.is_playing());
        assert_eq!(handle.starved(), 0, "pumped every block, never short");
        for (k, f) in want.iter().enumerate().step_by(997) {
            let g = got[k + 1];
            assert!((g.left - f.left).abs() < 1e-5 && (g.right - f.right).abs() < 1e-5, "frame {k}: {g:?} vs {f:?}");
        }
    }

    #[test]
    fn a_pumped_loop_wraps_sample_exact() {
        let want = all_frames(TONE);
        let (mut feed, mut sound, handle) = start(TONE.into(), 1.0, true).unwrap();
        let mut out = vec![Frame::ZERO; 480];
        let mut got = Vec::new();
        while got.len() < 48_000 * 3 + 10 {
            feed.pump();
            sound.process(&mut out, 1.0 / 48_000.0, &info());
            got.extend_from_slice(&out);
        }
        assert!(handle.is_playing() && !sound.finished());
        // Three times round: the frame after the last is the first, again.
        for lap in 0..3 {
            for k in [0usize, 1, 2, 47_998, 47_999] {
                let g = got[lap * 48_000 + k + 1];
                let f = want[k];
                assert!((g.left - f.left).abs() < 1e-5, "lap {lap}, frame {k}: {g:?} vs {f:?}");
            }
        }
        assert_eq!(handle.starved(), 0);
    }

    #[test]
    fn a_pumped_sound_follows_the_volume_smoothly_and_stops_without_a_click() {
        let (mut feed, mut sound, handle) = start(TONE.into(), 1.0, true).unwrap();
        let mut out = vec![Frame::ZERO; 4800];
        feed.pump();
        sound.process(&mut out, 1.0 / 48_000.0, &info());
        let loud = out.iter().map(|f| f.left.abs()).fold(0.0, f32::max);
        handle.set_volume(0.25);
        feed.pump();
        sound.process(&mut out, 1.0 / 48_000.0, &info());
        // A step of the volume is a slope: no jump between neighbours bigger
        // than the tone's own motion.
        let jump = out.windows(2).map(|w| (w[1].left - w[0].left).abs()).fold(0.0, f32::max);
        assert!(jump < 0.08, "{jump}");
        let quiet = out[2400..].iter().map(|f| f.left.abs()).fold(0.0, f32::max);
        assert!((quiet / loud - 0.25).abs() < 0.05, "{quiet} of {loud}");
        handle.stop();
        feed.pump();
        sound.process(&mut out, 1.0 / 48_000.0, &info());
        assert!(sound.finished() && !handle.is_playing());
        assert!(out[out.len() - 1].left.abs() < 1e-3);
    }

    #[test]
    fn a_pumped_sound_is_resampled_to_the_devices_rate() {
        let (mut feed, mut sound, handle) = start(TONE.into(), 1.0, false).unwrap();
        let mut out = vec![Frame::ZERO; 441];
        let mut blocks = 0;
        while !sound.finished() && blocks < 1000 {
            feed.pump();
            sound.process(&mut out, 1.0 / 44_100.0, &info());
            blocks += 1;
        }
        // One second of the sound is one second at 44.1 kHz too.
        assert!((99..=102).contains(&blocks), "{blocks} blocks of 10 ms");
        assert_eq!(handle.played(), 48_000);
    }

    #[test]
    fn a_pump_never_decodes_more_than_its_share_at_once() {
        let (feed, mut sound, _) = start(TONE.into(), 1.0, true).unwrap();
        // Primed with half a second at most: the rest comes on later frames.
        let mut out = vec![Frame::ZERO; 48_000];
        sound.process(&mut out, 1.0 / 48_000.0, &info());
        assert!(feed.shared.starved.load(Ordering::Relaxed) >= 23_000);
    }
}
