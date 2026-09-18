//! Skeletons that bend a mesh, and clips that move the skeleton.

use runity_math::{vec3, Mat4, Quat, Vec3, Vec4};
use runity_render::skin::{
    Animation, Channel, Influence, Interpolation, Joint, Skeleton, SkinnedMesh, Track, Transform,
};
use runity_render::{Color, Mesh, Vertex};

/// A two-joint arm: a shoulder at the origin and an elbow a metre along +X.
fn arm() -> Skeleton {
    Skeleton {
        joints: vec![
            Joint {
                parent: None,
                rest: Transform::default(),
                inverse_bind: Mat4::IDENTITY,
            },
            Joint {
                parent: Some(0),
                rest: Transform {
                    position: vec3(1.0, 0.0, 0.0),
                    ..Transform::default()
                },
                // The elbow sits one metre along X in the bind pose, so its
                // inverse bind takes a vertex back by that much.
                inverse_bind: Mat4::from_translation(vec3(-1.0, 0.0, 0.0)),
            },
        ],
    }
}

fn vertex(position: Vec3) -> Vertex {
    Vertex {
        position,
        normal: Vec3::Y,
        tangent: Vec4::new(1.0, 0.0, 0.0, 1.0),
        uv_density: 1.0,
        uv: runity_math::Vec2::ZERO,
        color: Color::WHITE,
    }
}

/// Two vertices: one bound to the shoulder, one to the elbow.
fn arm_mesh() -> SkinnedMesh {
    let rest = Mesh::new(
        vec![vertex(vec3(0.0, 0.0, 0.0)), vertex(vec3(2.0, 0.0, 0.0))],
        vec![0, 1, 0],
    );
    SkinnedMesh {
        rest,
        influences: vec![
            Influence {
                joints: [0, 0, 0, 0],
                weights: [1.0, 0.0, 0.0, 0.0],
            },
            Influence {
                joints: [1, 0, 0, 0],
                weights: [1.0, 0.0, 0.0, 0.0],
            },
        ],
    }
}

#[test]
fn the_rest_pose_leaves_the_mesh_where_it_was_modelled() {
    // The first thing that goes wrong in a skinning implementation: the bind
    // pose comes out distorted, and every animation is wrong from there.
    let skeleton = arm();
    let skinned = arm_mesh();
    let mut matrices = Vec::new();
    skeleton.skinning_matrices(&skeleton.rest_pose(), &mut matrices);

    let mut out = Mesh::default();
    skinned.apply(&matrices, &mut out);
    for (before, after) in skinned.rest.vertices.iter().zip(&out.vertices) {
        assert!(
            (before.position - after.position).length() < 1e-5,
            "{:?} became {:?}",
            before.position,
            after.position
        );
    }
    assert_eq!(out.indices, skinned.rest.indices);
}

#[test]
fn moving_a_joint_moves_the_vertices_bound_to_it() {
    let skeleton = arm();
    let skinned = arm_mesh();
    let mut pose = skeleton.rest_pose();
    pose.locals[1].position = vec3(2.0, 0.0, 0.0); // the elbow slides out

    let mut matrices = Vec::new();
    skeleton.skinning_matrices(&pose, &mut matrices);
    let mut out = Mesh::default();
    skinned.apply(&matrices, &mut out);

    assert!(
        (out.vertices[0].position - vec3(0.0, 0.0, 0.0)).length() < 1e-5,
        "the shoulder stays"
    );
    assert!(
        (out.vertices[1].position - vec3(3.0, 0.0, 0.0)).length() < 1e-5,
        "the hand follows the elbow: {:?}",
        out.vertices[1].position
    );
}

#[test]
fn a_child_joint_inherits_its_parent() {
    let skeleton = arm();
    let mut pose = skeleton.rest_pose();
    // A quarter turn at the shoulder takes the elbow from +X to -Z.
    pose.locals[0].rotation = Quat::from_axis_angle(Vec3::Y, core::f32::consts::FRAC_PI_2);

    let mut matrices = Vec::new();
    skeleton.world_matrices(&pose, &mut matrices);
    let elbow = matrices[1].transform_point(Vec3::ZERO);
    assert!(
        elbow.x.abs() < 1e-5 && (elbow.z + 1.0).abs() < 1e-5,
        "{elbow:?}"
    );
}

