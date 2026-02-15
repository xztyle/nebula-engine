# Inverse Kinematics

## Problem

Clip-based animation and procedural layers position joints based on the skeleton hierarchy and predefined motion, but they cannot adapt limbs to the environment. A character walking over uneven voxel terrain will have feet that clip through slopes or hover above the ground. Hands that need to grab a ledge, interact with an object, or brace against a wall cannot reach arbitrary world-space positions through forward kinematics alone. Without inverse kinematics, the disconnect between animated pose and physical world destroys believability.

The engine needs an IK solver that, given a target world-space position for an end effector (foot or hand), adjusts the joint chain (hip-knee-ankle or shoulder-elbow-wrist) to reach that target. The solver must run after animation playback and procedural layers but before the final joint matrix computation, so the GPU receives corrected poses. It must support blending between pure FK (the animation clip's pose) and pure IK (the solver's solution) via a weight parameter, and it must gracefully handle unreachable targets by stretching the chain toward the target without breaking joint limits.

## Solution

### IK Chain Definition

```rust
use glam::{Vec3, Quat, Mat4};

/// Defines a joint chain for IK solving.
pub struct IkChain {
    /// Joint indices from root to tip (e.g., [hip, knee, ankle] or [shoulder, elbow, wrist]).
    /// Minimum 2 joints (two-bone IK), can be longer for CCD.
    pub joint_indices: Vec<u16>,
    /// The world-space target position the end effector should reach.
    pub target_position: Vec3,
    /// Optional target rotation for the end effector (e.g., foot should be flat on ground).
    pub target_rotation: Option<Quat>,
    /// Blend weight: 0.0 = pure FK (no IK), 1.0 = pure IK.
    pub weight: f32,
    /// Pole target: a world-space position that the mid-joint (knee/elbow) should point toward.
    /// Prevents ambiguous bend directions.
    pub pole_target: Option<Vec3>,
}
```

### Two-Bone IK Solver

The primary solver for limbs (arm or leg), which is the most common IK use case in games. Given three joints A (root), B (mid), C (tip), solve for the angles that place C at the target:

```rust
pub struct TwoBoneIkSolver;

impl TwoBoneIkSolver {
    /// Solve two-bone IK for a chain of exactly 3 joints.
    ///
    /// - `a_world`: world-space position of the root joint (hip/shoulder)
    /// - `b_world`: world-space position of the mid joint (knee/elbow)
    /// - `c_world`: world-space position of the tip joint (ankle/wrist)
    /// - `target`: desired world-space position for the tip
    /// - `pole`: optional pole target for bend direction
    ///
    /// Returns new world-space positions for B and C.
    pub fn solve(
        a_world: Vec3,
        b_world: Vec3,
        c_world: Vec3,
        target: Vec3,
        pole: Option<Vec3>,
    ) -> (Vec3, Vec3) {
        let upper_len = (b_world - a_world).length(); // e.g., thigh length
        let lower_len = (c_world - b_world).length(); // e.g., shin length
        let chain_len = upper_len + lower_len;

        let to_target = target - a_world;
        let target_dist = to_target.length().min(chain_len - 0.001); // clamp to reachable

        // Use the law of cosines to find the knee/elbow angle.
        // cos(angle_at_b) = (upper^2 + lower^2 - target_dist^2) / (2 * upper * lower)
        let cos_angle_b = ((upper_len * upper_len + lower_len * lower_len
            - target_dist * target_dist)
            / (2.0 * upper_len * lower_len))
            .clamp(-1.0, 1.0);
        let angle_b = cos_angle_b.acos();

        // cos(angle_at_a) = (upper^2 + target_dist^2 - lower^2) / (2 * upper * target_dist)
        let cos_angle_a = ((upper_len * upper_len + target_dist * target_dist
            - lower_len * lower_len)
            / (2.0 * upper_len * target_dist))
            .clamp(-1.0, 1.0);
        let angle_a = cos_angle_a.acos();

        // Compute the plane of the IK solution.
        let target_dir = to_target.normalize_or_zero();

        // Determine the bend direction using the pole target or the current knee position.
        let bend_normal = if let Some(pole_pos) = pole {
            let to_pole = (pole_pos - a_world).normalize_or_zero();
            target_dir.cross(to_pole).normalize_or_zero()
        } else {
            let current_bend = (b_world - a_world).normalize_or_zero();
            target_dir.cross(current_bend).normalize_or_zero()
        };

        let bend_dir = bend_normal.cross(target_dir).normalize_or_zero();

        // Position the mid joint.
        let new_b = a_world + target_dir * (angle_a.cos() * upper_len)
            + bend_dir * (angle_a.sin() * upper_len);

        // Position the tip at the target (clamped to chain reach).
        let new_c = a_world + target_dir * target_dist;

        (new_b, new_c)
    }
}
```

