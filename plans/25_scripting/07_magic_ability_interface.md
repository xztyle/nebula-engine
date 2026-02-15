# Magic Ability Interface

## Problem

The game features a magic system where players cast abilities that interact with the voxel world, entities, and particles. Hard-coding each ability in Rust would make the system rigid and prevent modding. Abilities need to be defined entirely in scripts so that modders can create new spells by writing a `.rhai` file, without touching engine code. The engine must provide a structured interface for defining abilities with metadata (name, cost, cooldown) and a cast function that has access to the full scripting API.

## Solution

### Ability Definition API

Scripts define abilities by calling a registration function:

```rhai
define_ability(#{
    name: "Fireball",
    mana_cost: 25.0,
    cooldown: 3.0,      // seconds
    range: 50.0,
    icon: "textures/icons/fireball.png",
    description: "Hurls a ball of fire that explodes on impact.",
    cast: |caster, target| {
        let dir = target.position - get_position(caster);
        let projectile = spawn_entity("fireball_projectile");
        set_position(projectile, get_position(caster));
        set_velocity(projectile, dir.normalize() * 30.0);

        // Explode on impact (registered via event)
        on_entity_collision(projectile, |hit_entity| {
            let pos = get_position(projectile);
            // Damage nearby entities
            let nearby = get_entities_near(pos, 5.0);
            for e in nearby {
                apply_damage(e, 40.0, caster);
            }
            // Destroy voxels in radius
            for dx in -2..3 {
                for dy in -2..3 {
                    for dz in -2..3 {
                        set_voxel(pos.x + dx, pos.y + dy, pos.z + dz, AIR);
                    }
                }
            }
            // Spawn explosion particles
            spawn_particles(pos, "explosion_fire", 50);
            despawn_entity(projectile);
        });
    }
});
```

### AbilityRegistry

The engine maintains an `AbilityRegistry` resource:

```rust
#[derive(Resource)]
pub struct AbilityRegistry {
    pub abilities: HashMap<String, ScriptAbility>,
}

pub struct ScriptAbility {
    pub name: String,
    pub mana_cost: f64,
    pub cooldown: f64,
    pub range: f64,
    pub icon: String,
    pub description: String,
    /// The AST containing the cast function
    pub ast: Arc<AST>,
    /// Function name for the cast callback
    pub cast_fn_name: String,
}
```

When `define_ability` is called from a script, the engine extracts the metadata from the Rhai object-map and stores a `ScriptAbility` in the registry.

### Casting Pipeline

When a player activates an ability:

1. **Cooldown check**: The `AbilityCooldownTracker` component on the player entity is consulted. If the ability is still on cooldown, the cast is rejected and the UI shows the remaining time.

2. **Mana check**: The player's `Mana` component is checked. If insufficient, the cast is rejected with feedback.

3. **Range check**: If the ability has a target, distance is validated against `range`.

4. **Mana deduction**: `mana.current -= ability.mana_cost`.

5. **Cooldown start**: `cooldown_tracker.set(ability_name, ability.cooldown)`.

6. **Execute cast function**: The cast closure is invoked with `(caster_entity_id, target_info)` via the script engine. The cast function has full access to the ECS and voxel APIs.

```rust
fn cast_ability(
    player: Entity,
    ability_name: &str,
    target: TargetInfo,
    registry: &AbilityRegistry,
    engine: &ScriptEngine,
    mana: &mut Mana,
    cooldowns: &mut AbilityCooldownTracker,
) -> Result<(), CastError> {
    let ability = registry.abilities.get(ability_name)
        .ok_or(CastError::UnknownAbility)?;

    if cooldowns.is_on_cooldown(ability_name) {
        return Err(CastError::OnCooldown(cooldowns.remaining(ability_name)));
    }
    if mana.current < ability.mana_cost {
        return Err(CastError::InsufficientMana);
    }

    mana.current -= ability.mana_cost;
    cooldowns.set(ability_name, ability.cooldown);

    let caster_id = ScriptEntityId(player.to_bits());
    let target_data = target.to_script_dynamic();
    engine.call_fn_with_timeout(
        &mut scope, &ability.ast, &ability.cast_fn_name,
        (caster_id, target_data),
    )?;

    Ok(())
}
```

### Spell Book UI Integration

The `AbilityRegistry` is queried by the UI system to build the spell book:

