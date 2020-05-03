#![allow(dead_code)]
use crate::{components::*, ecs::prelude::*};
use smallvec::SmallVec;
use std::collections::HashMap;

pub fn build(_: &mut World, _: &mut Resources) -> Vec<Box<dyn Schedulable>> {
    let missing_previous_parent_system = SystemBuilder::<()>::new("MissingPreviousParentSystem")
        // Entities with missing `PreviousParent`
        .with_query(<Read<Parent>>::query().filter(
            component::<LocalToParent>()
                & component::<LocalToWorld>()
                & !component::<PreviousParent>(),
        ))
        .build(move |commands, world, _resource, query| {
            // Add missing `PreviousParent` components
            for (entity, _parent) in query.iter_entities(world) {
                log::trace!("Adding missing PreviousParent to {}", entity);
                commands.add_component(entity, PreviousParent(None));
            }
        });

    let parent_update_system = SystemBuilder::<()>::new("ParentUpdateSystem")
        // Entities with a removed `Parent`
        .with_query(<Read<PreviousParent>>::query().filter(!component::<Parent>()))
        // Entities with a changed `Parent`
        .with_query(<(Read<Parent>, Write<PreviousParent>)>::query().filter(
            component::<LocalToParent>() & component::<LocalToWorld>() & changed::<Parent>(),
        ))
        // Deleted Parents (ie Entities with `Children` and without a `LocalToWorld`).
        .with_query(<Read<Children>>::query().filter(!component::<LocalToWorld>()))
        .write_component::<Children>()
        .build(move |commands, world, _resource, queries| {
            // Entities with a missing `Parent` (ie. ones that have a `PreviousParent`), remove
            // them from the `Children` of the `PreviousParent`.
            for (entity, previous_parent) in queries.0.iter_entities(world) {
                log::trace!("Parent was removed from {}", entity);
                if let Some(previous_parent_entity) = previous_parent.0 {
                    // We have to do unsafe call because world was already borrowed. However,
                    // this is safe because we access `Children` component, which was not present
                    // in previous query and was defined for unique write access in `SystemBuilder`.
                    if let Some(mut previous_parent_children) = unsafe {
                        world.get_component_mut_unchecked::<Children>(previous_parent_entity)
                    } {
                        log::trace!(" > Removing {} from it's prev parent's children", entity);
                        previous_parent_children.0.retain(|e| *e != entity);
                    }
                }
            }

            // Tracks all newly created `Children` Components this frame.
            let mut children_additions =
                HashMap::<Entity, SmallVec<[Entity; 8]>>::with_capacity(16);

            // Entities with a changed Parent (that also have a PreviousParent, even if None)
            // We have to do unsafe call because we mutably access world multiple times. However,
            // this is safe because we ensure that each component is accessed uniquely and components
            // not present in the query (`Children`) were marked as writable by the `SystemBuilder`.
            for (entity, (parent, mut previous_parent)) in
                unsafe { queries.1.iter_entities_unchecked(world) }
            {
                log::trace!("Parent changed for {}", entity);

                // If the `PreviousParent` is not None.
                if let Some(previous_parent_entity) = previous_parent.0 {
                    // New and previous point to the same Entity, carry on, nothing to see here.
                    if previous_parent_entity == parent.0 {
                        log::trace!(" > But the previous parent is the same, ignoring...");
                        continue;
                    }

                    // Remove from `PreviousParent.Children`.
                    if let Some(mut previous_parent_children) = unsafe {
                        world.get_component_mut_unchecked::<Children>(previous_parent_entity)
                    } {
                        log::trace!(" > Removing {} from prev parent's children", entity);
                        (*previous_parent_children).0.retain(|e| *e != entity);
                    }
                }

                // Set `PreviousParent = Parent`.
                *previous_parent = PreviousParent(Some(parent.0));

                // Add to the parent's `Children` (either the real component, or
                // `children_additions`).
                log::trace!("Adding {} to it's new parent {}", entity, parent.0);
                if let Some(mut new_parent_children) =
                    unsafe { world.get_component_mut_unchecked::<Children>(parent.0) }
                {
                    // This is the parent
                    log::trace!(
                        " > The new parent {} already has a `Children`, adding to it.",
                        parent.0
                    );
                    (*new_parent_children).0.push(entity);
                } else {
                    // The parent doesn't have a children entity, lets add it
                    log::trace!(
                        "The new parent {} doesn't yet have `Children` component.",
                        parent.0
                    );
                    children_additions
                        .entry(parent.0)
                        .or_insert_with(Default::default)
                        .push(entity);
                }
            }

            // Deleted `Parents` (ie. Entities with a `Children` but no `LocalToWorld`).
            for (entity, children) in queries.2.iter_entities(world) {
                log::trace!("The entity {} doesn't have a LocalToWorld", entity);
                if children_additions.remove(&entity).is_none() {
                    log::trace!(" > It needs to be remove from the ECS.");
                    for child_entity in children.0.iter() {
                        commands.remove_component::<Parent>(*child_entity);
                        commands.remove_component::<PreviousParent>(*child_entity);
                        commands.remove_component::<LocalToParent>(*child_entity);
                    }
                    commands.remove_component::<Children>(entity);
                } else {
                    log::trace!(" > It was a new addition, removing it from additions map");
                }
            }

            // Flush the `children_additions` to the command buffer. It is stored separate to
            // collect multiple new children that point to the same parent into the same
            // SmallVec, and to prevent redundant add+remove operations.
            children_additions.iter().for_each(|(k, v)| {
                log::trace!("Flushing: Entity {} adding `Children` component {:?}", k, v);
                commands.add_component(*k, Children::with(v));
            });
        });

    vec![missing_previous_parent_system, parent_update_system]
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn correct_children() {
        let _ = env_logger::builder().is_test(true).try_init();

        let mut resources = Resources::default();

        let mut world = Universe::new().create_world();

        let mut systems = build(&mut world, &mut resources);

        // Add parent entities
        let parent = *world
            .insert(
                (),
                vec![(Translation::identity(), LocalToWorld::identity())],
            )
            .first()
            .unwrap();
        let children = world.insert(
            (),
            vec![
                (
                    Translation::identity(),
                    LocalToParent::identity(),
                    LocalToWorld::identity(),
                ),
                (
                    Translation::identity(),
                    LocalToParent::identity(),
                    LocalToWorld::identity(),
                ),
            ],
        );
        let (e1, e2) = (children[0], children[1]);

        // Parent `e1` and `e2` to `parent`.
        world.add_component(e1, Parent(parent)).unwrap();
        world.add_component(e2, Parent(parent)).unwrap();

        for system in systems.iter_mut() {
            system.run(&mut world, &mut resources);
            system
                .command_buffer_mut(world.id())
                .unwrap()
                .write(&mut world);
        }

        assert_eq!(
            world
                .get_component::<Children>(parent)
                .unwrap()
                .0
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            vec![e1, e2]
        );

        // Parent `e1` to `e2`.
        (*world.get_component_mut::<Parent>(e1).unwrap()).0 = e2;

        // Run the system on it
        for system in systems.iter_mut() {
            system.run(&mut world, &mut resources);
            system
                .command_buffer_mut(world.id())
                .unwrap()
                .write(&mut world);
        }

        assert_eq!(
            world
                .get_component::<Children>(parent)
                .unwrap()
                .0
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            vec![e2]
        );

        assert_eq!(
            world
                .get_component::<Children>(e2)
                .unwrap()
                .0
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            vec![e1]
        );

        world.delete(e1);

        // Run the system on it
        for system in systems.iter_mut() {
            system.run(&mut world, &mut resources);
            system
                .command_buffer_mut(world.id())
                .unwrap()
                .write(&mut world);
        }

        assert_eq!(
            world
                .get_component::<Children>(parent)
                .unwrap()
                .0
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            vec![e2]
        );
    }
}