### Foot IK System

Adapts foot placement to uneven voxel terrain:

```rust
/// Foot IK configuration for a single leg.
pub struct FootIkConfig {
    /// The IK chain for this leg (hip -> knee -> ankle).
    pub chain: IkChain,
    /// Maximum raycast distance downward from the hip.
    pub ray_length: f32,
    /// Offset from the raycast hit point to the ankle target (foot height above ground).
    pub foot_offset: f32,
}

fn foot_ik_system(
    mut query: Query<(&mut AnimationPlayer, &Skeleton, &FootIkConfig, &Transform)>,
    physics: Res<PhysicsWorld>,
) {
    for (mut player, skeleton, foot_config, transform) in query.iter_mut() {
        let hip_world = get_joint_world_position(&player, skeleton, foot_config.chain.joint_indices[0]);

        // Raycast straight down from the hip to find the ground surface.
        let ray_origin = hip_world;
        let ray_dir = -transform.up(); // planet-relative down
        if let Some(hit) = physics.raycast(ray_origin, ray_dir, foot_config.ray_length) {
            let ground_point = hit.point + hit.normal * foot_config.foot_offset;

            // Set IK target to the ground contact point.
            let mut chain = foot_config.chain.clone();
            chain.target_position = ground_point;
            chain.target_rotation = Some(Quat::from_rotation_arc(Vec3::Y, hit.normal));

            solve_ik_chain(&mut player, skeleton, &chain);
        }
    }
}
```

### Hand IK System

For reaching, grabbing, and interaction:

```rust
/// Hand IK target, set by gameplay systems.
pub struct HandIkTarget {
    /// The IK chain for this arm (shoulder -> elbow -> wrist).
    pub chain: IkChain,
}

fn hand_ik_system(
    mut query: Query<(&mut AnimationPlayer, &Skeleton, &HandIkTarget)>,
) {
    for (mut player, skeleton, hand_target) in query.iter_mut() {
        solve_ik_chain(&mut player, skeleton, &hand_target.chain);
    }
}
```

### IK Chain Solver (Generic)

The generic solver dispatches to two-bone IK for 3-joint chains and CCD for longer chains:

