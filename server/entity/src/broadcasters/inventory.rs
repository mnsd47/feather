//! Broadcasting of inventory-related events.

use crate::inventory::Equipment;
use feather_core::inventory::{Area, Inventory, SlotIndex, Window};
use feather_core::network::packets::{EntityEquipment, SetSlot};
use feather_server_types::{
    EntitySendEvent, Game, HeldItem, InventoryUpdateEvent, Network, NetworkId,
};
use fecs::World;
use num_traits::ToPrimitive;

/// System for broadcasting equipment updates.
#[fecs::event_handler]
pub fn on_inventory_update_broadcast_equipment_update(
    event: &InventoryUpdateEvent,
    game: &mut Game,
    world: &mut World,
) {
    let inv = world.get::<Inventory>(event.entity);
    let held_item = world.get::<HeldItem>(event.entity);

    for slot in &event.slots {
        // Skip this slot if it is not an equipment update.
        if let Ok(equipment) = is_equipment_update(held_item.0, *slot) {
            let slot = equipment.slot_index(held_item.0);
            let item = inv
                .item_at(slot.area, slot.slot)
                .expect("invalid InventoryUpdateEvent");

            let packet = EntityEquipment {
                entity_id: world.get::<NetworkId>(event.entity).0,
                slot: equipment.to_i32().unwrap(),
                item,
            };

            game.broadcast_entity_update(world, packet, event.entity, Some(event.entity));
        }
    }
}

/// System to send an entity's equipment when the
/// entity is sent to a client.
#[fecs::event_handler]
pub fn on_entity_send_send_equipment(event: &EntitySendEvent, world: &mut World) {
    let client = event.client;
    let entity = event.entity;
    if !world.is_alive(client) || !world.is_alive(entity) {
        return;
    }

    let network = world.get::<Network>(client);
    let inventory = match world.try_get::<Inventory>(entity) {
        Some(inv) => inv,
        None => return, // no equipment to send
    };
    let held_item = world.get::<HeldItem>(entity);

    let equipments = [
        Equipment::MainHand,
        Equipment::Boots,
        Equipment::Leggings,
        Equipment::Chestplate,
        Equipment::Helmet,
        Equipment::OffHand,
    ];

    for equipment in equipments.iter() {
        let item = {
            let slot = equipment.slot_index(held_item.0);
            match inventory.item_at(slot.area, slot.slot).unwrap() {
                Some(item) => item,
                None => continue, // don't send equipment if it doesn't exist
            }
        };

        let equipment_slot = equipment.to_i32().unwrap();

        let packet = EntityEquipment {
            entity_id: world.get::<NetworkId>(entity).0,
            slot: equipment_slot,
            item: Some(item),
        };
        network.send(packet);
    }
}

/// System for sending the Set Slot packet
/// when a player's inventory is updated.
#[fecs::event_handler]
pub fn on_inventory_update_send_set_slot(event: &InventoryUpdateEvent, world: &mut World) {
    let inv = world.get::<Inventory>(event.entity);
    let network = world.get::<Network>(event.entity);
    let window = world.get::<Window>(event.entity);

    for slot in &event.slots {
        let converted = window.convert_slot(*slot, event.entity).unwrap_or(0);
        let packet = SetSlot {
            window_id: 0,
            slot: converted as i16,
            slot_data: inv.item_at(slot.area, slot.slot).unwrap(),
        };

        network.send(packet);
    }
}

/// Returns whether the given update to an inventory
/// is an equipment update.
fn is_equipment_update(held_item: usize, slot: SlotIndex) -> Result<Equipment, ()> {
    if slot.area == Area::Hotbar && slot.slot == held_item {
        Ok(Equipment::MainHand)
    } else if let Some(equipment) = Equipment::from_slot_index(slot, held_item) {
        Ok(equipment)
    } else {
        Err(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use feather_core::items::{Item, ItemStack};
    use feather_test_framework::Test;
    use smallvec::smallvec;

    #[test]
    fn broadcast_equipment_updates() {
        let mut test = Test::new();

        let player1 = test.player("", position!(0.0, 100.0, 0.0));
        let player2 = test.player("", position!(45.0, 150.0, 45.0));
        let player3 = test.player("", position!(1000.00, 100.0, 0.0));

        let slot = SlotIndex {
            area: Area::Hotbar,
            slot: 2,
        };
        let stack = ItemStack::new(Item::Stone, 48);
        test.world.get_mut::<HeldItem>(player1).0 = 2;
        test.world
            .get::<Inventory>(player1)
            .set_item_at(slot.area, slot.slot, stack)
            .unwrap();

        test.handle(
            InventoryUpdateEvent {
                slots: smallvec![slot],
                entity: player1,
            },
            on_inventory_update_broadcast_equipment_update,
        );

        let packet = test.sent::<EntityEquipment>(player2).unwrap();
        assert_eq!(packet.entity_id, test.id(player1));
        assert_eq!(packet.item, Some(stack));
        assert_eq!(packet.slot, Equipment::MainHand.to_i32().unwrap());

        assert!(test.sent::<EntityEquipment>(player3).is_none());
        assert!(test.sent::<EntityEquipment>(player1).is_none());

        // now do player3
        test.world.get_mut::<HeldItem>(player3).0 = 2;
        test.world
            .get::<Inventory>(player3)
            .set_item_at(slot.area, slot.slot, stack)
            .unwrap();

        test.handle(
            InventoryUpdateEvent {
                slots: smallvec![slot],
                entity: player3,
            },
            on_inventory_update_broadcast_equipment_update,
        );

        for player in &[player1, player2, player3] {
            assert!(test.sent::<EntityEquipment>(*player).is_none());
        }
    }

    #[test]
    fn send_equipment_on_send() {
        let mut test = Test::new();

        let stack = ItemStack::new(Item::EnderPearl, 15);
        let slot = SlotIndex {
            area: Area::Hotbar,
            slot: 0,
        };
        let (packet, player) = test.broadcast_routine::<EntityEquipment, _, _, _>(
            |test, player1, player2| {
                test.world
                    .get::<Inventory>(player1)
                    .set_item_at(slot.area, slot.slot, stack)
                    .unwrap();
                EntitySendEvent {
                    entity: player1,
                    client: player2,
                }
            },
            on_entity_send_send_equipment,
            false,
        );

        assert_eq!(packet.slot, Equipment::MainHand.to_i32().unwrap());
        assert_eq!(packet.entity_id, test.id(player));
        assert_eq!(packet.item, Some(stack));
    }

    #[test]
    fn send_set_slot() {
        let mut test = Test::new();

        let stack = ItemStack::new(Item::RedstoneOre, 4);
        let slot = SlotIndex {
            area: Area::Main,
            slot: 4,
        };

        let player1 = test.player("", position!(0.0, 74.0, 0.0));
        let player2 = test.player("", position!(0.0, 50.0, 1.5));

        test.world
            .get::<Inventory>(player1)
            .set_item_at(slot.area, slot.slot, stack)
            .unwrap();

        test.handle(
            InventoryUpdateEvent {
                slots: smallvec![slot],
                entity: player1,
            },
            on_inventory_update_send_set_slot,
        );

        let packet = test.sent::<SetSlot>(player1).unwrap();
        assert_eq!(
            packet.slot,
            Window::player(player1).convert_slot(slot, player1).unwrap() as i16
        );
        assert_eq!(packet.slot_data, Some(stack));

        assert!(test.sent::<SetSlot>(player2).is_none());
    }
}
