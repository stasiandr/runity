//! How long each pass takes on the GPU: the GPU profiler.
//!
//! Every pass of a frame asks for a pair of timestamps by its name —
//! `timestamp_writes: crate::gpu_timer::render("ssao")` — written by the GPU
//! as the pass begins and ends; a pass is timed from the end of the one
//! before (or its own beginning, if later) to its end, since on a tiling GPU
//! a render pass's beginning comes with the frame's. Only at pass boundaries, which is what
//! Apple's GPUs give (wgpu's `TIMESTAMP_QUERY`, not the in-encoder kind).
//! At the frame's end they are resolved and copied out; a frame or two
//! later, when the copy has come back, each name's time is folded into a
//! running average — the frame never waits on the GPU for it. Passes of one
//! name in a frame (the shadow cascades, the lamps' faces) add up.
//!
//! Off unless asked (`Renderer::profile_gpu` (scrap-render), or
//! `SCRAP_GPU_TIMES=1`): the timestamps cost little, but not nothing. On a
//! device without timestamps it asks for nothing and reports nothing.
//!
//! The pass that wants a pair finds the frame's timer through a
//! thread-local, set for the length of the frame: the passes are spread
//! over a dozen modules, and threading a timer through each of them would
//! be a parameter on every function that draws.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Timestamps a frame can take: begin and end for so many passes.
const CAPACITY: u32 = 256;
/// Frames over which a pass's time is averaged: a plain mean until then,
/// a running one after.
const WINDOW: f32 = 10.0;

struct Frame {
    set: &'static wgpu::QuerySet,
    labels: Vec<&'static str>,
}

thread_local! {
    static CURRENT: RefCell<Option<Frame>> = const { RefCell::new(None) };
}

fn next(label: &'static str) -> Option<(&'static wgpu::QuerySet, u32)> {
    CURRENT.with(|current| {
        let mut current = current.borrow_mut();
        let frame = current.as_mut()?;
        let at = frame.labels.len() as u32 * 2;
        if at + 2 > CAPACITY {
            return None;
        }
        frame.labels.push(label);
        Some((frame.set, at))
    })
}