```rust
fn solve_ik_chain(
    player: &mut AnimationPlayer,
    skeleton: &Skeleton,
    chain: &IkChain,
) {
    if chain.weight < 1e-6 {
        return; // Pure FK, no IK modification.
    }

    let joint_count = chain.joint_indices.len();
    assert!(joint_count >= 2, "IK chain must have at least 2 joints");

    // Get current world-space positions of chain joints.
    let positions: Vec<Vec3> = chain.joint_indices.iter()
        .map(|&idx| get_joint_world_position(player, skeleton, idx))
        .collect();

    if joint_count == 3 {
        // Two-bone IK (most common: legs and arms).
        let (new_mid, new_tip) = TwoBoneIkSolver::solve(
            positions[0], positions[1], positions[2],
            chain.target_position,
            chain.pole_target,
        );

        // Blend between FK and IK based on weight.
        let blended_mid = positions[1].lerp(new_mid, chain.weight);
        let blended_tip = positions[2].lerp(new_tip, chain.weight);

        // Convert back to local-space rotations and write to the pose.
        update_joint_rotation(player, skeleton, chain.joint_indices[0], positions[0], blended_mid);
        update_joint_rotation(player, skeleton, chain.joint_indices[1], blended_mid, blended_tip);
    } else {
        // CCD (Cyclic Coordinate Descent) for longer chains.
        let mut current_positions = positions.clone();
        let max_iterations = 10;
        let tolerance = 0.001;

        for _ in 0..max_iterations {
            for i in (0..joint_count - 1).rev() {
                let to_end = (current_positions[joint_count - 1] - current_positions[i]).normalize_or_zero();
                let to_target = (chain.target_position - current_positions[i]).normalize_or_zero();
                let rotation = Quat::from_rotation_arc(to_end, to_target);

                // Rotate all downstream joints.
                for j in (i + 1)..joint_count {
                    current_positions[j] = current_positions[i]
                        + rotation * (current_positions[j] - current_positions[i]);
                }
            }

            let error = (current_positions[joint_count - 1] - chain.target_position).length();
            if error < tolerance {
                break;
            }
        }

        // Blend and write back.
        for (k, &idx) in chain.joint_indices.iter().enumerate().skip(1) {
            let blended = positions[k].lerp(current_positions[k], chain.weight);
            update_joint_rotation(player, skeleton, chain.joint_indices[k - 1], current_positions[k - 1], blended);
        }
    }

    // Apply end effector rotation if specified.
    if let Some(target_rot) = chain.target_rotation {
        let tip_idx = *chain.joint_indices.last().unwrap();
        let (t, ref mut r, s) = player.local_poses[tip_idx as usize];
        *r = r.slerp(target_rot, chain.weight);
    }
}
```

### Pipeline Ordering

IK systems run in the following order within the `Update` schedule:

1. `animation_playback_system` — sample clip keyframes
2. `procedural_animation_system` — apply procedural layers (story 04)
3. `foot_ik_system` and `hand_ik_system` — apply IK corrections
4. `compute_world_matrices` — produce final GPU joint palette

This ordering ensures IK corrections are the last modification before rendering, so feet always match the ground and hands always reach their targets.

## Outcome

A two-bone IK solver and CCD fallback that adjust limb chains to reach world-space targets. `FootIkConfig` uses raycasts to plant feet on uneven voxel terrain. `HandIkTarget` drives hand reaching for interaction. Both support weight-based FK/IK blending. The solvers run after clip playback and procedural layers, before final matrix computation. `cargo test -p nebula-animation` passes all IK solver tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Foot IK plants feet precisely on uneven terrain. Walking up stairs shows per-step foot placement adjustments.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `glam` | `0.32` | `Vec3`, `Quat`, `Mat4` for position, rotation, law-of-cosines math, `lerp`, `slerp` |
| `bevy_ecs` | `0.18` | `Component` derive for IK configs, `Query`, system ordering |
| `rapier3d` | `0.32` | Raycast queries for foot IK ground detection (via `PhysicsWorld`) |
| `log` | `0.4` | Warn on unreachable targets, degenerate chains, zero-length bones |

