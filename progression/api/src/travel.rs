//! The travel branches here in the game: the horse (its modifiers, binding, "Поклик",
//! "Сідельні сумки"), the minecart's "Кондуктор" and the elytra perks.
//! "Швидкі колії" and "Розгін" run inside the game's minecart tick (`apply.rs`); this file only
//! tells it who has them.
//!
//! Locks as in lib.rs: nothing of the host is called while `rides`, the engine or the queue is
//! locked.

use crate::core::engine::{HORSE_MODIFIERS, HORSES, Tone, Travel};
use crate::core::store::Horse;
use crate::{Progression, line, relock};
use horizon_client::mod_api::*;
use std::collections::{HashMap, HashSet};

/// A player got on or off a horse or a minecart (from `entity_mount`, done at the tick).
pub(crate) struct Ride {
    pub uuid: u128,
    pub dimension: String,
    /// The vehicle's UUID and kind.
    pub vehicle: u128,
    pub kind: String,
    pub mounting: bool,
}

/// A horse on its way to its rider ("Поклик"): waits for its chunk to load.
struct Call {
    uuid: u128,
    horse: u128,
    dimension: String,
    /// The chunk this call loaded, to let go of after.
    chunk: Option<[i32; 2]>,
    ticks: u32,
}

/// Ticks a call waits for a far horse's chunk.
const CALL_TICKS: u32 = 100;
/// How near the horse (blocks) the saddlebags open without riding it.
const BAG_REACH: f64 = 12.0;
const BAG_TITLE: &str = "Сідельні сумки";

#[derive(Default)]
pub(crate) struct Rides {
    pub travel: HashMap<u128, Travel>,
    /// Players' entity ids a wall doesn't hurt ("М'яка посадка").
    pub walls: HashSet<i32>,
    pub gliding: HashSet<u128>,
    /// Who rides which horse now.
    pub riding: HashMap<u128, u128>,
    calls: Vec<Call>,
    /// Open saddlebags: player -> window.
    pub bags: HashMap<u128, i32>,
}

/// A UUID as commands take it: `8-4-4-4-12` hex.
pub(crate) fn uuid_text(u: u128) -> String {
    let h = format!("{u:032x}");
    format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
}

fn chunk_of(pos: [f64; 3]) -> [i32; 2] {
    [(pos[0].floor() as i32) >> 4, (pos[2].floor() as i32) >> 4]
}

impl Progression {
    fn rides(&self) -> std::sync::MutexGuard<'_, Rides> {
        relock(&self.rides)
    }

    fn bar(host: &mut dyn Host, uuid: u128, text: &str, tone: Tone) {
        let _ = host.action_bar(uuid, line(text, tone));
    }

    fn command(host: &mut dyn Host, c: &str) -> bool {
        let ok = host.run_command(c);
        if !ok {
            log::debug!(target: "horizon::progression", "[Progression] the server refused: {c}");
        }
        ok
    }

    /// The horse modifiers of a rider's perks on (or off) the horse.
    fn horse_modifiers(host: &mut dyn Host, horse: u128, travel: Option<&Travel>) {
        let id = uuid_text(horse);
        for (attribute, modifier) in HORSE_MODIFIERS {
            // Fails when it isn't there; that is fine.
            let _ = host.run_command(&format!("attribute {id} {attribute} modifier remove {modifier}"));
        }
        for (attribute, modifier, amount, op) in travel.map(|t| t.horse.clone()).unwrap_or_default() {
            Self::command(host, &format!("attribute {id} {attribute} modifier add {modifier} {amount} {op}"));
        }
    }

}

/// An entity's attribute base, read with `/attribute ... base get` (None when it can't be).
fn base_of(host: &mut dyn Host, dimension: &str, uuid: u128, attribute: &str) -> Option<f64> {
    let r = host.run_command_captured(&format!("execute in {dimension} run attribute {} {attribute} base get 1000000", uuid_text(uuid)));
    (r.ran && r.result > 0).then(|| f64::from(r.result) / 1_000_000.0)
}