- Each registered ability appears as an entry with its icon, name, description, mana cost, and cooldown.
- Abilities are sorted alphabetically or by category tags.
- The UI shows real-time cooldown progress and grays out abilities when mana is insufficient.
- Drag-and-drop from the spell book to the hotbar assigns abilities to keybinds.

### Helper Functions for Abilities

Additional API functions available within cast functions:

```rhai
apply_damage(entity, amount, source)   // queue damage event
apply_heal(entity, amount, source)     // queue heal event
spawn_particles(pos, effect, count)    // create particle burst
play_sound(pos, sound_name)            // spatial audio
apply_status(entity, "burning", 5.0)   // status effect with duration
set_velocity(entity, vec3)             // set entity velocity
```

## Outcome

A data-driven magic ability system where abilities are defined entirely in `.rhai` scripts with metadata and a cast function, an engine-side registry that manages validation/cooldowns/mana, and UI integration that automatically populates the spell book from registered abilities.

## Demo Integration

**Demo crate:** `nebula-demo`

A fireball ability scripted in Rhai: charge phase, projectile with particle trail, impact explosion with voxel destruction radius.

## Crates & Dependencies

| Crate | Version | Purpose |
|---|---|---|
| `rhai` | `1.23` | Ability script execution and closure capture |

## Unit Tests

```rust
#[test]
fn test_ability_registers_with_correct_properties() {
    let mut world = setup_test_world();
    load_ability_script(&mut world, "test_fireball.rhai");
    run_script_system(&mut world); // triggers define_ability

    let registry = world.resource::<AbilityRegistry>();
    let fireball = registry.abilities.get("Fireball").unwrap();
    assert_eq!(fireball.name, "Fireball");
    assert!((fireball.mana_cost - 25.0).abs() < f64::EPSILON);
    assert!((fireball.cooldown - 3.0).abs() < f64::EPSILON);
}

#[test]
fn test_cast_function_executes() {
    let mut world = setup_test_world();
    load_ability_script(&mut world, "test_fireball.rhai");
    run_script_system(&mut world);

    let player = spawn_player_with_mana(&mut world, 100.0);
    let target = TargetInfo::position(ScriptVec3::new(10.0, 0.0, 0.0));
    let result = cast_ability_in_world(&mut world, player, "Fireball", target);
    assert!(result.is_ok());

    // Verify the cast function produced side effects (e.g., spawned a projectile)
    let commands = get_pending_commands(&world);
    assert!(commands.iter().any(|c| matches!(c, ScriptCommand::SpawnEntity { archetype, .. } if archetype == "fireball_projectile")));
}

#[test]
fn test_mana_is_deducted() {
    let mut world = setup_test_world();
    load_ability_script(&mut world, "test_fireball.rhai");
    run_script_system(&mut world);

    let player = spawn_player_with_mana(&mut world, 100.0);
    cast_ability_in_world(&mut world, player, "Fireball", TargetInfo::none()).unwrap();

    let mana = world.get::<Mana>(player).unwrap();
    assert!((mana.current - 75.0).abs() < f64::EPSILON); // 100 - 25
}

#[test]
fn test_cooldown_prevents_immediate_recast() {
    let mut world = setup_test_world();
    load_ability_script(&mut world, "test_fireball.rhai");
    run_script_system(&mut world);

    let player = spawn_player_with_mana(&mut world, 100.0);
    cast_ability_in_world(&mut world, player, "Fireball", TargetInfo::none()).unwrap();

    // Immediate recast should fail
    let result = cast_ability_in_world(&mut world, player, "Fireball", TargetInfo::none());
    assert!(matches!(result, Err(CastError::OnCooldown(_))));

    // After cooldown expires, should work again
    advance_time(&mut world, 3.1);
    tick_cooldowns(&mut world);
    let result = cast_ability_in_world(&mut world, player, "Fireball", TargetInfo::none());
    assert!(result.is_ok());
}

#[test]
fn test_ability_affects_target() {
    let mut world = setup_test_world();
    // Script: define_ability with cast that applies 40 damage to target
    load_ability_script(&mut world, "test_direct_damage.rhai");
    run_script_system(&mut world);

    let player = spawn_player_with_mana(&mut world, 100.0);
    let target_entity = spawn_entity_with_health(&mut world, 100.0);
    let target = TargetInfo::entity(target_entity);
    cast_ability_in_world(&mut world, player, "Zap", target).unwrap();
    apply_pending_commands(&mut world);

    let health = world.get::<Health>(target_entity).unwrap();
    assert!((health.current - 60.0).abs() < f64::EPSILON); // 100 - 40
}
```
