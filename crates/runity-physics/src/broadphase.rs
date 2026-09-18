//! Broad phase: which pairs are worth looking at closely.
//!
//! Testing every body against every other is quadratic, and most of those
//! tests answer "nowhere near each other". Sweep and prune sorts the bodies
//! along one axis and only compares those whose intervals overlap — which for
//! the scattered, mostly-resting scenes a game has is close to linear.

use crate::shape::Aabb;

/// One body as the broad phase sees it.
#[derive(Debug, Clone, Copy)]
pub struct Proxy {
    /// Index into the caller's body array.
    pub index: usize,
    pub aabb: Aabb,
    /// Static, sleeping, or otherwise not going anywhere this step.
    pub inert: bool,
}

/// Finds overlapping pairs. Keeps its buffers between calls.
#[derive(Debug, Default)]
pub struct BroadPhase {
    sorted: Vec<Proxy>,
    active: Vec<usize>,
}

impl BroadPhase {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append every overlapping pair to `out`, as index pairs with the lower
    /// index first.
    ///
    /// Pairs where neither body can move are skipped: two walls never need a
    /// contact.
    pub fn find_pairs(&mut self, proxies: &[Proxy], out: &mut Vec<(usize, usize)>) {
        out.clear();
        self.sorted.clear();
        self.sorted.extend_from_slice(proxies);
        // An infinite AABB (a half-space) sorts first and stays active
        // throughout, which is exactly right: the ground overlaps everything.
        self.sorted
            .sort_by(|a, b| a.aabb.min.x.total_cmp(&b.aabb.min.x));

        self.active.clear();
        for current in 0..self.sorted.len() {
            let proxy = self.sorted[current];
            // Drop everything that ended before this one starts.
            self.active
                .retain(|candidate| self.sorted[*candidate].aabb.max.x >= proxy.aabb.min.x);

            for &candidate in &self.active {
                let other = self.sorted[candidate];
                if proxy.inert && other.inert {
                    continue;
                }
                if !proxy.aabb.overlaps(&other.aabb) {
                    continue;
                }
                let (low, high) = if proxy.index < other.index {
                    (proxy.index, other.index)
                } else {
                    (other.index, proxy.index)
                };
                out.push((low, high));
            }
            self.active.push(current);
        }

        // A stable order makes the whole simulation reproducible, which matters
        // more here than the microseconds it costs.
        out.sort_unstable();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runity_math::Vec3;

    fn proxy(index: usize, x: f32, inert: bool) -> Proxy {
        Proxy {
            index,
            aabb: Aabb::from_center_half_extents(Vec3::new(x, 0.0, 0.0), Vec3::splat(0.5)),
            inert,
        }
    }

    #[test]
    fn overlapping_boxes_are_paired_and_distant_ones_are_not() {
        let mut broad = BroadPhase::new();
        let mut pairs = Vec::new();
        let proxies = [
            proxy(0, 0.0, false),
            proxy(1, 0.5, false),
            proxy(2, 10.0, false),
        ];
        broad.find_pairs(&proxies, &mut pairs);
        assert_eq!(pairs, vec![(0, 1)]);
    }

    #[test]
    fn two_bodies_that_cannot_move_are_never_paired() {
        let mut broad = BroadPhase::new();
        let mut pairs = Vec::new();
        broad.find_pairs(&[proxy(0, 0.0, true), proxy(1, 0.2, true)], &mut pairs);
        assert!(pairs.is_empty());

        // ...but a moving body against a static one is.
        broad.find_pairs(&[proxy(0, 0.0, true), proxy(1, 0.2, false)], &mut pairs);
        assert_eq!(pairs, vec![(0, 1)]);
    }

    #[test]
    fn an_infinite_proxy_pairs_with_everything_that_moves() {
        let mut broad = BroadPhase::new();
        let mut pairs = Vec::new();
        let ground = Proxy {
            index: 0,
            aabb: Aabb::infinite(),
            inert: true,
        };
        let proxies = [ground, proxy(1, 0.0, false), proxy(2, 50.0, false)];
        broad.find_pairs(&proxies, &mut pairs);
        assert_eq!(pairs, vec![(0, 1), (0, 2)]);
    }

    #[test]
    fn the_pair_order_does_not_depend_on_the_input_order() {
        let mut broad = BroadPhase::new();
        let mut forward = Vec::new();
        let mut backward = Vec::new();
        let a = proxy(0, 0.0, false);
        let b = proxy(1, 0.4, false);
        let c = proxy(2, 0.8, false);
        broad.find_pairs(&[a, b, c], &mut forward);
        broad.find_pairs(&[c, b, a], &mut backward);
        assert_eq!(forward, backward);
        assert_eq!(forward, vec![(0, 1), (0, 2), (1, 2)]);
    }

    #[test]
    fn a_long_row_of_bodies_stays_linear_ish() {
        // Not a timing test — just that a sparse row produces only neighbours.
        let proxies: Vec<Proxy> = (0..200).map(|i| proxy(i, i as f32 * 2.0, false)).collect();
        let mut broad = BroadPhase::new();
        let mut pairs = Vec::new();
        broad.find_pairs(&proxies, &mut pairs);
        assert!(pairs.is_empty(), "spaced two apart, nothing overlaps");
    }
}