impl Progression {
    /// The horse of an opened mount inventory (the ridden one, or the nearest within 8 blocks):
    /// its speed, jump and health. A Horizon client draws them over the screen, others get chat.
    pub(crate) fn mount_stats(&self, host: &mut dyn Host, players: &[NativeServerPlayer], opens: Vec<(u128, String)>, horizon: &[u128]) {
        for (uuid, dimension) in opens {
            let Some(me) = players.iter().find(|p| p.uuid == uuid) else { continue };
            let ridden = self.rides().riding.get(&uuid).copied();
            let horse = ridden.or_else(|| {
                let near = host
                    .level_entities(&dimension)
                    .into_iter()
                    .filter(|(_, kind, _)| HORSES.contains(&kind.as_str()))
                    .map(|(id, _, at)| (id, (0..3).map(|i| (at[i] - me.position[i]).powi(2)).sum::<f64>()))
                    .filter(|(_, d)| *d < 64.0)
                    .min_by(|a, b| a.1.total_cmp(&b.1))?;
                host.entity(near.0).and_then(|e| host.entity_uuid(e))
            });
            let Some(horse) = horse else { continue };
            let speed = base_of(host, &dimension, horse, "minecraft:movement_speed").unwrap_or(0.0);
            let jump = base_of(host, &dimension, horse, "minecraft:jump_strength").unwrap_or(0.0);
            let health = base_of(host, &dimension, horse, "minecraft:max_health").unwrap_or(0.0);
            if horizon.contains(&uuid) {
                let text = format!("{speed:.4}\t{jump:.4}\t{health:.1}\t{:.4}", crate::core::engine::FOAL_SPEED_MAX);
                let _ = host.net_send_to_player(uuid, crate::MOUNT_CHANNEL, text.as_bytes());
            } else {
                let (mps, blocks) = crate::core::engine::horse_numbers(speed, jump);
                host.tell(
                    uuid,
                    line(&format!("Кінь: швидкість {mps:.1} м/с ({speed:.4}), стрибок {blocks:.1} бл., здоров'я {health:.0}"), Tone::Info),
                );
            }
        }
    }
}

impl Progression {
    /// Foals just born: a speed from their parents' with twice vanilla's top, so breeding can
    /// go on past it.
    pub(crate) fn foals(&self, host: &mut dyn Host, foals: Vec<[u128; 3]>) {
        for [child, parent, partner] in foals {
            let Some((dimension, _, _)) = host.entity_by_uuid(child) else { continue };
            let a = base_of(host, &dimension, parent, "minecraft:movement_speed");
            let b = base_of(host, &dimension, partner, "minecraft:movement_speed");
            let (Some(a), Some(b)) = (a, b) else { continue };
            let quality = (self.chance() + self.chance() + self.chance()) / 3.0 - 0.5;
            let v = crate::core::engine::foal_speed(a, b, quality);
            host.run_command(&format!("execute in {dimension} run attribute {} minecraft:movement_speed base set {v:.6}", uuid_text(child)));
        }
    }

    /// "Крізь хащі": the leaves in front of a horse with such a rider go, without drops.
    fn clear_leaves(&self, host: &mut dyn Host, players: &[NativeServerPlayer]) {
        let riders: Vec<(u128, u128)> = {
            let r = self.rides();
            r.riding.iter().filter(|(p, _)| r.travel.get(p).is_some_and(|t| t.leaves)).map(|(p, h)| (*p, *h)).collect()
        };
        for (uuid, horse) in riders {
            let Some(me) = players.iter().find(|p| p.uuid == uuid) else { continue };
            let Some((dimension, id, _)) = host.entity_by_uuid(horse) else { continue };
            let Some(pos) = host.entity(id).and_then(|e| host.entity_position(e)) else { continue };
            let Some(level) = host.level(&dimension) else { continue };
            let leaves: Vec<[i32; 3]> = crate::core::engine::leaf_box(pos, me.yaw)
                .into_iter()
                .filter(|p| host.level_block(level, *p).is_some_and(|b| crate::core::engine::is_leaves(&b)))
                .collect();
            for p in leaves {
                host.run_command(&format!("execute in {dimension} run setblock {} {} {} minecraft:air", p[0], p[1], p[2]));
            }
        }
    }

