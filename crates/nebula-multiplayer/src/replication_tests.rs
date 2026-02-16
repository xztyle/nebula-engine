//! Unit tests for entity replication.

use super::*;

// Test component types.
#[derive(Component, Serialize, Deserialize, Debug, Clone, PartialEq)]
struct Position128 {
    x: i64,
    y: i64,
    z: i64,
}

#[derive(Component, Serialize, Deserialize, Debug, Clone, PartialEq)]
struct MeshHandle(u32);

/// Helper: create a ReplicationSet with Position128 and MeshHandle registered.
fn test_rep_set() -> ReplicationSet {
    let mut set = ReplicationSet::new();
    set.register::<Position128>("Position128");
    set.register::<MeshHandle>("MeshHandle");
    set
}

#[test]
fn test_entity_spawned_on_server_appears_on_client() {
    let rep_set = test_rep_set();
    let mut server = ReplicationServerSystem::new();
    server.add_client(1);

    // Server world with one replicated entity.
    let mut server_world = World::new();
    let net_id = server.allocate_network_id();
    server_world.spawn((
        net_id,
        Position128 {
            x: 100,
            y: 200,
            z: 300,
        },
        MeshHandle(42),
    ));

    // Run replication.
    let msgs = server.replicate(&server_world, &rep_set, 1);
    let client_msgs = &msgs[&1];

    // Client should get exactly one spawn.
    assert_eq!(client_msgs.spawns.len(), 1);
    let spawn = &client_msgs.spawns[0];
    assert_eq!(spawn.network_id, net_id);
    assert_eq!(spawn.components.len(), 2);

    // Apply to client world.
    let mut client_world = World::new();
    let mut client_sys = ReplicationClientSystem::new();
    client_sys.apply(&mut client_world, &rep_set, client_msgs);

    // Verify client entity has correct components.
    let local_entity = client_sys.local_entity(net_id).unwrap();
    let pos = client_world.get::<Position128>(local_entity).unwrap();
    assert_eq!(
        *pos,
        Position128 {
            x: 100,
            y: 200,
            z: 300
        }
    );
    let mesh = client_world.get::<MeshHandle>(local_entity).unwrap();
    assert_eq!(*mesh, MeshHandle(42));
}

#[test]
fn test_component_change_replicates() {
    let rep_set = test_rep_set();
    let mut server = ReplicationServerSystem::new();
    server.add_client(1);

    let mut server_world = World::new();
    let net_id = server.allocate_network_id();
    let entity = server_world
        .spawn((net_id, Position128 { x: 0, y: 0, z: 0 }, MeshHandle(1)))
        .id();

    // First tick: spawn.
    let _ = server.replicate(&server_world, &rep_set, 1);

    // Modify position.
    server_world.get_mut::<Position128>(entity).unwrap().x = 999;

    // Second tick: should get an update with only Position128.
    let msgs = server.replicate(&server_world, &rep_set, 2);
    let client_msgs = &msgs[&1];
    assert!(client_msgs.spawns.is_empty());
    assert_eq!(client_msgs.updates.len(), 1);
    let update = &client_msgs.updates[0];
    assert_eq!(update.network_id, net_id);
    assert_eq!(update.changed_components.len(), 1);
    assert_eq!(update.changed_components[0].0, "Position128");
}

#[test]
fn test_unchanged_components_not_sent() {
    let rep_set = test_rep_set();
    let mut server = ReplicationServerSystem::new();
    server.add_client(1);

    let mut server_world = World::new();
    let net_id = server.allocate_network_id();
    server_world.spawn((net_id, Position128 { x: 1, y: 2, z: 3 }, MeshHandle(5)));

    // First tick: spawn.
    let _ = server.replicate(&server_world, &rep_set, 1);

    // Second tick: no changes.
    let msgs = server.replicate(&server_world, &rep_set, 2);
    let client_msgs = &msgs[&1];
    assert!(client_msgs.spawns.is_empty());
    assert!(client_msgs.updates.is_empty());
    assert!(client_msgs.despawns.is_empty());
}

#[test]
fn test_network_id_is_consistent() {
    let rep_set = test_rep_set();
    let mut server = ReplicationServerSystem::new();
    server.add_client(1);

    let mut server_world = World::new();
    let net_id = NetworkId(42);
    server_world.spawn((
        net_id,
        Position128 {
            x: 10,
            y: 20,
            z: 30,
        },
    ));

    // Tick 1: spawn.
    let msgs = server.replicate(&server_world, &rep_set, 1);
    let client_msgs = &msgs[&1];

    let mut client_world = World::new();
    let mut client_sys = ReplicationClientSystem::new();
    client_sys.apply(&mut client_world, &rep_set, client_msgs);

    // Client entity should have NetworkId(42).
    let local_entity = client_sys.local_entity(NetworkId(42)).unwrap();
    let client_net_id = client_world.get::<NetworkId>(local_entity).unwrap();
    assert_eq!(*client_net_id, NetworkId(42));

    // Modify on server, replicate again.
    // Find the server entity by querying.
    let server_entity = {
        let world_ptr = &server_world as *const World as *mut World;
        unsafe {
            let mut q = (*world_ptr).query::<(Entity, &NetworkId)>();
            q.iter(&*world_ptr).find(|(_, n)| n.0 == 42).unwrap().0
        }
    };
    server_world
        .get_mut::<Position128>(server_entity)
        .unwrap()
        .x = 999;

    let msgs = server.replicate(&server_world, &rep_set, 2);
    let client_msgs = &msgs[&1];
    // Update should reference the same NetworkId.
    assert_eq!(client_msgs.updates.len(), 1);
    assert_eq!(client_msgs.updates[0].network_id, NetworkId(42));
}

#[test]
fn test_despawn_replicates() {
    let rep_set = test_rep_set();
    let mut server = ReplicationServerSystem::new();
    server.add_client(1);

    let mut server_world = World::new();
    let net_id = server.allocate_network_id();
    let entity = server_world
        .spawn((net_id, Position128 { x: 0, y: 0, z: 0 }))
        .id();

    // Tick 1: spawn.
    let msgs = server.replicate(&server_world, &rep_set, 1);
    let mut client_world = World::new();
    let mut client_sys = ReplicationClientSystem::new();
    client_sys.apply(&mut client_world, &rep_set, &msgs[&1]);
    assert!(client_sys.local_entity(net_id).is_some());

    // Despawn on server.
    server_world.despawn(entity);

    // Tick 2: should produce despawn.
    let msgs = server.replicate(&server_world, &rep_set, 2);
    let client_msgs = &msgs[&1];
    assert_eq!(client_msgs.despawns.len(), 1);
    assert_eq!(client_msgs.despawns[0].network_id, net_id);

    // Apply to client.
    client_sys.apply(&mut client_world, &rep_set, client_msgs);
    assert!(client_sys.local_entity(net_id).is_none());
}
