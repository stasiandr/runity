//! Lean pipelines: the lit shaders built again with `LEAN` on
//! (render.wgsl), for frames that use none of the rarer things they can
//! do — and so drawn with a shader whose code for those things is gone.
//!
//! Why it pays: a GPU keeps as many pixels in flight as its registers
//! hold, and a shader is given the registers its largest path needs,
//! whether a frame ever takes that path or not. The standard shader's
//! weather, water, rays, probes and the rest, switched off by the frame's
//! uniform, still cost Dacha's lit pass three times what the same pixels
//! cost without them (4.2 against 1.2 ms at 1080p on an M5 Pro).
//!
//! Whether a frame may be drawn lean is read off the frame's own uniform
//! and draws ([`Renderer`](crate::Renderer)'s `lean_allowed`): every
//! function `LEAN` cuts short returns what it returns there when its
//! switches are off, so a lean frame is the same picture.
//!
//! Built on a thread of its own, only for the looks a frame draws, the
//! first time it draws them: until one is in, the standard pipeline draws
//! that look, and no frame waits for a compile (DNA, postulate 6). A
//! shader reloaded, a material's shader set or the samples changed make
//! what was built stale: it is dropped and built again as asked.
//! `SCRAP_LEAN=0` turns it off, to compare; `SCRAP_LEAN_WHY=1` prints,
//! each frame, which switch keeps it off the lean ones.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc;

use crate::material::RenderFace;

/// How the worker builds a pipeline.
pub(crate) type Build = Box<dyn FnOnce(&wgpu::Device) -> wgpu::RenderPipeline + Send>;

/// What the worker is given: the key, the shaders' generation, how.
struct Job<K> {
    key: K,
    generation: u64,
    build: Build,
}

/// A scene pipeline's particulars, as `scene_pipelines` builds it.
#[derive(Clone, Copy)]
pub(crate) struct Describe {
    pub(crate) skinned: bool,
    pub(crate) terrain: bool,
    pub(crate) water: bool,
    pub(crate) unlit: bool,
    pub(crate) prepassed: bool,
    pub(crate) on_top: bool,
    pub(crate) face: RenderFace,
    pub(crate) blend: Option<wgpu::BlendState>,
    pub(crate) samples: u32,
}

pub(crate) struct Lean<K: Copy + Eq + std::hash::Hash + Send + 'static> {
    /// This frame is drawn lean where a pipeline is in.
    pub(crate) on: bool,
    /// Off by `SCRAP_LEAN=0`.
    pub(crate) enabled: bool,
    pub(crate) ready: HashMap<K, wgpu::RenderPipeline>,
    pending: HashSet<K>,
    /// Counts what was asked; a job carries the count it was asked at.
    generation: u64,
    /// What was asked before this is stale, all of it ([`Lean::forget`]).
    floor: u64,
    /// What was asked for a key before its number here is stale
    /// ([`Lean::forget_where`]).
    stale_before: HashMap<K, u64>,
    /// How many threads build, when they are started.
    pub(crate) workers: usize,
    jobs: Option<mpsc::Sender<Job<K>>>,
    done: Option<mpsc::Receiver<(K, u64, wgpu::RenderPipeline)>>,
}

impl<K: Copy + Eq + std::hash::Hash + Send + 'static> Default for Lean<K> {
    fn default() -> Self {
        Self {
            on: false,
            enabled: std::env::var("SCRAP_LEAN").map_or(true, |v| v != "0"),
            ready: HashMap::new(),
            pending: HashSet::new(),
            generation: 0,
            floor: 0,
            stale_before: HashMap::new(),
            workers: 1,
            jobs: None,
            done: None,
        }
    }
}