    /// Each tick: rides, boosts and calls; every second the perks and the elytra.
    pub(crate) fn travel_tick(&self, host: &mut dyn Host, players: &[NativeServerPlayer], tick: u32, rides: Vec<Ride>, boosts: Vec<u128>) {
        if tick % 20 == 0 {
            self.travel_second(host, players);
        }
        for r in rides {
            self.ride(host, players, r);
        }
        if tick % 2 == 0 {
            self.clear_leaves(host, players);
        }
        for uuid in boosts {
            let chance = self.rides().travel.get(&uuid).map_or(0.0, |t| t.rockets);
            if chance > 0.0 && self.chance() < chance {
                let rocket = host
                    .held_stack(uuid)
                    .filter(|s| &*s.item == "minecraft:firework_rocket")
                    .map(|mut s| {
                        s.count = 1;
                        s
                    })
                    .unwrap_or_else(|| ItemStack::new("minecraft:firework_rocket", 1));
                let _ = host.give_stack(uuid, rocket);
            }
        }
        self.calls(host, players);
    }

    fn travel_second(&self, host: &mut dyn Host, players: &[NativeServerPlayer]) {
        let uuids: Vec<u128> = players.iter().map(|p| p.uuid).collect();
        // The perks, and where the horses ridden now are.
        let travel = {
            let mut guard = self.engine();
            let Some(e) = guard.as_mut() else {
                return;
            };
            let riding = self.rides().riding.clone();
            for p in players {
                if let Some(h) = riding.get(&p.uuid)
                    && let Some(mut mine) = e.horse(p.uuid).cloned()
                    && mine.uuid == format!("{h:032x}")
                {
                    mine.dimension.clone_from(&p.dimension);
                    mine.pos = p.position;
                    let _ = e.set_horse(p.uuid, mine);
                }
            }
            uuids.iter().map(|u| (*u, e.travel(*u))).collect::<HashMap<u128, Travel>>()
        };
        horizon_client::progression_attrs::set_carts(
            travel.iter().filter(|(_, t)| t.cart_steps > 1 || t.cart_drive).map(|(u, t)| (*u, t.cart_steps, t.cart_drive)).collect(),
        );
        let walls: HashSet<i32> = players.iter().filter(|p| travel.get(&p.uuid).is_some_and(|t| t.wall)).map(|p| p.entity_id).collect();
        // "Легкі крила": each second of a glide the elytra may keep its durability.
        let wearing: Vec<(u128, f64)> = {
            let mut r = self.rides();
            r.gliding.retain(|u| uuids.contains(u));
            let out = r.gliding.iter().filter_map(|u| Some((*u, travel.get(u)?.wear))).filter(|(_, w)| *w > 0.0).collect();
            r.travel = travel;
            r.walls = walls;
            out
        };
        for (uuid, wear) in wearing {
            if self.chance() >= wear {
                continue;
            }
            let Some(old) = host.player_equipment(uuid, EquipmentSlot::Chest) else {
                continue;
            };
            if &*old.item != "minecraft:elytra" || old.damage == 0 {
                continue;
            }
            let mut new = old.clone();
            new.damage -= 1;
            let _ = host.set_inventory_slot(uuid, NativeInventoryKind::Player, 38, Some(&old), Some(new));
        }
    }

    fn ride(&self, host: &mut dyn Host, players: &[NativeServerPlayer], r: Ride) {
        let Some(me) = players.iter().find(|p| p.uuid == r.uuid) else {
            return;
        };
        let travel = self.rides().travel.get(&r.uuid).cloned();
        if r.kind == "minecraft:minecart" {
            // "Кондуктор": the cart back to the inventory, once it is surely gone.
            if !r.mounting
                && travel.as_ref().is_some_and(|t| t.cart_return)
                && Self::command(host, &format!("execute in {} run kill {}", r.dimension, uuid_text(r.vehicle)))
            {
                let _ = host.give_stack(r.uuid, ItemStack::new("minecraft:minecart", 1));
            }
            return;
        }
        if !HORSES.contains(&r.kind.as_str()) {
            return;
        }
        if r.mounting {
            Self::horse_modifiers(host, r.vehicle, travel.as_ref());
            self.rides().riding.insert(r.uuid, r.vehicle);
            let binds = travel.as_ref().is_some_and(|t| t.call || t.bag_rows > 0);
            if binds {
                let horse = Horse { uuid: format!("{:032x}", r.vehicle), kind: r.kind.clone(), dimension: r.dimension.clone(), pos: me.position };
                let (new, effects) = {
                    let mut guard = self.engine();
                    match guard.as_mut() {
                        Some(e) => {
                            let new = e.horse(r.uuid).is_none_or(|h| h.uuid != horse.uuid);
                            (new, e.set_horse(r.uuid, horse))
                        }
                        None => (false, Vec::new()),
                    }
                };
                self.apply(host, effects);
                if new {
                    Self::bar(host, r.uuid, "Цей кінь тепер твій: поклик, безсмертя і сумки для нього", Tone::Good);
                }
            }
        } else {
            Self::horse_modifiers(host, r.vehicle, None);
            self.rides().riding.remove(&r.uuid);
            let mut guard = self.engine();
            if let Some(e) = guard.as_mut()
                && let Some(mut h) = e.horse(r.uuid).cloned()
                && h.uuid == format!("{:032x}", r.vehicle)
            {
                h.dimension.clone_from(&r.dimension);
                h.pos = me.position;
                let _ = e.set_horse(r.uuid, h);
            }
        }
    }

