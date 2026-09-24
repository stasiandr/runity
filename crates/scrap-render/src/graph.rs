//! The frame as a graph: each pass the frame may run, what it reads and
//! what it writes, and from that which of them have to run at all.
//!
//! A frame's passes are declared in the order they run — the shadows, the
//! rays' structure, the probes, the prepass, the occlusion, the lit
//! scene, TAA, the lens, the upscale, post, the overlay — each with the
//! resources it reads and writes, by name ("depth", "normals",
//! "shadow map", "hdr", "screen"…). What reads a resource reads it from
//! the last pass before it that wrote it. [`FrameGraph::resolve`] then
//! walks back from what the frame is for — the screen, and the histories
//! kept for the next frame (TAA's, the Hi-Z pyramid, the probes',
//! ReSTIR's reservoirs) — and every pass nothing live reads is left out:
//! the prepass runs because something reads its depth, not because a list
//! of who might remembers it.
//!
//! What it does not do, yet: run the passes itself, reorder them, or give
//! passes' textures memory that another pass's reuses once it is done
//! (transient aliasing). The frame still records its passes in order; the
//! graph says which. [`Renderer::frame_graph`](crate::Renderer) hands the
//! last screen frame's graph to tools, and [`FrameGraph::dot`] draws it.

/// What a pass is to the GPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Render,
    Compute,
    Copy,
}

/// A pass of the frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Pass {
    pub name: &'static str,
    pub kind: Kind,
    pub reads: Vec<&'static str>,
    pub writes: Vec<&'static str>,
    /// What it writes is what the frame is for (the screen), or kept for
    /// the next frame (a history): it runs whether or not this frame reads
    /// it.
    pub output: bool,
    /// Whether it runs, once resolved.
    pub runs: bool,
}

/// The frame's passes, in order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FrameGraph {
    pub passes: Vec<Pass>,
}

impl FrameGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// A pass that runs if what it writes is read.
    pub fn pass(&mut self, name: &'static str, kind: Kind, reads: &[&'static str], writes: &[&'static str]) -> &mut Self {
        self.passes.push(Pass {
            name,
            kind,
            reads: reads.to_vec(),
            writes: writes.to_vec(),
            output: false,
            runs: false,
        });
        self
    }

    /// A pass that always runs: it writes the screen or a history.
    pub fn output(&mut self, name: &'static str, kind: Kind, reads: &[&'static str], writes: &[&'static str]) -> &mut Self {
        self.pass(name, kind, reads, writes);
        self.passes.last_mut().expect("just added").output = true;
        self
    }

    /// Work out which passes run: the outputs, and every pass whose writes
    /// a running pass after it reads.
    pub fn resolve(&mut self) -> &mut Self {
        for pass in &mut self.passes {
            pass.runs = pass.output;
        }
        // Back from the end: a running pass makes live the last writer
        // before it of each thing it reads.
        for i in (0..self.passes.len()).rev() {
            if !self.passes[i].runs {
                continue;
            }
            let reads = self.passes[i].reads.clone();
            for resource in reads {
                if let Some(writer) = (0..i).rev().find(|&j| self.passes[j].writes.contains(&resource)) {
                    self.passes[writer].runs = true;
                }
            }
        }
        self
    }

    /// Whether the pass of this name runs.
    pub fn runs(&self, name: &str) -> bool {
        self.passes.iter().any(|p| p.name == name && p.runs)
    }

    /// Resources a running pass reads that no pass before it writes and
    /// that are not the frame's inputs (`inputs`): an order that cannot
    /// work.
    pub fn problems(&self, inputs: &[&str]) -> Vec<String> {
        let mut out = Vec::new();
        for (i, pass) in self.passes.iter().enumerate() {
            if !pass.runs {
                continue;
            }
            for resource in &pass.reads {
                let written = self.passes[..i].iter().any(|p| p.runs && p.writes.contains(resource));
                if !written && !inputs.contains(resource) {
                    out.push(format!("{} reads {resource}, which nothing before it writes", pass.name));
                }
            }
        }
        out
    }

    /// The graph in Graphviz's dot: passes as boxes (grey when left out),
    /// resources as the edges between them.
    pub fn dot(&self) -> String {
        let mut out = String::from("digraph frame {\n  rankdir=LR;\n  node [shape=box, fontname=\"Helvetica\"];\n");
        for pass in &self.passes {
            let style = if pass.runs { "" } else { ", style=dashed, fontcolor=grey" };
            out.push_str(&format!("  \"{}\" [label=\"{}\"{style}];\n", pass.name, pass.name));
        }
        for (i, pass) in self.passes.iter().enumerate() {
            for resource in &pass.reads {
                if let Some(writer) = self.passes[..i].iter().rev().find(|p| p.writes.contains(resource)) {
                    out.push_str(&format!("  \"{}\" -> \"{}\" [label=\"{resource}\"];\n", writer.name, pass.name));
                }
            }
        }
        out.push_str("}\n");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pass_runs_only_when_what_it_writes_is_read_on_the_way_to_the_screen() {
        let mut graph = FrameGraph::new();
        graph
            .pass("prepass", Kind::Render, &[], &["depth", "normals"])
            .pass("ssao", Kind::Render, &["depth", "normals"], &["occlusion"])
            .pass("unused", Kind::Compute, &["depth"], &["nothing read"])
            .pass("scene", Kind::Render, &["occlusion"], &["hdr"])
            .output("post", Kind::Render, &["hdr"], &["screen"]);
        graph.resolve();
        assert!(graph.runs("prepass") && graph.runs("ssao") && graph.runs("scene") && graph.runs("post"));
        assert!(!graph.runs("unused"), "nothing reads what it writes");
        assert!(graph.problems(&[]).is_empty());
        // Without the occlusion read, SSAO and the prepass go too.
        let mut plain = FrameGraph::new();
        plain
            .pass("prepass", Kind::Render, &[], &["depth"])
            .pass("ssao", Kind::Render, &["depth"], &["occlusion"])
            .pass("scene", Kind::Render, &[], &["hdr"])
            .output("post", Kind::Render, &["hdr"], &["screen"]);
        plain.resolve();
        assert!(!plain.runs("prepass") && !plain.runs("ssao"));
        // A history runs though this frame does not read it.
        let mut kept = FrameGraph::new();
        kept.pass("prepass", Kind::Render, &[], &["depth"]).output("hi-z", Kind::Compute, &["depth"], &["hi-z"]);
        kept.resolve();
        assert!(kept.runs("prepass"));
        // An order that cannot work is named.
        let mut wrong = FrameGraph::new();
        wrong.output("post", Kind::Render, &["hdr"], &["screen"]).pass("scene", Kind::Render, &[], &["hdr"]);
        wrong.resolve();
        assert_eq!(wrong.problems(&[]).len(), 1);
        assert!(graph.dot().contains("\"prepass\" -> \"ssao\""));
    }
}
