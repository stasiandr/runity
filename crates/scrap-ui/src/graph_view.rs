//! A graph drawn as boxes and arrows — what the Animator window is, and
//! what a dialogue, particle or shader graph editor would be: the boxes
//! placed from the graph itself, never dragged (DNA, «Графовые редакторы»),
//! the arrows in right angles round the boxes. The domain brings its nodes
//! and edges; this knows nothing of what they mean.

use std::collections::BTreeMap;

use crate::{Color, NodeId, Style, Ui};

/// A box's size on the canvas.
pub const BOX_W: f32 = 150.0;
pub const BOX_H: f32 = 36.0;

/// Where each node stands, as (column, row): columns by the steps from
/// `root` along `edges`, what cannot be reached from it in column one;
/// rows in each column ordered to sit near what leads to them.
pub fn layout(
    root: &str,
    nodes: &[&str],
    edges: &[(&str, &str)],
) -> BTreeMap<String, (usize, usize)> {
    let known = |n: &str| nodes.contains(&n);
    let mut depth: BTreeMap<&str, usize> = BTreeMap::new();
    if known(root) {
        depth.insert(root, 0);
        let mut frontier = vec![root];
        while !frontier.is_empty() {
            let mut next = Vec::new();
            for from in frontier {
                let d = depth[from];
                for (_, to) in edges.iter().filter(|(f, _)| *f == from) {
                    if known(to) && !depth.contains_key(to) {
                        depth.insert(to, d + 1);
                        next.push(*to);
                    }
                }
            }
            frontier = next;
        }
    }
    let mut columns: BTreeMap<usize, Vec<&str>> = BTreeMap::new();
    for name in nodes {
        let column = depth.get(name).copied().unwrap_or(1);
        columns.entry(column).or_default().push(name);
    }
    let mut placed: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for (column, names) in columns {
        let pull = |name: &str| -> f32 {
            let rows: Vec<f32> = edges
                .iter()
                .filter(|(_, to)| *to == name)
                .filter_map(|(from, _)| placed.get(*from))
                .filter(|(c, _)| *c < column)
                .map(|(_, r)| *r as f32)
                .collect();
            if rows.is_empty() {
                f32::MAX
            } else {
                rows.iter().sum::<f32>() / rows.len() as f32
            }
        };
        let mut order: Vec<(f32, &str)> = names.into_iter().map(|n| (pull(n), n)).collect();
        order.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(b.1)));
        for (row, (_, name)) in order.into_iter().enumerate() {
            placed.insert(name.to_string(), (column, row));
        }
    }
    placed
}

/// An arrow from the box at `from` to the box at `to` (their top-left
/// corners), in right angles round the `others` boxes where it can, with a
/// head where it meets the box; `back` moves it aside, so a pair both ways
/// is two arrows. Drawn in `layer`, the head in `heads`, each piece named
/// `name` and clickable: the nodes, for the caller to know a click on it.
#[allow(clippy::too_many_arguments)]
pub fn arrow(
    ui: &mut Ui,
    layer: NodeId,
    heads: NodeId,
    name: &str,
    from: (f32, f32),
    to: (f32, f32),
    back: bool,
    others: &[(f32, f32)],
    color: Color,
    t: f32,
) -> Vec<NodeId> {
    let mut out = Vec::new();
    let shift = if back { 8.0 } else { 0.0 };
    let (fx, fy) = (from.0 + BOX_W / 2.0 + shift, from.1 + BOX_H / 2.0 + shift);
    let (tx, ty) = (to.0 + BOX_W / 2.0 + shift, to.1 + BOX_H / 2.0 + shift);
    // Routes to try, in order; the first that crosses no other box
    // is drawn, or the first when every one does.
    type Route = (Vec<(f32, f32, f32, f32)>, (f32, f32));
    let mut routes: Vec<Route> = Vec::new();
    let h_seg = |y: f32, a: f32, b: f32| (a.min(b), y - t / 2.0, (b - a).abs() + t, t);
    let v_seg = |x: f32, a: f32, b: f32| (x - t / 2.0, a.min(b), t, (b - a).abs());
    if (fy - ty).abs() < BOX_H {
        // Side by side: straight across to the box's edge…
        let end = if tx > fx { to.0 } else { to.0 + BOX_W };
        routes.push((vec![h_seg(fy, fx, end)], (end, fy)));
        // …or under the row, round whatever is between.
        let below = from.1.max(to.1) + BOX_H + 22.0 + shift;
        routes.push((
            vec![
                v_seg(fx, from.1 + BOX_H, below),
                h_seg(below, fx, tx),
                v_seg(tx, below, to.1 + BOX_H),
            ],
            (tx, to.1 + BOX_H),
        ));
    } else {
        // Along, then down or up into the box…
        let end = if ty > fy { to.1 } else { to.1 + BOX_H };
        routes.push((vec![h_seg(fy, fx, tx), v_seg(tx, fy, end)], (tx, end)));
        // …or down or up first, then along into its side.
        if (fx - tx).abs() > BOX_W {
            let end = if tx > fx { to.0 } else { to.0 + BOX_W };
            routes.push((vec![v_seg(fx, fy, ty), h_seg(ty, fx, end)], (end, ty)));
        }
    }
    let crosses = |(x, y, w, h): (f32, f32, f32, f32)| {
        others
            .iter()
            .any(|&(bx, by)| x < bx + BOX_W && x + w > bx && y < by + BOX_H && y + h > by)
    };
    let pick = routes
        .iter()
        .position(|(segs, _)| !segs.iter().any(|s| crosses(*s)))
        .unwrap_or(0);
    let (segments, head) = routes.swap_remove(pick);
    for (x, y, w, h) in segments {
        // A wider strip to click than to see.
        let hit = ui.add(
            layer,
            Style::row()
                .absolute(x - 3.0, y - 3.0)
                .size(w + 6.0, h + 6.0)
                .padding(3.0)
                .clickable(),
        );
        ui.set_name(hit, name.to_string());
        ui.add(hit, Style::default().size(w, h).background(color));
        out.push(hit);
    }
    let head_node = ui.add(
        heads,
        Style::default()
            .absolute(head.0 - 5.0, head.1 - 5.0)
            .size(10.0, 10.0)
            .radius(5.0)
            .background(color)
            .clickable(),
    );
    ui.set_name(head_node, format!("{name} head"));
    out.push(head_node);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nodes_stand_in_columns_by_their_steps_from_the_root() {
        let placed = layout(
            "idle",
            &["idle", "walk", "run", "swim"],
            &[("idle", "walk"), ("walk", "run")],
        );
        assert_eq!(placed["idle"], (0, 0));
        assert_eq!(placed["walk"], (1, 0));
        assert_eq!(placed["run"], (2, 0));
        assert_eq!(placed["swim"].0, 1, "unreached: column one");
    }
}