#[test]
fn weights_spread_a_vertex_between_joints() {
    let skeleton = arm();
    let mut skinned = arm_mesh();
    // Put the hand halfway between the two joints' influence.
    skinned.influences[1] = Influence {
        joints: [0, 1, 0, 0],
        weights: [0.5, 0.5, 0.0, 0.0],
    };

    let mut pose = skeleton.rest_pose();
    pose.locals[1].position = vec3(3.0, 0.0, 0.0); // elbow moves two metres out

    let mut matrices = Vec::new();
    skeleton.skinning_matrices(&pose, &mut matrices);
    let mut out = Mesh::default();
    skinned.apply(&matrices, &mut out);

    // Half of the two-metre shift, so one metre.
    assert!(
        (out.vertices[1].position.x - 3.0).abs() < 1e-4,
        "{:?}",
        out.vertices[1].position
    );
}

#[test]
fn weights_that_do_not_sum_to_one_are_normalized() {
    // Exporters round weights, and a vertex whose weights sum to 0.98 shrinks
    // toward the origin — which looks like a modelling mistake and is not.
    let influence = Influence {
        joints: [0, 1, 0, 0],
        weights: [0.49, 0.49, 0.0, 0.0],
    };
    let normalized = influence.normalized();
    let total: f32 = normalized.weights.iter().sum();
    assert!((total - 1.0).abs() < 1e-6);

    // And weights that are all zero fall back to rigid rather than collapsing.
    let broken = Influence {
        joints: [2, 0, 0, 0],
        weights: [0.0; 4],
    }
    .normalized();
    assert_eq!(broken.weights[0], 1.0);
}

#[test]
fn normals_are_rotated_and_renormalized() {
    let skeleton = arm();
    let skinned = arm_mesh();
    let mut pose = skeleton.rest_pose();
    pose.locals[0].rotation = Quat::from_axis_angle(Vec3::Z, core::f32::consts::FRAC_PI_2);

    let mut matrices = Vec::new();
    skeleton.skinning_matrices(&pose, &mut matrices);
    let mut out = Mesh::default();
    skinned.apply(&matrices, &mut out);

    for vertex in &out.vertices {
        assert!(
            (vertex.normal.length() - 1.0).abs() < 1e-4,
            "{:?}",
            vertex.normal
        );
    }
    // +Y turned a quarter about +Z is -X.
    assert!(
        (out.vertices[0].normal.x + 1.0).abs() < 1e-4,
        "{:?}",
        out.vertices[0].normal
    );
}

#[test]
fn a_sorted_skeleton_needs_only_one_pass() {
    let skeleton = arm();
    assert!(skeleton.is_sorted());

    let unsorted = Skeleton {
        joints: vec![
            Joint {
                parent: Some(1),
                rest: Transform::default(),
                inverse_bind: Mat4::IDENTITY,
            },
            Joint {
                parent: None,
                rest: Transform::default(),
                inverse_bind: Mat4::IDENTITY,
            },
        ],
    };
    assert!(
        !unsorted.is_sorted(),
        "a child before its parent breaks the single pass"
    );
}

// ------------------------------------------------------------- animation

/// A clip that slides the elbow out over one second.
fn slide() -> Animation {
    let mut animation = Animation {
        name: "slide".into(),
        channels: vec![Channel {
            joint: 1,
            track: Track::Translation(vec![(0.0, vec3(1.0, 0.0, 0.0)), (1.0, vec3(3.0, 0.0, 0.0))]),
            interpolation: Interpolation::Linear,
        }],
        duration: 0.0,
    };
    animation.recompute_duration();
    animation
}

#[test]
fn a_clip_interpolates_between_its_keyframes() {
    let skeleton = arm();
    let clip = slide();
    assert_eq!(clip.duration, 1.0);

    let start = clip.pose_at(&skeleton, 0.0);
    assert!((start.locals[1].position.x - 1.0).abs() < 1e-6);

    let middle = clip.pose_at(&skeleton, 0.5);
    assert!(
        (middle.locals[1].position.x - 2.0).abs() < 1e-5,
        "{:?}",
        middle.locals[1].position
    );

    let end = clip.pose_at(&skeleton, 1.0);
    assert!((end.locals[1].position.x - 3.0).abs() < 1e-6);
}

#[test]
fn sampling_outside_the_clip_holds_the_ends() {
    let skeleton = arm();
    let clip = slide();
    assert!((clip.pose_at(&skeleton, -5.0).locals[1].position.x - 1.0).abs() < 1e-6);
    assert!((clip.pose_at(&skeleton, 50.0).locals[1].position.x - 3.0).abs() < 1e-6);
}

