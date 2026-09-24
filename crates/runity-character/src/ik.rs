//! Inverse kinematics: where the joints of a limb go for its end to reach
//! a point.
//!
//! * [`two_bone`] — a thigh and a shin, an upper arm and a forearm: solved
//!   exactly by the law of cosines, the middle joint bending toward a
//!   `pole` (a knee forward, an elbow back).
//! * [`fabrik`] — a chain of any length (a tail, a tentacle, a spider's
//!   leg of three): Forward And Backward Reaching IK (Aristidou, Lasenby
//!   2011), moving each joint along the chain toward the target and back to
//!   the root until the end is there.

use glam::Vec3;

/// The middle joint of a two-bone limb from `root`, bones `upper` and
/// `lower` long, its end reaching for `target` (or as near as it can),
/// bent toward `pole`. Returns the middle joint and where the end is.
pub fn two_bone(root: Vec3, upper: f32, lower: f32, target: Vec3, pole: Vec3) -> (Vec3, Vec3) {
    let to = target - root;
    let reach = (upper + lower) * 0.9999;
    let d = to.length().clamp((upper - lower).abs() + 1e-4, reach);
    let dir = to.normalize_or(Vec3::NEG_Y);
    let end = root + dir * d;
    // The law of cosines: the angle at the root.
    let cos = ((upper * upper + d * d - lower * lower) / (2.0 * upper * d)).clamp(-1.0, 1.0);
    let along = upper * cos;
    let out = upper * (1.0 - cos * cos).max(0.0).sqrt();
    // Bent toward the pole, square to the limb.
    let toward = pole - root;
    let bend = (toward - dir * toward.dot(dir)).normalize_or(dir.any_orthonormal_vector());
    (root + dir * along + bend * out, end)
}

/// A chain through `joints` (the first its root, held), its bones kept
/// their lengths, its end brought to `target` as near as it reaches.
pub fn fabrik(joints: &mut [Vec3], target: Vec3, rounds: usize) {
    let n = joints.len();
    if n < 2 {
        return;
    }
    let lengths: Vec<f32> = joints.windows(2).map(|w| w[0].distance(w[1])).collect();
    let root = joints[0];
    let total: f32 = lengths.iter().sum();
    if root.distance(target) >= total {
        // Out of reach: straight toward it.
        let dir = (target - root).normalize_or(Vec3::X);
        for i in 1..n {
            joints[i] = joints[i - 1] + dir * lengths[i - 1];
        }
        return;
    }
    for _ in 0..rounds {
        // Backward: the end to the target, each joint pulled after it.
        joints[n - 1] = target;
        for i in (0..n - 1).rev() {
            let dir = (joints[i] - joints[i + 1]).normalize_or(Vec3::Y);
            joints[i] = joints[i + 1] + dir * lengths[i];
        }
        // Forward: the root back where it is held.
        joints[0] = root;
        for i in 1..n {
            let dir = (joints[i] - joints[i - 1]).normalize_or(Vec3::Y);
            joints[i] = joints[i - 1] + dir * lengths[i - 1];
        }
        if joints[n - 1].distance(target) < 1e-4 {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_bones_reach_what_they_can_bent_toward_the_pole() {
        let root = Vec3::new(0.0, 1.0, 0.0);
        let target = Vec3::new(0.0, 0.3, 0.3);
        let (knee, foot) = two_bone(root, 0.45, 0.45, target, Vec3::new(0.0, 0.6, 2.0));
        assert!(foot.distance(target) < 1e-4);
        assert!((knee.distance(root) - 0.45).abs() < 1e-4 && (knee.distance(foot) - 0.45).abs() < 1e-4);
        assert!(knee.z > 0.2, "the knee forward: {knee}");
        // Too far: stretched straight toward it.
        let (_, end) = two_bone(root, 0.45, 0.45, Vec3::new(0.0, -2.0, 0.0), Vec3::Z);
        assert!((end.distance(root) - 0.9).abs() < 1e-3);
    }

    #[test]
    fn a_chain_reaches_round_and_keeps_its_lengths() {
        let mut chain: Vec<Vec3> = (0..5).map(|i| Vec3::new(i as f32 * 0.3, 0.0, 0.0)).collect();
        let target = Vec3::new(0.3, 0.7, 0.2);
        fabrik(&mut chain, target, 30);
        assert!(chain[4].distance(target) < 1e-3, "{}", chain[4]);
        assert_eq!(chain[0], Vec3::ZERO);
        for w in chain.windows(2) {
            assert!((w[0].distance(w[1]) - 0.3).abs() < 1e-3);
        }
    }
}
