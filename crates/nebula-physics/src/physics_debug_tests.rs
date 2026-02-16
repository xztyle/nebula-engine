use super::*;

#[test]
fn test_debug_vis_toggles_on_off() {
    let mut state = PhysicsDebugState::default();
    assert!(!state.enabled);

    physics_debug_toggle_system(true, &mut state);
    assert!(state.enabled);

    physics_debug_toggle_system(true, &mut state);
    assert!(!state.enabled);
}

#[test]
fn test_collider_shapes_rendered() {
    let mut physics = PhysicsWorld::new();

    let cuboid_col = ColliderBuilder::cuboid(1.0, 1.0, 1.0).build();
    physics.collider_set.insert(cuboid_col);

    let body = RigidBodyBuilder::dynamic()
        .translation(Vector::new(5.0, 0.0, 0.0))
        .build();
    let bh = physics.rigid_body_set.insert(body);
    let capsule_col = ColliderBuilder::capsule_y(0.5, 0.3).build();
    physics
        .collider_set
        .insert_with_parent(capsule_col, bh, &mut physics.rigid_body_set);

    let mut lines = DebugLineBuffer::default();

    for (_handle, collider) in physics.collider_set.iter() {
        let color = match collider.parent() {
            Some(bh2) => match physics.rigid_body_set.get(bh2) {
                Some(b) if b.is_dynamic() => COLORS.dynamic_body,
                Some(b) if b.is_kinematic() => COLORS.kinematic_body,
                _ => COLORS.static_collider,
            },
            None => COLORS.static_collider,
        };
        let position = collider.position();
        let shape = collider.shape();
        match shape.shape_type() {
            ShapeType::Cuboid => {
                if let Some(cuboid) = shape.as_cuboid() {
                    emit_cuboid_wireframe(&mut lines, position, cuboid, color);
                }
            }
            ShapeType::Capsule => {
                if let Some(capsule) = shape.as_capsule() {
                    emit_capsule_wireframe(&mut lines, position, capsule, color);
                }
            }
            _ => {
                let aabb = collider.compute_aabb();
                emit_aabb_wireframe(&mut lines, &aabb, color);
            }
        }
    }

    // Cuboid = 12 edges, capsule = 2×16 ring segments + 4 verticals = 36
    assert!(lines.lines.len() >= 12, "got {}", lines.lines.len());
    assert!(lines.lines.len() >= 48, "got {}", lines.lines.len());
}

#[test]
fn test_contacts_shown_at_correct_positions() {
    let mut physics = PhysicsWorld::new();

    let floor = ColliderBuilder::cuboid(10.0, 0.1, 10.0)
        .translation(Vector::new(0.0, 0.0, 0.0))
        .build();
    physics.collider_set.insert(floor);

    let body = RigidBodyBuilder::dynamic()
        .translation(Vector::new(0.0, 0.15, 0.0))
        .build();
    let bh = physics.rigid_body_set.insert(body);
    let col = ColliderBuilder::cuboid(0.5, 0.5, 0.5).build();
    physics
        .collider_set
        .insert_with_parent(col, bh, &mut physics.rigid_body_set);

    for _ in 0..10 {
        physics.step();
    }

    let mut lines = DebugLineBuffer::default();
    for pair in physics.narrow_phase.contact_pairs() {
        for manifold in pair.manifolds.iter() {
            for contact in manifold.contacts() {
                let p = Vec3::new(contact.local_p1.x, contact.local_p1.y, contact.local_p1.z);
                emit_cross(&mut lines, p, 0.05, COLORS.contact_point);
            }
        }
    }

    assert!(!lines.lines.is_empty(), "Expected contact debug lines");
}