    /// "Поклик": the horse comes to its rider, its chunk loaded for it when it is far.
    pub(crate) fn call_horse(&self, host: &mut dyn Host, me: &NativeServerPlayer) {
        let uuid = me.uuid;
        if !self.rides().travel.get(&uuid).is_some_and(|t| t.call) {
            return Self::bar(host, uuid, "Потрібен перк «Поклик» (гілка коня)", Tone::Bad);
        }
        let Some(h) = self.engine().as_ref().and_then(|e| e.horse(uuid).cloned()) else {
            return Self::bar(host, uuid, "Спершу осідлай свого коня: поклик кличе останнього", Tone::Bad);
        };
        let Ok(horse) = u128::from_str_radix(&h.uuid, 16) else {
            return;
        };
        if self.rides().riding.get(&uuid) == Some(&horse) {
            return Self::bar(host, uuid, "Ти вже на ньому", Tone::Info);
        }
        if self.rides().calls.iter().any(|c| c.uuid == uuid) {
            return Self::bar(host, uuid, "Кінь уже біжить", Tone::Info);
        }
        match host.entity_by_uuid(horse) {
            Some((dimension, _, _)) if dimension == me.dimension => {
                self.bring(host, me, horse);
            }
            Some(_) => Self::bar(host, uuid, "Твій кінь в іншому світі", Tone::Bad),
            None if h.dimension != me.dimension => Self::bar(host, uuid, "Твій кінь в іншому світі", Tone::Bad),
            None => {
                // Far away: load its chunk for a few seconds and wait for it.
                let c = chunk_of(h.pos);
                let (x, z) = (c[0] * 16, c[1] * 16);
                let held = host.run_command(&format!("execute in {} run forceload query {x} {z}", h.dimension));
                let chunk = if held {
                    None
                } else if Self::command(host, &format!("execute in {} run forceload add {x} {z}", h.dimension)) {
                    Some(c)
                } else {
                    return Self::bar(host, uuid, "Не вдалося дістатися до коня", Tone::Bad);
                };
                self.rides().calls.push(Call { uuid, horse, dimension: h.dimension.clone(), chunk, ticks: 0 });
                Self::bar(host, uuid, "Кличу коня...", Tone::Info);
            }
        }
    }

    fn bring(&self, host: &mut dyn Host, me: &NativeServerPlayer, horse: u128) {
        let [x, y, z] = me.position;
        if Self::command(host, &format!("execute in {} run tp {} {x} {y} {z}", me.dimension, uuid_text(horse))) {
            Self::bar(host, me.uuid, "Кінь прибіг", Tone::Good);
        } else {
            Self::bar(host, me.uuid, "Кінь не може сюди прийти", Tone::Bad);
        }
    }