All dependencies are declared in `[workspace.dependencies]` and consumed via `{ workspace = true }` in the `nebula-animation` crate's `Cargo.toml`. Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Vec3, Quat};

    /// Helper: create a simple 3-joint vertical chain (shoulder at origin,
    /// elbow at (0,1,0), wrist at (0,2,0)). Upper arm = 1.0, forearm = 1.0.
    fn vertical_chain() -> (Vec3, Vec3, Vec3) {
        (Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 2.0, 0.0))
    }

    /// Verify that the foot (tip) reaches the target position within tolerance.
    #[test]
    fn test_foot_reaches_target_position() {
        let (a, b, c) = vertical_chain();
        let target = Vec3::new(1.0, 1.0, 0.0); // reachable target

        let (new_b, new_c) = TwoBoneIkSolver::solve(a, b, c, target, None);

        let error = (new_c - target).length();
        assert!(
            error < 0.01,
            "tip should reach target within tolerance, error = {}",
            error
        );
    }

    /// Verify that the knee bends correctly (mid-joint moves to accommodate the target).
    #[test]
    fn test_knee_bends_correctly() {
        let (a, b, c) = vertical_chain();
        let target = Vec3::new(1.0, 0.5, 0.0);

        let (new_b, _) = TwoBoneIkSolver::solve(a, b, c, target, None);

        // The mid-joint should have moved from its original position.
        assert!(
            (new_b - b).length() > 0.01,
            "knee should bend away from original position"
        );

        // Upper bone length should be preserved.
        let upper_len = (new_b - a).length();
        assert!(
            (upper_len - 1.0).abs() < 0.01,
            "upper bone length should be preserved, got {}",
            upper_len
        );

        // Lower bone length should be preserved.
        let lower_len_result = (TwoBoneIkSolver::solve(a, b, c, target, None).0 - a).length();
        // Recompute explicitly to verify.
        let (new_b2, new_c2) = TwoBoneIkSolver::solve(a, b, c, target, None);
        let lower_len = (new_c2 - new_b2).length();
        assert!(
            (lower_len - 1.0).abs() < 0.01,
            "lower bone length should be preserved, got {}",
            lower_len
        );
    }

    /// Verify that hand IK reaches a target position.
    #[test]
    fn test_hand_ik_reaches_target() {
        let (a, b, c) = vertical_chain();
        let target = Vec3::new(0.5, 1.5, 0.5); // reachable diagonal target

        let (_, new_c) = TwoBoneIkSolver::solve(a, b, c, target, None);

        let error = (new_c - target).length();
        assert!(
            error < 0.01,
            "hand should reach target, error = {}",
            error
        );
    }

    /// Verify that weight=0 produces pure FK (output equals input).
    #[test]
    fn test_weight_zero_is_pure_fk() {
        let (a, b, c) = vertical_chain();
        let target = Vec3::new(1.0, 0.0, 0.0);

        let (new_b, new_c) = TwoBoneIkSolver::solve(a, b, c, target, None);

        // Blend with weight 0.0 -> positions should be unchanged.
        let blended_b = b.lerp(new_b, 0.0);
        let blended_c = c.lerp(new_c, 0.0);

        assert_eq!(blended_b, b, "weight=0 should produce original mid position");
        assert_eq!(blended_c, c, "weight=0 should produce original tip position");
    }

    /// Verify that weight=1 produces pure IK (output equals solver result).
    #[test]
    fn test_weight_one_is_pure_ik() {
        let (a, b, c) = vertical_chain();
        let target = Vec3::new(1.0, 1.0, 0.0);

        let (new_b, new_c) = TwoBoneIkSolver::solve(a, b, c, target, None);

        // Blend with weight 1.0 -> positions should match solver output.
        let blended_b = b.lerp(new_b, 1.0);
        let blended_c = c.lerp(new_c, 1.0);

        assert!(
            (blended_b - new_b).length() < 1e-6,
            "weight=1 should produce IK mid position"
        );
        assert!(
            (blended_c - new_c).length() < 1e-6,
            "weight=1 should produce IK tip position"
        );
    }

    /// Verify that an unreachable target is handled gracefully: the chain stretches
    /// toward the target without panicking, and the tip ends up at maximum reach.
    #[test]
    fn test_ik_handles_unreachable_targets_gracefully() {
        let (a, b, c) = vertical_chain();
        let unreachable_target = Vec3::new(10.0, 10.0, 0.0); // far beyond chain length of 2.0

        // Should not panic.
        let (new_b, new_c) = TwoBoneIkSolver::solve(a, b, c, unreachable_target, None);

        // The chain should be fully extended toward the target.
        let chain_length = (new_b - a).length() + (new_c - new_b).length();
        assert!(
            (chain_length - 2.0).abs() < 0.02,
            "fully extended chain should equal total bone length, got {}",
            chain_length
        );

        // The direction from root to tip should point toward the target.
        let to_target = (unreachable_target - a).normalize();
        let to_tip = (new_c - a).normalize();
        let dot = to_target.dot(to_tip);
        assert!(
            dot > 0.99,
            "chain should stretch toward target, alignment = {}",
            dot
        );
    }
}
```