#[test]
fn looping_wraps_the_time_including_backwards() {
    let clip = slide();
    assert_eq!(clip.wrap(0.25), 0.25);
    assert!((clip.wrap(3.25) - 0.25).abs() < 1e-6);
    assert!(
        (clip.wrap(-0.25) - 0.75).abs() < 1e-6,
        "a clip played backwards still wraps"
    );

    let still = Animation::default();
    assert_eq!(
        still.wrap(4.0),
        0.0,
        "a clip with no length does not divide by zero"
    );
}

#[test]
fn step_interpolation_holds_each_key() {
    let skeleton = arm();
    let mut clip = slide();
    clip.channels[0].interpolation = Interpolation::Step;

    assert!((clip.pose_at(&skeleton, 0.9).locals[1].position.x - 1.0).abs() < 1e-6);
    assert!((clip.pose_at(&skeleton, 1.0).locals[1].position.x - 3.0).abs() < 1e-6);
}

#[test]
fn rotation_takes_the_short_way_round() {
    // Interpolating quaternions componentwise goes the long way round half
    // the time, which looks like a joint spinning through the body.
    let skeleton = arm();
    let mut clip = Animation {
        name: "turn".into(),
        channels: vec![Channel {
            joint: 0,
            track: Track::Rotation(vec![
                (0.0, Quat::from_axis_angle(Vec3::Y, -0.1)),
                (1.0, Quat::from_axis_angle(Vec3::Y, 0.1)),
            ]),
            interpolation: Interpolation::Linear,
        }],
        duration: 0.0,
    };
    clip.recompute_duration();

    let middle = clip.pose_at(&skeleton, 0.5).locals[0].rotation;
    let forward = middle.rotate(Vec3::X);
    assert!(
        (forward - Vec3::X).length() < 0.01,
        "halfway should be near the identity: {forward:?}"
    );
}

#[test]
fn a_clip_leaves_alone_what_it_does_not_mention() {
    // What makes an arm-only clip usable on top of a walk.
    let skeleton = arm();
    let mut pose = skeleton.rest_pose();
    pose.locals[0].position = vec3(0.0, 5.0, 0.0);
    slide().sample(0.5, &mut pose);

    assert_eq!(
        pose.locals[0].position,
        vec3(0.0, 5.0, 0.0),
        "the root was not in the clip"
    );
    assert!((pose.locals[1].position.x - 2.0).abs() < 1e-5);
}

#[test]
fn poses_blend_joint_by_joint() {
    let skeleton = arm();
    let standing = skeleton.rest_pose();
    let reaching = slide().pose_at(&skeleton, 1.0);

    let halfway = standing.blend(&reaching, 0.5);
    assert!((halfway.locals[1].position.x - 2.0).abs() < 1e-5);

    assert_eq!(
        standing.blend(&reaching, 0.0).locals[1].position,
        standing.locals[1].position
    );
    assert_eq!(
        standing.blend(&reaching, 1.0).locals[1].position,
        reaching.locals[1].position
    );
    // Weights outside the range are clamped rather than extrapolated.
    assert_eq!(
        standing.blend(&reaching, 5.0).locals[1].position,
        reaching.locals[1].position
    );
}

#[test]
fn a_mask_blends_only_the_joints_it_names() {
    // A villager who waves while walking: the arm comes from one clip and
    // everything else from the other.
    let skeleton = arm();
    let walking = skeleton.rest_pose();
    let waving = slide().pose_at(&skeleton, 1.0);

    let mixed = walking.blend_masked(&waving, 1.0, &[false, true]);
    assert_eq!(
        mixed.locals[0].position, walking.locals[0].position,
        "the root keeps walking"
    );
    assert_eq!(
        mixed.locals[1].position, waving.locals[1].position,
        "the arm waves"
    );
}

#[test]
fn sampling_a_long_clip_is_not_a_scan() {
    // Hundreds of keys sampled per character per frame; the search has to be
    // a binary one, and the values have to come out right either way.
    let keys: Vec<(f32, Vec3)> = (0..500)
        .map(|index| (index as f32 * 0.01, vec3(index as f32, 0.0, 0.0)))
        .collect();
    let mut clip = Animation {
        name: "long".into(),
        channels: vec![Channel {
            joint: 1,
            track: Track::Translation(keys),
            interpolation: Interpolation::Linear,
        }],
        duration: 0.0,
    };
    clip.recompute_duration();
    assert!((clip.duration - 4.99).abs() < 1e-4);

    let skeleton = arm();
    for (time, expected) in [(0.0, 0.0), (1.0, 100.0), (2.505, 250.5), (4.99, 499.0)] {
        let pose = clip.pose_at(&skeleton, time);
        assert!(
            (pose.locals[1].position.x - expected).abs() < 0.01,
            "at {time} expected {expected}, got {}",
            pose.locals[1].position.x
        );
    }
}