impl<K: Copy + Eq + std::hash::Hash + Send + 'static> Lean<K> {
    /// A builder always on, of `workers` threads: the lit pipelines as
    /// they are (the renderer's `full`).
    pub(crate) fn always(workers: usize) -> Self {
        Self { enabled: true, workers, ..Default::default() }
    }

    /// What was built is stale: the shaders or the samples changed.
    pub(crate) fn forget(&mut self) {
        self.generation += 1;
        self.floor = self.generation;
        self.ready.clear();
        self.pending.clear();
        self.stale_before.clear();
    }

    /// What was built for these keys is stale (one material's shader set
    /// again): the rest is kept.
    pub(crate) fn forget_where(&mut self, stale: impl Fn(&K) -> bool) {
        self.generation += 1;
        let keys: Vec<K> = self.ready.keys().chain(self.pending.iter()).copied().filter(|k| stale(k)).collect();
        for key in keys {
            self.ready.remove(&key);
            self.pending.remove(&key);
            self.stale_before.insert(key, self.generation);
        }
    }

    /// Put in a pipeline built here, not on the worker.
    pub(crate) fn put(&mut self, key: K, pipeline: wgpu::RenderPipeline) {
        self.pending.remove(&key);
        self.ready.insert(key, pipeline);
    }

    /// How many asked for are not in yet.
    pub(crate) fn waiting(&self) -> usize {
        self.pending.len()
    }

    /// Whether this key was asked for already (and so is in, or coming).
    pub(crate) fn asked(&self, key: &K) -> bool {
        self.ready.contains_key(key) || self.pending.contains(key)
    }

    /// Ask for a pipeline: built on the worker, in by a later frame.
    pub(crate) fn ask(&mut self, device: &wgpu::Device, key: K, build: Build) {
        if self.asked(&key) {
            return;
        }
        if self.jobs.is_none() {
            let (jobs, inbox) = mpsc::channel::<Job<K>>();
            let (outbox, done) = mpsc::channel();
            // The workers take jobs off one queue, each as it is free.
            let inbox = std::sync::Arc::new(std::sync::Mutex::new(inbox));
            let mut started = 0;
            for _ in 0..self.workers.max(1) {
                let device = device.clone();
                let inbox = inbox.clone();
                let outbox = outbox.clone();
                let spawned = std::thread::Builder::new().name("scrap-pipelines".into()).spawn(move || loop {
                    let Ok(job) = inbox.lock().map_err(|_| ()).and_then(|q| q.recv().map_err(|_| ())) else {
                        break;
                    };
                    let pipeline = (job.build)(&device);
                    if outbox.send((job.key, job.generation, pipeline)).is_err() {
                        break;
                    }
                });
                started += spawned.is_ok() as usize;
            }
            if started == 0 {
                self.enabled = false;
                return;
            }
            self.jobs = Some(jobs);
            self.done = Some(done);
        }
        self.pending.insert(key);
        if let Some(jobs) = &self.jobs {
            let _ = jobs.send(Job { key, generation: self.generation, build });
        }
    }

    /// Take in what the worker has finished for the shaders as they are.
    pub(crate) fn collect(&mut self) {
        let Some(done) = &self.done else { return };
        while let Ok((key, generation, pipeline)) = done.try_recv() {
            if generation >= self.floor && generation >= self.stale_before.get(&key).copied().unwrap_or(0) && self.pending.contains(&key) {
                self.pending.remove(&key);
                self.ready.insert(key, pipeline);
            }
        }
    }
}

/// A pipeline of the lit scene: the standard one, or with `lean` its
/// fragment stage built with `LEAN` on.
pub(crate) fn scene_pipeline(device: &wgpu::Device, module: &wgpu::ShaderModule, layout: &wgpu::PipelineLayout, d: Describe, lean: bool) -> wgpu::RenderPipeline {
    let buffers = crate::render::vertex_buffers(d.skinned);
    let constants: &[(&str, f64)] = if lean { &[("LEAN", 1.0)] } else { &[] };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(match (lean, d.skinned) {
            (true, _) => "scrap::render (lean)",
            (false, true) => "scrap::skinned",
            (false, false) => "scrap::render",
        }),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some(if d.terrain {
                "vs_terrain"
            } else if d.skinned {
                "vs_skinned"
            } else {
                "vs"
            }),
            compilation_options: Default::default(),
            buffers: &buffers,
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(if d.water {
                "fs_water"
            } else if d.unlit {
                "fs_unlit"
            } else if d.prepassed {
                "fs_prepassed"
            } else {
                "fs"
            }),
            compilation_options: wgpu::PipelineCompilationOptions {
                constants,
                ..Default::default()
            },
            targets: &[Some(wgpu::ColorTargetState {
                format: crate::post::HDR_FORMAT,
                blend: d.blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            // Back faces are dropped unless a material asks for them, which
            // is why the importer cares about winding: a model wound inside
            // out disappears.
            cull_mode: match d.face {
                RenderFace::Front => Some(wgpu::Face::Back),
                RenderFace::Back => Some(wgpu::Face::Front),
                RenderFace::Both | RenderFace::BothAsFront => None,
            },
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: crate::render::DEPTH_FORMAT,
            // What is see-through does not hide what is drawn after it; it
            // is tested against the solid world only.
            depth_write_enabled: Some(d.blend.is_none()),
            // Equal passes: the prepass's own depth may be there already.
            depth_compare: Some(if d.on_top {
                wgpu::CompareFunction::Always
            } else if d.prepassed {
                wgpu::CompareFunction::Equal
            } else {
                wgpu::CompareFunction::LessEqual
            }),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: d.samples,
            ..Default::default()
        },
        multiview_mask: None,
        cache: None,
    })
}