/// A render pass's pair of timestamps, when the frame is being timed.
pub fn render(label: &'static str) -> Option<wgpu::RenderPassTimestampWrites<'static>> {
    let (set, at) = next(label)?;
    Some(wgpu::RenderPassTimestampWrites {
        query_set: set,
        beginning_of_pass_write_index: Some(at),
        end_of_pass_write_index: Some(at + 1),
    })
}

/// A compute pass's pair of timestamps, when the frame is being timed.
pub fn compute(label: &'static str) -> Option<wgpu::ComputePassTimestampWrites<'static>> {
    let (set, at) = next(label)?;
    Some(wgpu::ComputePassTimestampWrites {
        query_set: set,
        beginning_of_pass_write_index: Some(at),
        end_of_pass_write_index: Some(at + 1),
    })
}

/// A renderer's timer: its timestamps, where they are read back, and the
/// averages so far.
pub struct GpuTimer {
    /// Leaked for as long as the timer lives, so a pass's descriptor can
    /// borrow it through the thread-local; given back when the timer is
    /// dropped (it holds its device, and every buffer on it, alive).
    set: &'static wgpu::QuerySet,
    resolve: wgpu::Buffer,
    read: wgpu::Buffer,
    /// Nanoseconds a tick.
    period: f32,
    /// The labels of the frame whose timestamps are in `read`, while its
    /// copy is on its way back.
    waiting: Option<Vec<&'static str>>,
    mapped: Arc<AtomicBool>,
    /// Asked for by the copy at the frame's end, before it is submitted.
    to_map: bool,
    /// Each name's average, milliseconds, and how many frames it is of, in
    /// the order first seen.
    times: Vec<(&'static str, f32, f32)>,
    /// Frames read back so far: the first is left out, its pipelines
    /// still being built.
    frames: u32,
    /// The GPU's time for the last frame read back, all its passes.
    last_frame: Option<f32>,
}

impl GpuTimer {
    /// A timer, on a device that has timestamps.
    pub fn new(gpu: &crate::gpu::Gpu) -> Option<Self> {
        if !gpu.device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let set = gpu.device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("scrap::gpu times"),
            ty: wgpu::QueryType::Timestamp,
            count: CAPACITY,
        });
        let size = CAPACITY as u64 * 8;
        let resolve = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu times resolved"),
            size,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu times read"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Some(Self {
            set: Box::leak(Box::new(set)),
            resolve,
            read,
            period: gpu.queue.get_timestamp_period(),
            waiting: None,
            mapped: Arc::new(AtomicBool::new(false)),
            to_map: false,
            times: Vec::new(),
            frames: 0,
            last_frame: None,
        })
    }

    /// Start timing a frame: fold in what has come back, and let the
    /// frame's passes ask for timestamps.
    pub fn begin(&mut self, gpu: &crate::gpu::Gpu) {
        if self.waiting.is_some() {
            let _ = gpu.device.poll(wgpu::PollType::Poll);
        }
        if self.mapped.swap(false, Ordering::AcqRel) {
            if let Some(labels) = self.waiting.take() {
                let ticks: Vec<u64> = match self.read.slice(..).get_mapped_range() {
                    Ok(view) => view.chunks_exact(8).map(|b| u64::from_le_bytes(b.try_into().expect("eight bytes"))).collect(),
                    Err(_) => Vec::new(),
                };
                self.read.unmap();
                if ticks.len() >= labels.len() * 2 {
                    self.fold(&labels, &ticks);
                }
            }
        }
        CURRENT.with(|current| {
            *current.borrow_mut() = Some(Frame {
                set: self.set,
                labels: Vec::new(),
            })
        });
    }

    fn fold(&mut self, labels: &[&'static str], ticks: &[u64]) {
        self.frames += 1;
        if self.frames == 1 {
            return;
        }
        let mut frame: Vec<(&'static str, f32)> = Vec::new();
        // A render pass's own beginning is not to be trusted on a tiling
        // GPU: its vertex work starts long before the pass before it has
        // finished its pixels, so every render pass seems to start with the
        // frame. The ends come in order. So a pass is timed from the later
        // of its beginning and the end of the pass before — what it held
        // the GPU for — and a pair whose end was never written is left out.
        let mut last_end: Option<u64> = None;
        for (i, label) in labels.iter().enumerate() {
            let (begin, end) = (ticks[i * 2], ticks[i * 2 + 1]);
            if end == 0 || end < begin {
                continue;
            }
            let from = last_end.map_or(begin, |e| e.max(begin)).min(end);
            last_end = Some(end);
            let ms = (end - from) as f32 * self.period / 1.0e6;
            match frame.iter_mut().find(|(l, _)| l == label) {
                Some((_, t)) => *t += ms,
                None => frame.push((label, ms)),
            }
        }
        self.last_frame = Some(frame.iter().map(|(_, ms)| ms).sum());
        for (label, ms) in frame {
            match self.times.iter_mut().find(|(l, _, _)| *l == label) {
                Some((_, t, n)) => {
                    *n = (*n + 1.0).min(WINDOW);
                    *t += (ms - *t) / *n;
                }
                None => self.times.push((label, ms, 1.0)),
            }
        }
    }

    /// End the frame's timing: its timestamps resolved into `encoder`, and
    /// copied out when the last copy has come back.
    pub fn end(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let frame = CURRENT.with(|current| current.borrow_mut().take());
        let Some(frame) = frame else { return };
        if frame.labels.is_empty() || self.waiting.is_some() {
            return;
        }
        let count = frame.labels.len() as u32 * 2;
        encoder.resolve_query_set(self.set, 0..count, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.read, 0, count as u64 * 8);
        self.waiting = Some(frame.labels);
        self.to_map = true;
    }

    /// After the frame is submitted: ask for its copy back.
    pub fn submitted(&mut self) {
        if !std::mem::take(&mut self.to_map) {
            return;
        }
        let mapped = self.mapped.clone();
        self.read.slice(..).map_async(wgpu::MapMode::Read, move |done| {
            if done.is_ok() {
                mapped.store(true, Ordering::Release);
            }
        });
    }

    /// The GPU's time for the last frame read back, milliseconds.
    pub fn frame_ms(&self) -> Option<f32> {
        self.last_frame
    }

    /// Each pass's average time, milliseconds.
    pub fn times(&self) -> Vec<(String, f32)> {
        self.times.iter().map(|(l, t, _)| (l.to_string(), *t)).collect()
    }
}

impl Drop for GpuTimer {
    fn drop(&mut self) {
        // No pass can still be borrowing the set: a frame's passes are
        // recorded between `begin` and `end` on this thread, and whatever
        // frame was left open is closed here first.
        CURRENT.with(|current| {
            let mut current = current.borrow_mut();
            if current.as_ref().is_some_and(|f| std::ptr::eq(f.set, self.set)) {
                *current = None;
            }
        });
        // SAFETY: `set` came from `Box::leak` in `new`, and nothing else
        // refers to it now.
        drop(unsafe { Box::from_raw(self.set as *const wgpu::QuerySet as *mut wgpu::QuerySet) });
    }
}