    fn calls(&self, host: &mut dyn Host, players: &[NativeServerPlayer]) {
        let calls = std::mem::take(&mut self.rides().calls);
        let mut left = Vec::new();
        for mut c in calls {
            c.ticks += 1;
            let me = players.iter().find(|p| p.uuid == c.uuid);
            let found = host.entity_by_uuid(c.horse);
            let done = match (me, found) {
                (Some(me), Some((dimension, _, _))) if dimension == me.dimension => {
                    self.bring(host, me, c.horse);
                    true
                }
                (Some(_), None) if c.ticks < CALL_TICKS => false,
                (Some(me), _) => {
                    Self::bar(host, me.uuid, "Кінь не відгукнувся: можливо, його вже немає", Tone::Bad);
                    true
                }
                (None, _) => true,
            };
            if !done {
                left.push(c);
                continue;
            }
            if let Some([cx, cz]) = c.chunk {
                Self::command(host, &format!("execute in {} run forceload remove {} {}", c.dimension, cx * 16, cz * 16));
            }
        }
        self.rides().calls.extend(left);
    }

    /// "Сідельні сумки": the horse's bags, beside it or on it. What is in them can always be
    /// taken out, the perk or not.
    pub(crate) fn open_bags(&self, host: &mut dyn Host, me: &NativeServerPlayer) {
        let uuid = me.uuid;
        let perk_rows = self.rides().travel.get(&uuid).map_or(0, |t| t.bag_rows);
        let (bag, horse) = match self.engine().as_ref() {
            Some(e) => (e.bag(uuid), e.horse(uuid).cloned()),
            None => return,
        };
        let need = bag.iter().rposition(|s| s != "-").map_or(0, |i| i / 9 + 1) as u8;
        let rows = perk_rows.max(need).min(6);
        if rows == 0 {
            return Self::bar(host, uuid, "Потрібен перк «Сідельні сумки» (гілка коня)", Tone::Bad);
        }
        let Some(h) = horse else {
            return Self::bar(host, uuid, "Спершу осідлай свого коня", Tone::Bad);
        };
        let on_it = u128::from_str_radix(&h.uuid, 16).is_ok_and(|hu| self.rides().riding.get(&uuid) == Some(&hu));
        let near = h.dimension == me.dimension && (0..3).map(|i| (h.pos[i] - me.position[i]).powi(2)).sum::<f64>() <= BAG_REACH * BAG_REACH;
        if !on_it && !near {
            return Self::bar(host, uuid, "Сумки на коні: підійди до нього або сядь верхи", Tone::Bad);
        }
        let size = usize::from(rows) * 9;
        let items: Vec<Option<ItemStack>> = (0..size)
            .map(|i| bag.get(i).filter(|s| *s != "-").and_then(|s| horizon_client::item_data::parse_stack(s).ok().flatten()))
            .collect();
        if let Some(window) = host.open_menu_with_input(uuid, rows, Text::literal(BAG_TITLE.to_owned()), items, 0..size) {
            self.rides().bags.insert(uuid, window);
        }
    }

    /// The saddlebags closed: what is in them stays in them.
    pub(crate) fn bags_closed(&self, uuid: u128, window: i32, slots: &mut [Option<ItemStack>]) {
        {
            let mut r = self.rides();
            if r.bags.get(&uuid) != Some(&window) {
                return;
            }
            r.bags.remove(&uuid);
        }
        // Taken: nothing goes back to the player.
        let texts: Vec<String> = slots.iter_mut().map(|s| horizon_client::item_data::canon_stack(s.take().as_ref())).collect();
        let effects = self.engine().as_mut().map(|e| e.set_bag(uuid, texts)).unwrap_or_default();
        relock(&self.queue).effects.extend(effects);
    }

    /// The damage hook's part: a wall that doesn't hurt ("М'яка посадка").
    pub(crate) fn travel_spares(&self, entity: i32, damage_type: &str) -> bool {
        let Ok(r) = self.rides.try_lock() else {
            return false;
        };
        damage_type == "minecraft:fly_into_wall" && r.walls.contains(&entity)
    }

    /// A glide started or ended: "Планер"'s lighter gravity while it lasts.
    pub(crate) fn glide(&self, uuid: u128, gliding: bool) {
        let gravity = {
            let mut r = self.rides();
            if gliding {
                r.gliding.insert(uuid);
            } else {
                r.gliding.remove(&uuid);
            }
            r.travel.get(&uuid).map_or(0.0, |t| t.glide)
        };
        if gravity < 0.0 || !gliding {
            let add = (gliding && gravity < 0.0).then(|| (gravity, "add_multiplied_base".to_owned()));
            horizon_client::progression_attrs::push(uuid, "minecraft:gravity", "progression:glide", add);
        }
    }
}