#[test]
fn an_empty_or_broken_clip_is_harmless() {
    let skeleton = arm();
    let empty = Animation::default();
    let pose = empty.pose_at(&skeleton, 1.0);
    assert_eq!(pose.locals.len(), skeleton.len());

    // A channel naming a joint that does not exist.
    let mut stray = Animation {
        name: "stray".into(),
        channels: vec![Channel {
            joint: 99,
            track: Track::Translation(vec![(0.0, Vec3::ZERO)]),
            interpolation: Interpolation::Linear,
        }],
        duration: 0.0,
    };
    stray.recompute_duration();
    let pose = stray.pose_at(&skeleton, 0.5);
    assert_eq!(pose.locals.len(), 2);

    // And a mesh whose influences name a joint that does not exist.
    let mut skinned = arm_mesh();
    skinned.influences[0] = Influence {
        joints: [42, 0, 0, 0],
        weights: [1.0, 0.0, 0.0, 0.0],
    };
    let mut matrices = Vec::new();
    skeleton.skinning_matrices(&skeleton.rest_pose(), &mut matrices);
    let mut out = Mesh::default();
    skinned.apply(&matrices, &mut out);
    assert_eq!(out.vertices.len(), 2);
}

#[test]
fn skinning_reuses_the_output_buffer() {
    // Allocating a mesh per character per frame is the difference between a
    // crowd and a slideshow.
    let skeleton = arm();
    let skinned = arm_mesh();
    let mut matrices = Vec::new();
    skeleton.skinning_matrices(&skeleton.rest_pose(), &mut matrices);

    let mut out = Mesh::default();
    skinned.apply(&matrices, &mut out);
    let capacity = out.vertices.capacity();
    for _ in 0..50 {
        skinned.apply(&matrices, &mut out);
    }
    assert_eq!(
        out.vertices.capacity(),
        capacity,
        "the buffer should be reused, not regrown"
    );
    assert_eq!(out.vertices.len(), 2);
}

/// glTF does not require a skin's joints to be listed parent-first, and vertex
/// indices point at that array, so the joints cannot simply be reordered. The
/// matrices must come out the same either way.
#[test]
fn an_unsorted_skeleton_gives_the_same_matrices() {
    let sorted = arm();
    assert!(sorted.is_sorted());

    // The same two joints, with the child listed first.
    let reversed = Skeleton {
        joints: vec![
            Joint {
                parent: Some(1),
                rest: sorted.joints[1].rest,
                inverse_bind: sorted.joints[1].inverse_bind,
            },
            Joint {
                parent: None,
                rest: sorted.joints[0].rest,
                inverse_bind: sorted.joints[0].inverse_bind,
            },
        ],
    };
    assert!(!reversed.is_sorted());

    let mut pose = sorted.rest_pose();
    pose.locals[0].rotation = Quat::from_axis_angle(Vec3::Z, std::f32::consts::FRAC_PI_2);
    let mut flipped = reversed.rest_pose();
    flipped.locals[1].rotation = pose.locals[0].rotation;

    let mut expected = Vec::new();
    sorted.world_matrices(&pose, &mut expected);
    let mut actual = Vec::new();
    reversed.world_matrices(&flipped, &mut actual);

    // Index 0 of one skeleton is index 1 of the other.
    for (a, b) in [(0, 1), (1, 0)] {
        let left = expected[a] * Vec4::new(0.0, 0.0, 0.0, 1.0);
        let right = actual[b] * Vec4::new(0.0, 0.0, 0.0, 1.0);
        let apart = vec3(left.x - right.x, left.y - right.y, left.z - right.z);
        assert!(
            apart.length() < 1e-5,
            "joint {a} differs: {left:?} vs {right:?}"
        );
    }
}

/// A parent cycle is not describable by a well-formed file, but a malformed one
/// can say anything; resolving it must terminate rather than hang the loader.
#[test]
fn a_cycle_between_joints_terminates() {
    let tangled = Skeleton {
        joints: vec![
            Joint {
                parent: Some(1),
                rest: Transform::default(),
                inverse_bind: Mat4::IDENTITY,
            },
            Joint {
                parent: Some(0),
                rest: Transform::default(),
                inverse_bind: Mat4::IDENTITY,
            },
        ],
    };
    let mut matrices = Vec::new();
    tangled.world_matrices(&tangled.rest_pose(), &mut matrices);
    assert_eq!(matrices.len(), 2);
}