#[test]
fn test_debug_rendering_does_not_affect_physics() {
    let pos_a = {
        let mut physics = PhysicsWorld::new();
        let body = RigidBodyBuilder::dynamic()
            .translation(Vector::new(0.0, 10.0, 0.0))
            .build();
        let bh = physics.rigid_body_set.insert(body);
        let col = ColliderBuilder::ball(0.5).build();
        physics
            .collider_set
            .insert_with_parent(col, bh, &mut physics.rigid_body_set);
        for _ in 0..60 {
            physics.step();
        }
        let t = physics.rigid_body_set[bh].translation();
        (t.x, t.y, t.z)
    };

    let pos_b = {
        let mut physics = PhysicsWorld::new();
        let body = RigidBodyBuilder::dynamic()
            .translation(Vector::new(0.0, 10.0, 0.0))
            .build();
        let bh = physics.rigid_body_set.insert(body);
        let col = ColliderBuilder::ball(0.5).build();
        physics
            .collider_set
            .insert_with_parent(col, bh, &mut physics.rigid_body_set);

        let mut line_buf = DebugLineBuffer::default();
        for _ in 0..60 {
            physics.step();
            line_buf.clear();
            for (_h, collider) in physics.collider_set.iter() {
                let aabb = collider.compute_aabb();
                emit_aabb_wireframe(&mut line_buf, &aabb, COLORS.static_collider);
            }
            for (_h, b) in physics.rigid_body_set.iter() {
                if b.is_dynamic() {
                    let p = b.translation();
                    let v = b.linvel();
                    line_buf.push_line(
                        Vec3::new(p.x, p.y, p.z),
                        Vec3::new(p.x + v.x, p.y + v.y, p.z + v.z),
                        COLORS.velocity,
                    );
                }
            }
        }
        let t = physics.rigid_body_set[bh].translation();
        (t.x, t.y, t.z)
    };

    assert_eq!(pos_a.0.to_bits(), pos_b.0.to_bits(), "x mismatch");
    assert_eq!(pos_a.1.to_bits(), pos_b.1.to_bits(), "y mismatch");
    assert_eq!(pos_a.2.to_bits(), pos_b.2.to_bits(), "z mismatch");
}

#[test]
fn test_disabled_debug_produces_no_lines() {
    let mut physics = PhysicsWorld::new();

    let col = ColliderBuilder::cuboid(1.0, 1.0, 1.0).build();
    physics.collider_set.insert(col);

    let body = RigidBodyBuilder::dynamic()
        .translation(Vector::new(0.0, 1.5, 0.0))
        .build();
    let bh = physics.rigid_body_set.insert(body);
    let col2 = ColliderBuilder::ball(0.5).build();
    physics
        .collider_set
        .insert_with_parent(col2, bh, &mut physics.rigid_body_set);

    for _ in 0..10 {
        physics.step();
    }

    let debug = PhysicsDebugState::default(); // enabled = false
    let lines = DebugLineBuffer::default();

    assert!(!debug.enabled);
    assert!(lines.lines.is_empty(), "got {}", lines.lines.len());
}

#[test]
fn test_velocity_vectors_rendered() {
    let mut physics = PhysicsWorld::new();

    let body = RigidBodyBuilder::dynamic()
        .translation(Vector::new(1.0, 2.0, 3.0))
        .linvel(Vector::new(5.0, 0.0, 0.0))
        .build();
    let bh = physics.rigid_body_set.insert(body);
    let col = ColliderBuilder::ball(0.5).build();
    physics
        .collider_set
        .insert_with_parent(col, bh, &mut physics.rigid_body_set);

    let mut lines = DebugLineBuffer::default();

    for (_h, b) in physics.rigid_body_set.iter() {
        if !b.is_dynamic() {
            continue;
        }
        let pos = b.translation();
        let vel = b.linvel();
        let start = Vec3::new(pos.x, pos.y, pos.z);
        let end = start + Vec3::new(vel.x, vel.y, vel.z);
        lines.push_line(start, end, COLORS.velocity);
    }

    assert!(!lines.lines.is_empty(), "Expected velocity lines");

    let vel_line = &lines.lines[0];
    let body = &physics.rigid_body_set[bh];
    let pos = body.translation();
    assert!(
        (vel_line.start[0] - pos.x).abs() < 0.01,
        "Line should start at body position"
    );
    assert!(
        vel_line.end[0] > vel_line.start[0],
        "Line should extend in +x"
    );
}
