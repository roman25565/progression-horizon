//! The rules of the mod, with no game in them: what a join, a kill, a click or a key does to a
//! player's progress, and what the game is then asked to do ([`Effect`]).

use super::config::{ALTARS, Book, CONFIG_VERSION, Config, DEFAULT_CONFIG, Perk, Rewards, altar_group, altar_of, file_version, rewardable};
use super::gui::{self, Cell, Frame, MenuView};
use super::store::{Anchor, Body, Draft, Horse, PlayerData, Store, Zone, uuid_hex};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Info,
    Good,
    Bad,
}

/// An item the mod hands out.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// The toolkit's (kirca) 3x3 tools.
    Pickaxe3,
    Shovel3,
    Book { enchant: String, level: u32 },
    /// "Дистанційний сигнал": a transmitter with no targets.
    Transmitter,
}

/// What the game is asked to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    Tell(u128, String, Tone),
    ActionBar(u128, String),
    Broadcast(String),
    Console(String),
    /// A console command.
    Command(String),
    /// Player, attribute, modifier id, and the amount and operation (None takes it away).
    Attribute(u128, String, String, Option<(f64, String)>),
    /// Player, name, `dash_forward` or `dash_up`.
    Dash(u128, String, &'static str),
    /// The client-side perks of a Horizon client.
    ClientPerks(u128, Vec<String>),
    /// Give the item, then take the vanilla levels; nothing is taken when it doesn't fit.
    Sell { uuid: u128, levels: u32, item: Item },
    TakeLevels(u128, u32),
    /// Hold (or let go of) chunks for `owner`.
    Anchor { owner: String, dimension: String, chunks: Vec<[i32; 2]>, hold: bool },
    /// Fill these empty places with the block in the player's off hand ("Кресляр" II).
    Fill { uuid: u128, dimension: String, blocks: Vec<[i32; 3]> },
}

/// What the server sees of an online player.
#[derive(Clone, Debug, Default)]
pub struct Seen {
    pub uuid: u128,
    pub name: String,
    pub dimension: String,
    pub position: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    /// 0 survival, 1 creative, 2 adventure, 3 spectator.
    pub game_mode: i32,
    /// Vanilla experience levels.
    pub xp_level: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Perks,
    /// The travel branches: minecart, horse, elytra.
    Travel,
    Altar,
    Respec,
    /// The redstone lab's settings.
    Lab,
    /// The drawing's shapes ("Кресляр").
    Draft,
}

impl Page {
    pub fn rows(self) -> u8 {
        match self {
            Page::Respec | Page::Lab | Page::Draft => 3,
            Page::Perks | Page::Travel | Page::Altar => 6,
        }
    }
}

/// What a key or a dialog button did.
#[derive(Default)]
pub struct Acted {
    pub effects: Vec<Effect>,
    pub open: Option<Page>,
}

/// What a menu click did.
#[derive(Default)]
pub struct Clicked {
    pub effects: Vec<Effect>,
    pub show: Option<Page>,
    pub close: bool,
}

pub const SLOT_INFO: usize = 45;
pub const SLOT_KEYS: usize = 49;
pub const SLOT_RESPEC: usize = 52;
/// The other page of perks (the main branches or the travel ones).
pub const SLOT_PAGE: usize = 47;
pub const SLOT_CLOSE: usize = 53;
pub const SLOT_YES: usize = 11;
pub const SLOT_NO: usize = 15;
/// The altar's books: rows 1 to 4.
pub const BOOK_FIRST: usize = 9;
/// The lab menu: put the zone here, blasts, observers, take it away, close.
pub const LAB_HERE: usize = 10;
pub const LAB_BLAST: usize = 12;
pub const LAB_FREEZE: usize = 14;
pub const LAB_REMOVE: usize = 16;
pub const LAB_CLOSE: usize = 26;
/// The drawing menu: three shapes, fill, clear, close.
pub const DRAFT_LINE: usize = 10;
pub const DRAFT_BOX: usize = 11;
pub const DRAFT_CIRCLE: usize = 12;
pub const DRAFT_FILL: usize = 14;
pub const DRAFT_CLEAR: usize = 16;
pub const DRAFT_CLOSE: usize = 26;
/// The most points of a drawing the particles show (spread evenly over it).
pub const DRAFT_SHOWN: usize = 160;

/// Effects that help: what the alchemist makes last longer.
pub const BENEFICIAL: [&str; 17] = [
    "minecraft:speed", "minecraft:haste", "minecraft:strength", "minecraft:instant_health", "minecraft:jump_boost",
    "minecraft:regeneration", "minecraft:resistance", "minecraft:fire_resistance", "minecraft:water_breathing",
    "minecraft:invisibility", "minecraft:night_vision", "minecraft:health_boost", "minecraft:absorption",
    "minecraft:saturation", "minecraft:slow_falling", "minecraft:conduit_power", "minecraft:dolphins_grace",
];

/// What "clear mind" takes off at each tier (each tier keeps the ones before).
pub const CLEAR_MIND: [&[&str]; 3] = [
    &["minecraft:nausea", "minecraft:blindness", "minecraft:darkness"],
    &["minecraft:poison", "minecraft:hunger", "minecraft:weakness"],
    &["minecraft:wither", "minecraft:slowness", "minecraft:mining_fatigue"],
];

/// An effect a player with `tier` alchemist tiers just got: how long (and how strong) it should
/// be instead. None when it stays as it is. Tier 1 makes it last half again, tier 2 twice as
/// long, tier 3 also a level stronger (up to IV).
pub fn alchemy(tier: u32, id: &str, amplifier: u8, duration: i32) -> Option<(u8, i32)> {
    if tier == 0 || !BENEFICIAL.contains(&id) || duration <= 0 {
        return None;
    }
    // Instant effects have no length; an endless one stays endless.
    let instant = id == "minecraft:instant_health" || id == "minecraft:saturation";
    let longer = if instant { duration } else { (duration as i64 * if tier >= 2 { 2 } else { 3 } / if tier >= 2 { 1 } else { 2 }).min(20 * 60 * 60 * 8) as i32 };
    let stronger = if tier >= 3 && amplifier < 3 { amplifier + 1 } else { amplifier };
    (longer != duration || stronger != amplifier).then_some((stronger, longer))
}

/// The logs of a tree from `start` (a log): those touching it, sides and corners, out to `max`
/// blocks, nearest first. `is_log` says what a position holds.
pub fn fell_tree(start: [i32; 3], max: usize, mut is_log: impl FnMut([i32; 3]) -> bool) -> Vec<[i32; 3]> {
    let mut seen = HashSet::from([start]);
    let mut queue = std::collections::VecDeque::from([start]);
    let mut out = Vec::new();
    while let Some(p) = queue.pop_front() {
        if out.len() >= max {
            break;
        }
        if p != start {
            out.push(p);
        }
        for dy in 0..=1 {
            for dx in -1..=1 {
                for dz in -1..=1 {
                    let q = [p[0] + dx, p[1] + dy, p[2] + dz];
                    // Only up and sideways from the cut: the stump below stays.
                    if q[1] < start[1] || !seen.insert(q) {
                        continue;
                    }
                    // Keep to the tree: at most 12 blocks out from the trunk.
                    if (q[0] - start[0]).abs() > 12 || (q[2] - start[2]).abs() > 12 {
                        continue;
                    }
                    if is_log(q) {
                        queue.push_back(q);
                    }
                }
            }
        }
    }
    out
}

/// A log (or a nether stem) a lumberjack fells.
pub fn is_log_block(id: &str) -> bool {
    let path = id.split_once(':').map_or(id, |(_, p)| p);
    (path.ends_with("_log") || path.ends_with("_stem") || path.ends_with("_wood") || path.ends_with("_hyphae"))
        && !path.starts_with("stripped_")
}

pub fn roman(n: u32) -> String {
    const R: [&str; 11] = ["0", "I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX", "X"];
    R.get(n as usize).map_or_else(|| n.to_string(), |s| (*s).to_owned())
}

/// A name that is safe inside a command line (vanilla's player names).
pub fn safe_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 16 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn f3(v: f64) -> String {
    format!("{v:.3}")
}

/// What sets `perk` at `tier` for the player (tier 0 takes it away).
pub fn perk_effects(uuid: u128, perk: &Perk, tier: u32) -> Vec<Effect> {
    let mut out = Vec::new();
    if let Some(attr) = &perk.attribute {
        let add = (tier > 0).then(|| (perk.per_level * f64::from(tier), perk.operation.clone()));
        out.push(Effect::Attribute(uuid, attr.clone(), perk.modifier_id(), add));
    }
    for (n, x) in perk.also.iter().enumerate() {
        let add = (tier > 0).then(|| (x.per_level * f64::from(tier), x.operation.clone()));
        out.push(Effect::Attribute(uuid, x.attribute.clone(), perk.extra_id(n), add));
    }
    out
}

/// A UUID as NBT's int array (`[I; a, b, c, d]`).
fn uuid_nbt(uuid: u128) -> String {
    let p = |i: u32| ((uuid >> (96 - 32 * i)) as u32) as i32;
    format!("[I;{},{},{},{}]", p(0), p(1), p(2), p(3))
}

/// The wind-charge trick: a charge of the player's own (so it never hits them) goes into the
/// ground under them (`dash_up`) or a block behind them (`dash_forward`) and bursts there; the
/// burst throws them away from it, as a thrown wind charge does, without fall damage.
pub fn dash_command(name: &str, uuid: u128, id: &str) -> Option<String> {
    if !safe_name(name) {
        return None;
    }
    let charge = format!(
        "summon minecraft:wind_charge ~ ~0.05 ~ {{Motion:[0.0d,-1.0d,0.0d],Owner:{}}}",
        uuid_nbt(uuid)
    );
    match id {
        "dash_up" => Some(format!("execute as {name} at @s run {charge}")),
        "dash_forward" => Some(format!("execute as {name} at @s rotated ~ 0 positioned ^ ^ ^-1 run {charge}")),
        _ => None,
    }
}

/// What a key (or a dialog button, or a command) can ask for.
pub const ACTIONS: &[&str] = &[
    "perks", "travel", "respec", "dash_forward", "dash_up", "spirit", "anchor", "lab", "draft", "altar", "keys", "horse", "bags",
];

/// The four protections, which the altar's anvil lets share one piece of armour.
pub const PROTECTIONS: [&str; 4] = [
    "minecraft:protection",
    "minecraft:fire_protection",
    "minecraft:blast_protection",
    "minecraft:projectile_protection",
];

/// The enchantments an anvil result should carry, given both inputs and vanilla's result: no
/// level is ever lower than on either input (vanilla cuts levels above its top), and a
/// protection from the right input joins the other protections on the left one. None when
/// vanilla's result already is that.
pub fn anvil_levels(left: &[(String, u32)], right: &[(String, u32)], vanilla: &[(String, u32)]) -> Option<Vec<(String, u32)>> {
    let level = |list: &[(String, u32)], id: &str| list.iter().find(|(e, _)| e == id).map_or(0, |(_, l)| *l);
    let mut out: Vec<(String, u32)> = vanilla
        .iter()
        .map(|(e, l)| (e.clone(), (*l).max(level(left, e)).max(level(right, e))))
        .collect();
    let armoured = left.iter().any(|(e, _)| PROTECTIONS.contains(&e.as_str()));
    if armoured {
        for (e, l) in right {
            if PROTECTIONS.contains(&e.as_str()) && !out.iter().any(|(o, _)| o == e) {
                out.push((e.clone(), (*l).max(level(left, e))));
            }
        }
    }
    (out.as_slice() != vanilla).then_some(out)
}

/// Enchantments that don't share an item in vanilla (the protections aside, which do here).
const EXCLUSIVE: [&[&str]; 6] = [
    &["minecraft:sharpness", "minecraft:smite", "minecraft:bane_of_arthropods", "minecraft:density", "minecraft:breach", "minecraft:impaling"],
    &["minecraft:silk_touch", "minecraft:fortune"],
    &["minecraft:silk_touch", "minecraft:looting"],
    &["minecraft:infinity", "minecraft:mending"],
    &["minecraft:riptide", "minecraft:loyalty", "minecraft:channeling"],
    &["minecraft:multishot", "minecraft:piercing"],
];

fn clash(a: &str, b: &str) -> bool {
    if a == "minecraft:riptide" || b == "minecraft:riptide" {
        // Loyalty and channeling do share a trident.
        return a != b && EXCLUSIVE[4].contains(&a) && EXCLUSIVE[4].contains(&b);
    }
    if (a == "minecraft:loyalty" && b == "minecraft:channeling") || (a == "minecraft:channeling" && b == "minecraft:loyalty") {
        return false;
    }
    if (a == "minecraft:depth_strider" && b == "minecraft:frost_walker") || (a == "minecraft:frost_walker" && b == "minecraft:depth_strider") {
        return true;
    }
    a != b && EXCLUSIVE.iter().any(|g| g.contains(&a) && g.contains(&b))
}

/// The result of an anvil vanilla calls too expensive (it gives none at 40 levels): the left
/// item's enchantments with the right one's worked in. An enchantment the left item already has
/// rises to the higher level, or by one for equal levels while that stays within `top` (its
/// vanilla top level); a new one joins only from the same kind of item (`same_item`), where it
/// surely fits, and when nothing on the left clashes with it. None when nothing would change.
pub fn anvil_fallback(left: &[(String, u32)], right: &[(String, u32)], same_item: bool, top: &dyn Fn(&str) -> u32) -> Option<Vec<(String, u32)>> {
    let mut out = left.to_vec();
    for (e, l) in right {
        if let Some(at) = out.iter().position(|(o, _)| o == e) {
            let cur = out[at].1;
            out[at].1 = if cur == *l && cur < top(e) { cur + 1 } else { cur.max(*l) };
        } else if same_item && !out.iter().any(|(o, _)| clash(o, e)) {
            out.push((e.clone(), *l));
        }
    }
    (out.as_slice() != left).then_some(out)
}

/// The lab's half side at `tier`: `amount`, then twice, then four times.
pub fn lab_radius(p: &Perk, tier: u32) -> i32 {
    if tier == 0 {
        return 0;
    }
    ((p.amount.max(1.0)) * f64::from(1u32 << (tier.min(4) - 1))).round() as i32
}

/// Logs one cut fells at `tier`.
/// How many blocks one fill of "Кресляр" may place: none at I, `amount` at II, eight times as
/// many at III.
pub fn fill_limit(p: &Perk, tier: u32) -> usize {
    if tier < 2 {
        return 0;
    }
    (p.amount.max(1.0) * 8f64.powi(tier.min(4) as i32 - 2)).round() as usize
}

/// Where a branch's perks stand on the map, from its badge: one star after another.
const STAR_STEPS: [(f32, f32); 8] =
    [(-0.7, -1.3), (0.8, -2.0), (-0.4, -3.0), (0.9, -3.8), (-0.9, -4.5), (0.3, -5.4), (1.1, -6.1), (-0.6, -6.8)];

/// The branches a page of the perk menu shows, a row each.
pub fn page_branches(page: Page) -> &'static [&'static str] {
    let all = &super::config::BRANCHES;
    match page {
        Page::Travel => &all[super::config::MAIN_BRANCHES..],
        _ => &all[..super::config::MAIN_BRANCHES],
    }
}

/// Track steps a ridden minecart makes a tick with "Швидкі колії" at `tier`: each is vanilla's
/// own (at most 0.4 blocks), so the cart stays on its rails in the curves.
pub fn cart_steps(tier: u32) -> u32 {
    match tier {
        0 => 1,
        1..=3 => 1 + tier,
        _ => 6,
    }
}

/// Every modifier the horse perks put on a ridden horse: attribute and modifier id.
pub const HORSE_MODIFIERS: [(&str, &str); 8] = [
    ("minecraft:movement_speed", "progression:horse_speed"),
    ("minecraft:gravity", "progression:horse_gravity"),
    ("minecraft:jump_strength", "progression:horse_jump"),
    ("minecraft:fall_damage_multiplier", "progression:horse_fall"),
    ("minecraft:safe_fall_distance", "progression:horse_safe_fall"),
    ("minecraft:step_height", "progression:horse_step"),
    ("minecraft:water_movement_efficiency", "progression:horse_water"),
    ("minecraft:movement_efficiency", "progression:horse_ground"),
];

/// The modifiers a horse gets under a rider with these tiers: attribute, modifier id, amount,
/// operation. "Скакун": `speed_per` of the base speed a tier; "Впевнений спуск": six times the
/// gravity at I, the most the attribute takes (x12.5) at II (falls and slopes take less time, so it keeps its speed), a jump that
/// still clears the same height, and no fall damage at II; "Прохідливість": half a block of
/// step a tier (two blocks at II), water and soul sand slow it less.
pub fn horse_modifiers(speed: u32, descent: u32, terrain: u32, speed_per: f64) -> Vec<(&'static str, &'static str, f64, &'static str)> {
    let mut out = Vec::new();
    let m = |i: usize| HORSE_MODIFIERS[i];
    if speed > 0 {
        out.push((m(0).0, m(0).1, speed_per / 100.0 * f64::from(speed.min(4)), "add_multiplied_base"));
    }
    // "Впевнений спуск" sets no modifier: Horizon doesn't move a mob by its gravity or fall
    // attributes (the rider's client takes only speed, jump and health), so serverkeys makes the
    // driven horse heavier on the client (`horse_fall_times`).
    let _ = descent;
    if terrain > 0 {
        let t = f64::from(terrain.min(2));
        out.push((m(5).0, m(5).1, 0.5 * t, "add_value"));
        out.push((m(6).0, m(6).1, 0.5 * t, "add_value"));
        out.push((m(7).0, m(7).1, 0.5 * t, "add_value"));
    }
    out
}

/// "Впевнений спуск": how many times faster the horse falls at `tier` (serverkeys' client side).
pub fn horse_fall_times(tier: u32) -> u32 {
    match tier {
        0 => 1,
        1 => 10,
        _ => 100,
    }
}

/// The fastest a bred horse can be: twice vanilla's 0.3375 (about 29 m/s instead of 14).
pub const FOAL_SPEED_MAX: f64 = 0.675;

/// A foal's speed from its parents', as vanilla's `AbstractHorse.createOffspringAttribute` with
/// the top at [`FOAL_SPEED_MAX`]: their mean, give or take their gap and a margin, bounced back
/// in at the ends. `quality` is vanilla's (three randoms averaged) - 0.5.
pub fn foal_speed(a: f64, b: f64, quality: f64) -> f64 {
    let (min, max) = (0.1125, FOAL_SPEED_MAX);
    let (a, b) = (a.clamp(min, max), b.clamp(min, max));
    // Vanilla's margin (15% of its own range), so a generation moves as far as there.
    let margin = 0.15 * (0.3375 - 0.1125);
    let v = (a + b) / 2.0 + ((a - b).abs() + margin * 2.0) * quality.clamp(-0.5, 0.5);
    if v > max {
        max - (v - max)
    } else if v < min {
        min + (min - v)
    } else {
        v
    }
}

/// A horse's movement speed and jump strength as players know them: metres a second at a
/// gallop, and the height in blocks it clears (the wiki's fit of vanilla's jump).
pub fn horse_numbers(speed: f64, jump: f64) -> (f64, f64) {
    let height = -0.181_758_495_2 * jump.powi(3) + 3.689_713_992 * jump.powi(2) + 2.128_599_134 * jump - 0.343_930_367;
    (speed * 42.16, height.max(0.0))
}

/// The places "Крізь хащі" clears for a horse at `pos` going towards `yaw` (degrees, as
/// Minecraft's: 0 is +z, 90 is -x): from its feet to above its rider's head, a block either
/// side, from under it to three blocks ahead.
pub fn leaf_box(pos: [f64; 3], yaw: f32) -> Vec<[i32; 3]> {
    let r = f64::from(yaw).to_radians();
    let (fx, fz) = (-r.sin(), r.cos());
    let (sx, sz) = (fz, -fx);
    let mut out = Vec::new();
    for ahead in 0..=6 {
        let a = f64::from(ahead) * 0.5;
        for side in -2..=2 {
            let s = f64::from(side) * 0.5;
            let x = (pos[0] + fx * a + sx * s).floor() as i32;
            let z = (pos[2] + fz * a + sz * s).floor() as i32;
            for dy in 0..=3 {
                let p = [x, pos[1].floor() as i32 + dy, z];
                if !out.contains(&p) {
                    out.push(p);
                }
            }
        }
    }
    out
}

/// Leaves of any tree (and azalea's).
pub fn is_leaves(block: &str) -> bool {
    let id = block.split('[').next().unwrap_or(block);
    id.ends_with("_leaves")
}

/// The kinds of horse the horse perks work on.
pub const HORSES: [&str; 6] = [
    "minecraft:horse",
    "minecraft:donkey",
    "minecraft:mule",
    "minecraft:skeleton_horse",
    "minecraft:zombie_horse",
    "minecraft:camel",
];

/// Rows of the saddlebags at "Сідельні сумки" `tier`: three, six at II.
pub fn bag_rows(tier: u32) -> u8 {
    (3 * tier.min(2)) as u8
}

/// The gravity modifier while gliding with "Планер" at `tier`: 15% less a tier.
pub fn glide_gravity(tier: u32) -> f64 {
    -0.15 * f64::from(tier.min(3))
}

/// What the travel perks let a player do now.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Travel {
    /// Track steps a tick of a minecart they ride (1: vanilla).
    pub cart_steps: u32,
    /// Forward speeds their minecart up on any rails.
    pub cart_drive: bool,
    /// Their minecart goes back to their inventory when they get out.
    pub cart_return: bool,
    /// The modifiers of a horse they ride.
    pub horse: Vec<(&'static str, &'static str, f64, &'static str)>,
    pub call: bool,
    pub bag_rows: u8,
    /// Chances (0 to 1): the elytra keeps its durability each second, a rocket comes back.
    pub wear: f64,
    pub rockets: f64,
    /// No damage from flying into a wall.
    pub wall: bool,
    /// The gravity modifier while gliding (0: none).
    pub glide: f64,
    /// Leaves in front of their ridden horse go away ("Крізь хащі").
    pub leaves: bool,
}

/// What "Дистанційний сигнал" lets transmitters have at `tier`: `amount` targets at I, four times
/// more a tier, at any distance in the same world (0: no limit). None without the perk.
pub fn relay_limits(p: &Perk, tier: u32) -> Option<(usize, i32)> {
    if tier == 0 {
        return None;
    }
    let times = 4f64.powi(tier.min(4) as i32 - 1);
    Some(((p.amount.max(1.0) * times).round() as usize, 0))
}

/// The places of a drawing between `a` and `b`, at most `max` (one more tells it is bigger).
/// `line`: a straight line; `box`: the edges of the box (`filled`: all of it); `circle`: a
/// ring around `a` in its layer through `b`'s distance (`filled`: the disc).
pub fn draft_shape(shape: &str, a: [i32; 3], b: [i32; 3], filled: bool, max: usize) -> Vec<[i32; 3]> {
    let mut out: Vec<[i32; 3]> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut put = |p: [i32; 3], out: &mut Vec<[i32; 3]>| {
        if out.len() <= max && seen.insert(p) {
            out.push(p);
        }
    };
    match shape {
        "box" => {
            let lo = [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])];
            let hi = [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])];
            if filled {
                'all: for y in lo[1]..=hi[1] {
                    for x in lo[0]..=hi[0] {
                        for z in lo[2]..=hi[2] {
                            put([x, y, z], &mut out);
                            if out.len() > max {
                                break 'all;
                            }
                        }
                    }
                }
            } else {
                // The twelve edges.
                for (i, j, k) in [(0, 1, 2), (1, 0, 2), (2, 0, 1)] {
                    for vj in [lo[j], hi[j]] {
                        for vk in [lo[k], hi[k]] {
                            for vi in lo[i]..=hi[i] {
                                let mut p = [0; 3];
                                p[i] = vi;
                                p[j] = vj;
                                p[k] = vk;
                                put(p, &mut out);
                                if out.len() > max {
                                    return out;
                                }
                            }
                        }
                    }
                }
            }
        }
        "circle" => {
            let (dx, dz) = (f64::from(b[0] - a[0]), f64::from(b[2] - a[2]));
            let r = dx.hypot(dz).round().min(256.0);
            let ri = r as i32;
            for x in -ri..=ri {
                for z in -ri..=ri {
                    let d = f64::from(x).hypot(f64::from(z));
                    let take = if filled { d <= r + 0.5 } else { (d - r).abs() < 0.5 };
                    if take {
                        put([a[0] + x, a[1], a[2] + z], &mut out);
                        if out.len() > max {
                            return out;
                        }
                    }
                }
            }
        }
        _ => {
            let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let n = d.iter().map(|v| v.abs()).max().unwrap_or(0);
            for i in 0..=n {
                let at = |k: usize| a[k] + if n == 0 { 0 } else { (f64::from(d[k]) * f64::from(i) / f64::from(n)).round() as i32 };
                put([at(0), at(1), at(2)], &mut out);
                if out.len() > max {
                    break;
                }
            }
        }
    }
    out
}

/// The order a fill places its blocks in: chunk by chunk, so each tick touches few chunks, and
/// from the top down in each, so the sky light goes dark once instead of block by block.
pub fn fill_order(blocks: &mut [[i32; 3]]) {
    blocks.sort_by_key(|p| (p[0] >> 4, p[2] >> 4, -p[1], p[2], p[0]));
}

/// Places in a row along x (as `fill_order` leaves them) as (from, to) runs: one command each.
pub fn runs(blocks: &[[i32; 3]]) -> Vec<([i32; 3], [i32; 3])> {
    let mut out: Vec<([i32; 3], [i32; 3])> = Vec::new();
    for p in blocks {
        match out.last_mut() {
            Some((_, to)) if to[1] == p[1] && to[2] == p[2] && to[0] + 1 == p[0] => *to = *p,
            _ => out.push((*p, *p)),
        }
    }
    out
}

/// Every `len / most`-th of `points`, so a big drawing still shows as a whole.
pub fn spread<T: Copy>(points: &[T], most: usize) -> Vec<T> {
    if points.len() <= most || most == 0 {
        return points.to_vec();
    }
    (0..most).map(|i| points[i * points.len() / most]).collect()
}

/// Stone families the "Каменяр" turns one into the next of, as the stonecutter would (and the
/// cracked and mossy kinds, which builders want too).
pub const MASON_FAMILIES: &[&[&str]] = &[
    &["stone", "smooth_stone", "stone_bricks", "cracked_stone_bricks", "mossy_stone_bricks", "chiseled_stone_bricks"],
    &["cobblestone", "mossy_cobblestone"],
    &["granite", "polished_granite"],
    &["diorite", "polished_diorite"],
    &["andesite", "polished_andesite"],
    &["deepslate", "cobbled_deepslate", "polished_deepslate", "deepslate_bricks", "cracked_deepslate_bricks", "deepslate_tiles", "cracked_deepslate_tiles", "chiseled_deepslate"],
    &["blackstone", "polished_blackstone", "polished_blackstone_bricks", "cracked_polished_blackstone_bricks", "chiseled_polished_blackstone", "gilded_blackstone"],
    &["sandstone", "cut_sandstone", "chiseled_sandstone", "smooth_sandstone"],
    &["red_sandstone", "cut_red_sandstone", "chiseled_red_sandstone", "smooth_red_sandstone"],
    &["packed_mud", "mud_bricks"],
    &["prismarine", "prismarine_bricks", "dark_prismarine"],
    &["quartz_block", "chiseled_quartz_block", "quartz_pillar", "quartz_bricks", "smooth_quartz"],
    &["purpur_block", "purpur_pillar"],
    &["end_stone", "end_stone_bricks"],
    &["nether_bricks", "cracked_nether_bricks", "chiseled_nether_bricks", "red_nether_bricks"],
    &["tuff", "polished_tuff", "tuff_bricks", "chiseled_tuff", "chiseled_tuff_bricks"],
    &["basalt", "polished_basalt", "smooth_basalt"],
    &["resin_bricks", "chiseled_resin_bricks"],
    &["bricks"],
    &["copper_block", "cut_copper", "chiseled_copper", "copper_grate"],
    &["exposed_copper", "exposed_cut_copper", "exposed_chiseled_copper", "exposed_copper_grate"],
    &["weathered_copper", "weathered_cut_copper", "weathered_chiseled_copper", "weathered_copper_grate"],
    &["oxidized_copper", "oxidized_cut_copper", "oxidized_chiseled_copper", "oxidized_copper_grate"],
];

/// Full blocks with an `axis`: their state carries over between them.
const AXIS_BLOCKS: &[&str] = &["deepslate", "basalt", "polished_basalt", "quartz_pillar", "purpur_pillar"];

/// The stairs, slab or wall made of a family's block: `stone_bricks` -> `stone_brick`.
fn stem(block: &str) -> String {
    if let Some(s) = block.strip_suffix("_bricks") {
        return format!("{s}_brick");
    }
    if let Some(s) = block.strip_suffix("_tiles") {
        return format!("{s}_tile");
    }
    if block == "bricks" {
        return "brick".to_owned();
    }
    match block.strip_suffix("_block") {
        Some(s) if !s.ends_with("copper") => s.to_owned(),
        _ => block.to_owned(),
    }
}

/// What kind of shape a block is: `stairs`, `slab`, `wall` or `block`.
pub fn shape_of(id: &str) -> &'static str {
    let path = id.split_once(':').map_or(id, |(_, p)| p);
    if path.ends_with("_stairs") {
        "stairs"
    } else if path.ends_with("_slab") {
        "slab"
    } else if path.ends_with("_wall") {
        "wall"
    } else {
        "block"
    }
}

/// The next stone of `state`'s family in the same shape (`minecraft:stone_brick_stairs[...]` ->
/// `minecraft:mossy_stone_brick_stairs[...]`), skipping the ones `exists` doesn't know. The
/// properties go along when the shape keeps them.
pub fn mason_next(state: &str, exists: impl Fn(&str) -> bool) -> Option<String> {
    let (id, props) = match state.split_once('[') {
        Some((id, rest)) => (id, Some(rest.trim_end_matches(']'))),
        None => (state, None),
    };
    let path = id.strip_prefix("minecraft:")?;
    let shape = shape_of(path);
    let named = |block: &str| -> String {
        match shape {
            "block" => block.to_owned(),
            s => format!("{}_{s}", stem(block)),
        }
    };
    let family = MASON_FAMILIES.iter().find(|f| f.iter().any(|b| named(b) == path))?;
    let here = family.iter().position(|b| named(b) == path)?;
    for step in 1..family.len() {
        let next = named(family[(here + step) % family.len()]);
        let full = format!("minecraft:{next}");
        if !exists(&full) {
            continue;
        }
        let keep = match shape {
            "block" => AXIS_BLOCKS.contains(&path) && AXIS_BLOCKS.contains(&next.as_str()),
            _ => true,
        };
        return Some(match props {
            Some(p) if keep && !p.is_empty() => format!("{full}[{p}]"),
            _ => full,
        });
    }
    None
}

/// Whether "Кельма" may put `to` where `from` was placed: the same shape; the state's
/// properties go along unless they are full blocks of different kinds.
pub fn trowel_state(from_state: &str, to: &str) -> Option<String> {
    let (from, props) = match from_state.split_once('[') {
        Some((id, rest)) => (id, Some(rest.trim_end_matches(']'))),
        None => (from_state, None),
    };
    if from == to || shape_of(from) != shape_of(to) {
        return None;
    }
    let path = |s: &str| s.split_once(':').map_or(s.to_owned(), |(_, p)| p.to_owned());
    let pillar = |s: &str| {
        let s = path(s);
        AXIS_BLOCKS.contains(&s.as_str()) || ["_log", "_wood", "_stem", "_hyphae"].iter().any(|e| s.ends_with(e))
    };
    let keep = shape_of(from) != "block" || (pillar(from) && pillar(to));
    Some(match props {
        Some(p) if keep && !p.is_empty() => format!("{to}[{p}]"),
        _ => to.to_owned(),
    })
}

pub fn timber_logs(p: &Perk, tier: u32) -> u32 {
    (p.amount.max(1.0) * f64::from(tier)).round() as u32
}

/// Seconds a rescue perk waits at `tier`: its cooldown, halved by each tier past the first.
pub fn rescue_seconds(p: &Perk, tier: u32) -> u32 {
    (p.cooldown_seconds >> tier.saturating_sub(1).min(8)).max(5)
}

/// An effect's name for the menus.
pub fn effect_name(id: &str) -> &str {
    match id {
        "minecraft:nausea" => "нудота",
        "minecraft:blindness" => "сліпота",
        "minecraft:darkness" => "темрява",
        "minecraft:poison" => "отрута",
        "minecraft:hunger" => "голод",
        "minecraft:weakness" => "слабкість",
        "minecraft:wither" => "іссушення",
        "minecraft:slowness" => "сповільнення",
        "minecraft:mining_fatigue" => "втома",
        "minecraft:speed" => "швидкість",
        "minecraft:haste" => "квапливість",
        "minecraft:strength" => "сила",
        "minecraft:jump_boost" => "стрибучість",
        "minecraft:regeneration" => "регенерація",
        "minecraft:resistance" => "опір",
        "minecraft:fire_resistance" => "вогнестійкість",
        "minecraft:water_breathing" => "підводне дихання",
        "minecraft:invisibility" => "невидимість",
        "minecraft:night_vision" => "нічне бачення",
        "minecraft:health_boost" => "приплив здоров'я",
        "minecraft:absorption" => "поглинання",
        "minecraft:slow_falling" => "повільне падіння",
        "minecraft:conduit_power" => "сила провідника",
        "minecraft:dolphins_grace" => "грація дельфіна",
        other => other,
    }
}

/// `s` with its first letter in upper case.
fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

/// Words into lines of at most `max` characters.
pub fn wrap(s: &str, max: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in s.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > max {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

fn body_tag(uuid: u128) -> String {
    format!("pb_{uuid:032x}")
}

fn anchor_owner(uuid: u128) -> String {
    format!("progression:anchor/{}", uuid_hex(uuid))
}

pub struct Engine {
    pub config: Config,
    rewards: Rewards,
    store: Store,
    dir: PathBuf,
    players: HashMap<u128, PlayerData>,
    dirty: HashSet<u128>,
    /// (player, skill) -> the tick it can be used again.
    ready_at: HashMap<(u128, String), u64>,
    /// Player -> the tick they last took damage.
    hurt_at: HashMap<u128, u64>,
}

fn load_files(dir: &Path, log: &mut Vec<String>) -> (Config, Rewards) {
    let path = dir.join("progression.toml");
    if !path.exists() {
        let made = std::fs::create_dir_all(dir).and_then(|()| std::fs::write(&path, DEFAULT_CONFIG));
        match made {
            Ok(()) => log.push(format!("[Progression] wrote the default settings to {}", path.display())),
            Err(e) => log.push(format!("[Progression] can't write {}: {e}", path.display())),
        }
    }
    let default = || Config::parse(DEFAULT_CONFIG).expect("the built-in settings are valid");
    // Settings of an older version (0.x chains, 1.0 perks) are kept aside for new default ones.
    if let Ok(text) = std::fs::read_to_string(&path)
        && file_version(&text).is_some_and(|v| v < CONFIG_VERSION)
    {
        let old = file_version(&text).unwrap_or(0);
        let kept = dir.join(format!("progression-v{old}.toml"));
        match std::fs::rename(&path, &kept).and_then(|()| std::fs::write(&path, DEFAULT_CONFIG)) {
            Ok(()) => log.push(format!(
                "[Progression] the settings were of an older version: kept as {}, new default ones written to {}",
                kept.display(),
                path.display()
            )),
            Err(e) => log.push(format!("[Progression] can't replace the old settings {}: {e}", path.display())),
        }
    }
    let config = match std::fs::read_to_string(&path) {
        Ok(text) => Config::parse(&text).unwrap_or_else(|e| {
            log.push(format!("[Progression] {e}; using the built-in settings until it is fixed"));
            default()
        }),
        Err(_) => default(),
    };
    let rewards = match std::fs::read_to_string(dir.join("advancements.toml")) {
        Ok(text) => Rewards::parse(&text).unwrap_or_else(|e| {
            log.push(format!("[Progression] {e}"));
            Rewards::default()
        }),
        Err(_) => Rewards::default(),
    };
    (config, rewards)
}

impl Engine {
    /// Opens `dir` (`config/progression`): writes the default settings on the first start, reads
    /// them and the advancement weights. Bad settings fall back to the defaults, with the error
    /// in the returned lines.
    pub fn open(dir: &Path) -> (Engine, Vec<String>) {
        let mut log = Vec::new();
        let (config, rewards) = load_files(dir, &mut log);
        let e = Engine {
            config,
            rewards,
            store: Store::new(dir),
            dir: dir.to_path_buf(),
            players: HashMap::new(),
            dirty: HashSet::new(),
            ready_at: HashMap::new(),
            hurt_at: HashMap::new(),
        };
        log.push(format!(
            "[Progression] {} perks, {} books, {} advancement weights; the tree costs {} points, level {} gives {}",
            e.config.perks.len(),
            e.config.books.len(),
            e.rewards.points.len(),
            e.config.tree_cost(),
            e.config.settings.max_level,
            e.config.points_by(e.config.settings.max_level),
        ));
        (e, log)
    }

    /// Reads the files again (`/progressadmin reload`) and gives every online player their perks
    /// as the new settings make them.
    pub fn reload(&mut self, online: &[(u128, String)]) -> Vec<Effect> {
        let mut log = Vec::new();
        let (config, rewards) = load_files(&self.dir, &mut log);
        let mut out: Vec<Effect> = log.into_iter().map(Effect::Console).collect();
        // Clear what the old perks set, then apply the new ones.
        for (uuid, _) in online {
            for p in &self.config.perks {
                out.extend(perk_effects(*uuid, p, 0));
            }
        }
        self.config = config;
        self.rewards = rewards;
        for (uuid, _) in online {
            out.extend(self.reapply(*uuid));
        }
        out.push(Effect::Console("[Progression] reloaded".into()));
        out
    }

    pub fn data(&self, uuid: u128) -> Option<&PlayerData> {
        self.players.get(&uuid)
    }

    fn data_mut(&mut self, uuid: u128) -> &mut PlayerData {
        self.dirty.insert(uuid);
        self.players.entry(uuid).or_default()
    }

    /// The tier of a perk the settings still have.
    pub fn tier(&self, uuid: u128, perk: &str) -> u32 {
        match (self.config.perk(perk), self.players.get(&uuid)) {
            (Some(p), Some(d)) => d.tier(perk).min(p.max()),
            _ => 0,
        }
    }

    /// What the travel perks let the player do.
    pub fn travel(&self, uuid: u128) -> Travel {
        let t = |id: &str| self.tier(uuid, id);
        let amount = |id: &str| self.config.perk(id).map_or(0.0, |p| p.amount);
        let chance = |id: &str| (amount(id) * f64::from(t(id)) / 100.0).clamp(0.0, 1.0);
        Travel {
            cart_steps: cart_steps(t("cart_speed")),
            cart_drive: t("cart_drive") > 0,
            cart_return: t("cart_return") > 0,
            horse: horse_modifiers(t("horse_speed"), t("horse_descent"), t("horse_terrain"), amount("horse_speed")),
            call: t("horse_call") > 0,
            bag_rows: bag_rows(t("horse_bags")),
            wear: chance("elytra_wear"),
            rockets: chance("elytra_rockets"),
            wall: t("elytra_wall") > 0,
            glide: glide_gravity(t("elytra_glide")),
            leaves: t("horse_leaves") > 0,
        }
    }

    /// The player's horse, if they bound one.
    pub fn horse(&self, uuid: u128) -> Option<&Horse> {
        self.players.get(&uuid)?.horse.as_ref()
    }

    /// The horse they ride now becomes theirs (or, the same one, is seen here).
    pub fn set_horse(&mut self, uuid: u128, horse: Horse) -> Vec<Effect> {
        if !self.players.contains_key(&uuid) || self.players[&uuid].horse.as_ref() == Some(&horse) {
            return Vec::new();
        }
        let new = self.players[&uuid].horse.as_ref().is_none_or(|h| h.uuid != horse.uuid);
        self.data_mut(uuid).horse = Some(horse);
        if new { self.save(uuid) } else { Vec::new() }
    }

    /// The saddlebags' slots (SNBT, "-" empty).
    pub fn bag(&self, uuid: u128) -> Vec<String> {
        self.players.get(&uuid).map(|d| d.bag.clone()).unwrap_or_default()
    }

    /// What the saddlebags hold after the player closed them.
    pub fn set_bag(&mut self, uuid: u128, mut slots: Vec<String>) -> Vec<Effect> {
        if !self.players.contains_key(&uuid) {
            return Vec::new();
        }
        while slots.last().is_some_and(|s| s == "-") {
            slots.pop();
        }
        self.data_mut(uuid).bag = slots;
        self.save(uuid)
    }

    /// Each player's transmitter limits from "Дистанційний сигнал" (None: may not make them).
    pub fn relays(&self, uuids: &[u128]) -> Vec<(u128, Option<(usize, i32)>)> {
        let Some(p) = self.config.perk("relay") else {
            return uuids.iter().map(|u| (*u, None)).collect();
        };
        uuids.iter().map(|u| (*u, relay_limits(p, self.tier(*u, "relay")))).collect()
    }

    /// `amount` percent a tier of a special perk, as a fraction (0.12 for two tiers of 6).
    pub fn bonus(&self, uuid: u128, perk: &str) -> f64 {
        let tier = self.tier(uuid, perk);
        self.config.perk(perk).map_or(0.0, |p| p.amount * f64::from(tier) / 100.0)
    }

    /// A player joined: their progress is read and their perks given.
    pub fn join(&mut self, uuid: u128, name: &str) -> Vec<Effect> {
        let mut out = Vec::new();
        if !self.players.contains_key(&uuid) {
            let (d, err) = self.store.load(uuid);
            if let Some(err) = err {
                out.push(Effect::Console(format!("[Progression] {err}")));
            }
            self.players.insert(uuid, d);
        }
        let rate = u64::from(self.config.settings.advancement_xp_per_point);
        let d = self.data_mut(uuid);
        d.name = name.to_owned();
        // A 0.x file: what they earned then becomes experience, the old perks are gone.
        let old = std::mem::take(&mut d.earned);
        d.chains.clear();
        d.skills.clear();
        if old > 0 && d.total_xp == 0 {
            d.points = 0;
            out.extend(self.add_xp(uuid, u64::from(old) * rate, false));
            out.push(Effect::Tell(uuid, "Прогресію оновлено: старі очки стали досвідом і рівнями.".into(), Tone::Info));
        }
        // Perks bought under an older layout: every point back, for free.
        if self.players[&uuid].version < CONFIG_VERSION {
            let level = self.players[&uuid].level;
            let refund = self.config.points_by(level);
            let had = !self.players[&uuid].perks.is_empty() || !self.players[&uuid].books.is_empty();
            for p in &self.config.perks {
                out.extend(perk_effects(uuid, p, 0));
            }
            let d = self.data_mut(uuid);
            d.version = CONFIG_VERSION;
            if had {
                d.perks.clear();
                d.books.clear();
                d.points = refund;
                out.push(Effect::Tell(uuid, format!("Перки оновлено: усі {refund} ОП повернуто безкоштовно, обери нові."), Tone::Info));
            }
        }
        // Left (or the server stopped) as a spirit: back in the body.
        out.extend(self.spirit_return(uuid, name, "Ти повернувся в тіло"));
        out.extend(self.reapply(uuid));
        if let Some(a) = self.players[&uuid].anchor.clone() {
            out.push(Effect::Anchor { owner: anchor_owner(uuid), dimension: a.dimension.clone(), chunks: a.chunks(), hold: true });
        }
        let d = &self.players[&uuid];
        if d.points > 0 {
            out.push(Effect::Tell(uuid, format!("Рівень прогресії {}, вільних ОП: {}. Меню: /progress", d.level, d.points), Tone::Info));
        }
        out
    }

    /// A player left: their progress is written and let go.
    pub fn leave(&mut self, uuid: u128) -> Vec<Effect> {
        let mut out = Vec::new();
        if let Some(a) = self.players.get(&uuid).and_then(|d| d.anchor.clone())
            && !self.config.settings.anchor_offline
        {
            out.push(Effect::Anchor { owner: anchor_owner(uuid), dimension: a.dimension.clone(), chunks: a.chunks(), hold: false });
        }
        out.extend(self.save(uuid));
        self.players.remove(&uuid);
        self.hurt_at.remove(&uuid);
        self.ready_at.retain(|(u, _), _| *u != uuid);
        out
    }

    /// Everything the player's perks set, as it should be now (a join, a respawn, a reload).
    pub fn reapply(&self, uuid: u128) -> Vec<Effect> {
        let mut out = Vec::new();
        for p in &self.config.perks {
            out.extend(perk_effects(uuid, p, self.tier(uuid, &p.id)));
        }
        out.push(Effect::ClientPerks(uuid, self.client_perks(uuid)));
        out
    }

    /// The client-side perks to tell a Horizon client about.
    pub fn client_perks(&self, uuid: u128) -> Vec<String> {
        self.config
            .perks
            .iter()
            .filter_map(|p| Some((p.client.as_ref()?, self.tier(uuid, &p.id))))
            .filter(|(_, tier)| *tier > 0)
            // The name, and with its tier ("progression:horse_descent#2") for a client that scales.
            .flat_map(|(c, tier)| [format!("progression:{c}"), format!("progression:{c}#{tier}")])
            .collect()
    }

    /// Gives progression experience: levels up as far as it goes, with the points of each level.
    /// `boosted` applies the "wisdom" perk.
    pub fn add_xp(&mut self, uuid: u128, amount: u64, boosted: bool) -> Vec<Effect> {
        if amount == 0 || !self.players.contains_key(&uuid) {
            return Vec::new();
        }
        let amount = if boosted { (amount as f64 * (1.0 + self.bonus(uuid, "wisdom"))).round() as u64 } else { amount };
        let max = self.config.settings.max_level;
        let (mut level, mut xp) = {
            let d = &self.players[&uuid];
            (d.level, d.xp + amount)
        };
        let mut points = 0;
        while level < max && xp >= self.config.xp_need(level) {
            xp -= self.config.xp_need(level);
            level += 1;
            points += self.config.points_at(level);
        }
        if level >= max {
            xp = 0;
        }
        let d = self.data_mut(uuid);
        let leveled = level > d.level;
        d.level = level;
        d.xp = xp;
        d.total_xp += amount;
        d.points += points;
        let name = d.name.clone();
        let mut out = Vec::new();
        if leveled {
            out.push(Effect::Tell(uuid, format!("Рівень прогресії {level}! +{points} ОП (вільних: {})", self.players[&uuid].points), Tone::Good));
            if safe_name(&name) {
                out.push(Effect::Command(format!(
                    "execute at {name} run playsound minecraft:entity.player.levelup player {name} ~ ~ ~ 1 0.6"
                )));
            }
            let opened: Vec<&str> = self
                .config
                .perks
                .iter()
                .filter(|p| p.levels.first().is_some_and(|l| *l == level))
                .map(|p| p.name.as_str())
                .collect();
            if !opened.is_empty() {
                out.push(Effect::Tell(uuid, format!("Відкрито перки: {}", opened.join(", ")), Tone::Info));
            }
            out.extend(self.save(uuid));
        } else {
            let need = self.config.xp_need(level);
            out.push(Effect::ActionBar(uuid, format!("+{amount} досвіду прогресії ({xp}/{need})")));
        }
        out
    }

    /// An advancement the player completed: its weight as experience, once.
    pub fn advancement(&mut self, uuid: u128, id: &str) -> Vec<Effect> {
        if !rewardable(id) || !self.players.contains_key(&uuid) || self.players[&uuid].quests.contains(id) {
            return Vec::new();
        }
        let weight = self
            .config
            .advancement_points
            .get(id)
            .or_else(|| self.rewards.points.get(id))
            .copied()
            .unwrap_or(self.config.settings.default_advancement_points);
        if weight == 0 {
            return Vec::new();
        }
        self.data_mut(uuid).quests.insert(id.to_owned());
        let xp = u64::from(weight) * u64::from(self.config.settings.advancement_xp_per_point);
        let mut out = self.add_xp(uuid, xp, true);
        out.push(Effect::Tell(uuid, format!("Досягнення: +{xp} досвіду прогресії"), Tone::Good));
        out
    }

    /// For admin commands on a player who is offline: their file is read in.
    pub fn load_offline(&mut self, name: &str) -> Option<u128> {
        if let Some((uuid, _)) = self.players.iter().find(|(_, d)| d.name.eq_ignore_ascii_case(name)) {
            return Some(*uuid);
        }
        let (uuid, d) = self.store.find_by_name(name)?;
        self.players.insert(uuid, d);
        Some(uuid)
    }

    pub fn save(&mut self, uuid: u128) -> Vec<Effect> {
        self.dirty.remove(&uuid);
        let Some(d) = self.players.get(&uuid) else {
            return Vec::new();
        };
        match self.store.save(uuid, d) {
            Ok(()) => Vec::new(),
            Err(e) => vec![Effect::Console(format!("[Progression] {e}"))],
        }
    }

    /// Writes every player whose progress changed (each minute, and when the server stops).
    pub fn save_dirty(&mut self) -> Vec<Effect> {
        let ids: Vec<u128> = self.dirty.iter().copied().collect();
        ids.into_iter().flat_map(|u| self.save(u)).collect()
    }

    /// Points the player has in perks.
    pub fn spent(&self, d: &PlayerData) -> u32 {
        self.config.perks.iter().map(|p| p.costs.iter().take(d.tier(&p.id) as usize).sum::<u32>()).sum()
    }

    pub fn respec_cost(&self, d: &PlayerData) -> u32 {
        self.spent(d) * self.config.settings.respec_levels_per_point
    }

    pub fn pvp_multiplier(&self) -> f32 {
        self.config.settings.pvp_damage_multiplier
    }

    /// `/progressadmin ...`: the lines to answer with, and what to do.
    pub fn admin(&mut self, args: &[&str], online: &[(u128, String)]) -> (Vec<String>, Vec<Effect>) {
        let usage = || {
            vec![
                "/progressadmin xp <гравець> <досвід>".to_owned(),
                "/progressadmin level <гравець> <рівень>".to_owned(),
                "/progressadmin points <гравець> <±ОП>".to_owned(),
                "/progressadmin reset <гравець>".to_owned(),
                "/progressadmin info <гравець>".to_owned(),
                "/progressadmin reload".to_owned(),
            ]
        };
        let Some(&cmd) = args.first() else {
            return (usage(), Vec::new());
        };
        if cmd.eq_ignore_ascii_case("reload") {
            let effects = self.reload(online);
            return (vec!["[Progression] перечитано".into()], effects);
        }
        let Some(&who) = args.get(1) else {
            return (usage(), Vec::new());
        };
        let Some(uuid) = self.load_offline(who) else {
            return (vec![format!("Гравця {who} ще не було на сервері")], Vec::new());
        };
        let is_online = online.iter().any(|(u, _)| *u == uuid);
        let number = args.get(2).and_then(|n| n.parse::<i64>().ok());
        let mut effects = Vec::new();
        let lines = match (cmd.to_ascii_lowercase().as_str(), number) {
            ("xp", Some(n)) if n > 0 => {
                effects.extend(self.add_xp(uuid, n as u64, false));
                let d = &self.players[&uuid];
                vec![format!("{who}: рівень {}, {} ОП", d.level, d.points)]
            }
            ("level", Some(n)) if n >= 0 => {
                let target = (n as u32).min(self.config.settings.max_level);
                let gained: u32 = {
                    let d = &self.players[&uuid];
                    (d.level + 1..=target).map(|l| self.config.points_at(l)).sum()
                };
                let d = self.data_mut(uuid);
                d.level = target;
                d.xp = 0;
                d.points += gained;
                vec![format!("{who}: рівень {target}, +{gained} ОП")]
            }
            ("points", Some(n)) => {
                let d = self.data_mut(uuid);
                d.points = (i64::from(d.points) + n).clamp(0, 1_000_000) as u32;
                vec![format!("{who}: {} ОП", d.points)]
            }
            ("reset", _) => {
                if is_online {
                    for p in &self.config.perks {
                        effects.extend(perk_effects(uuid, p, 0));
                    }
                    effects.push(Effect::ClientPerks(uuid, Vec::new()));
                }
                let d = self.data_mut(uuid);
                if let Some(a) = d.anchor.take() {
                    effects.push(Effect::Anchor { owner: anchor_owner(uuid), dimension: a.dimension.clone(), chunks: a.chunks(), hold: false });
                }
                let name = std::mem::take(&mut d.name);
                let spirit = d.spirit.take();
                *d = PlayerData { name, spirit, ..PlayerData::default() };
                vec![format!("{who}: прогрес стерто")]
            }
            ("info", _) => {
                let d = &self.players[&uuid];
                let perks: Vec<String> = d.perks.iter().map(|(k, v)| format!("{k} {v}")).collect();

                vec![
                    format!("{who}: рівень {} ({}/{} досвіду), {} ОП вільних, {} у дереві", d.level, d.xp, self.config.xp_need(d.level), d.points, self.spent(d)),
                    format!("перки: {}", if perks.is_empty() { "-".into() } else { perks.join(", ") }),
                    format!("лабораторія: {}", d.zone.as_ref().map_or("-".to_owned(), |z| format!("{} {:?} r{}", z.dimension, z.center, z.radius))),
                ]
            }
            _ => usage(),
        };
        effects.extend(self.save(uuid));
        if !is_online {
            self.players.remove(&uuid);
        }
        (lines, effects)
    }
}

// The menus.
impl Engine {
    fn perk_at(&self, page: Page, slot: usize) -> Option<&Perk> {
        let (row, col) = (slot / 9, slot % 9);
        let branch = page_branches(page).get(row)?;
        if col == 0 {
            return None;
        }
        self.config.perks.iter().filter(|p| p.branch == *branch).nth(col - 1)
    }

    /// Where a perk stands for the player: its frame, the price of its next tier and the level
    /// that tier needs (None when every tier is bought).
    fn perk_state(&self, p: &Perk, d: &PlayerData) -> (Frame, Option<(u32, u32)>) {
        let tier = d.tier(&p.id).min(p.max());
        if tier >= p.max() {
            return (Frame::Max, None);
        }
        let (cost, need) = (p.costs[tier as usize], p.levels[tier as usize]);
        let frame = if d.level < need {
            if tier > 0 { Frame::Owned } else { Frame::Locked }
        } else if d.points < cost {
            if tier > 0 { Frame::Owned } else { Frame::Poor }
        } else {
            Frame::Buy
        };
        (frame, Some((cost, need)))
    }

    /// What a perk gives at `tier`, in numbers and short words ("" when the about says it all).
    pub fn perk_numbers(&self, p: &Perk, tier: u32) -> String {
        let t = tier.clamp(1, p.max());
        let tf = f64::from(t);
        let amount = |per: f64| p.amount * per;
        let num = |v: f64| {
            let s = format!("{v:.2}");
            let s = s.trim_end_matches('0').trim_end_matches('.').to_owned();
            s.replace('.', ",")
        };
        let attr = |name: &str| p.attributes().find(|a| a.0 == name).map_or(0.0, |a| a.1 * tf);
        match p.id.as_str() {
            "health" => format!("+{} серця", num(attr("minecraft:max_health") / 2.0)),
            "fall" => format!(
                "безпечно з {} блоків, шкода від падіння −{:.0}%",
                num(3.0 + attr("minecraft:safe_fall_distance")),
                -attr("minecraft:fall_damage_multiplier") * 100.0
            ),
            "swimming" => {
                let air = attr("minecraft:oxygen_bonus") + 1.0;
                let mut s = format!(
                    "плавання +{:.0}%, копання під водою {:.0}% від звичайного, повітря на {}× довше",
                    attr("minecraft:water_movement_efficiency").min(1.0) * 100.0,
                    (0.2 + attr("minecraft:submerged_mining_speed")).min(1.0) * 100.0,
                    num(air)
                );
                if t >= 2 {
                    s.push_str(", не тонеш");
                }
                s
            }
            "wisdom" => format!("+{:.0}% досвіду прогресії", amount(tf)),
            "hunter" => format!("+{:.0}% шкоди по мобах", amount(tf)),
            "trophies" => format!("{:.0}% шанс подвоїти кожну річ", amount(tf)),
            "scholar" => format!("+{:.0}% досвіду з кульок", amount(tf)),
            "toughness" => format!("+{} міцності броні (незеритова частина дає 3)", num(attr("minecraft:armor_toughness"))),
            "steadfast" => format!(
                "відкидання від ударів −{:.0}%, від вибухів −{:.0}%",
                attr("minecraft:knockback_resistance").min(1.0) * 100.0,
                attr("minecraft:explosion_knockback_resistance").min(1.0) * 100.0
            ),
            "lunge" => format!("дальність удару +{} бл.", num(attr("minecraft:entity_interaction_range"))),
            "reach" => format!("дальність рук +{} бл.", num(attr("minecraft:block_interaction_range"))),
            "step" => format!(
                "крок на {} блока, крадешся на {:.0}% швидше",
                num(0.6 + attr("minecraft:step_height")),
                attr("minecraft:sneaking_speed") / 0.3 * 100.0
            ),
            "lab" => {
                let side = lab_radius(p, t) * 2 + 1;
                format!("зона {side}×{side}×{side} блоків")
            }
            "anchor" => format!("квадрат {0}×{0} чанків", t * 2 + 1),
            "timber" => format!("до {} колод за раз", timber_logs(p, t)),
            "cart_speed" => format!("{} кроки за тік, до {} м/с", cart_steps(t), cart_steps(t) * 8),
            "horse_speed" => format!("+{:.0}% швидкості коня", amount(tf)),
            "horse_descent" => format!("падає в {} разів швидше", horse_fall_times(t)),
            "horse_leaves" => String::new(),
            "horse_terrain" => format!("переступає {} блоки", if t >= 2 { "2" } else { "1,5" }),
            "horse_bags" => format!("слотів: {}", u32::from(bag_rows(t)) * 9),
            "elytra_wear" => format!("{:.0}% шанс, що елітри не зносяться", amount(tf)),
            "elytra_rockets" => format!("{:.0}% шанс повернути феєрверк", amount(tf)),
            "elytra_glide" => format!("тяжіння в польоті −{:.0}%", -glide_gravity(t) * 100.0),
            "relay" => match relay_limits(p, t) {
                Some((n, _)) => format!("до {n} цілей на передавач"),
                None => String::new(),
            },
            "draft" => match fill_limit(p, t) {
                0 => "лише розмітка".to_owned(),
                n => format!("заливка до {n} блоків за раз"),
            },
            "void_walker" | "last_stand" => format!("раз на {} с", rescue_seconds(p, t)),
            "clear_mind" => {
                let names: Vec<&str> = CLEAR_MIND.iter().take(t as usize).flat_map(|l| l.iter()).map(|e| effect_name(e)).collect();
                format!("знімає: {}", names.join(", "))
            }
            "alchemist" => match t {
                1 => "корисні ефекти тривають у 1,5 раза довше".to_owned(),
                2 => "корисні ефекти тривають удвічі довше".to_owned(),
                _ => "корисні ефекти тривають удвічі довше і на рівень сильніші".to_owned(),
            },
            id => match altar_group(id) {
                Some(group) => {
                    let books: Vec<String> = self
                        .config
                        .books
                        .iter()
                        .filter(|b| b.group == group)
                        .map(|b| format!("{} {}", b.name, roman(b.vanilla_max + t.min(b.max_extra))))
                        .collect();
                    format!("книги: {}", books.join(", "))
                }
                None => String::new(),
            },
        }
    }

    /// The perk's card text: what it does, then what the player has and what the next tier
    /// gives, each starting a line (`¶` breaks a line).
    pub fn perk_about(&self, p: &Perk, tier: u32) -> String {
        let mut out = p.about.clone();
        let now = if tier > 0 { self.perk_numbers(p, tier) } else { String::new() };
        if !now.is_empty() && p.max() > 1 {
            out.push_str(&format!("¶Зараз: {now}"));
        }
        if tier < p.max() {
            let next = self.perk_numbers(p, tier + 1);
            if !next.is_empty() {
                let label = if tier == 0 { "Дає" } else { "Далі" };
                out.push_str(&format!("¶{label}: {next}"));
            }
        } else if p.max() == 1 && !now.is_empty() {
            out.push_str(&format!("¶{}", capital(&now)));
        }
        out
    }

    fn perk_cell(&self, p: &Perk, d: &PlayerData) -> Cell {
        let tier = d.tier(&p.id).min(p.max());
        let (frame, next) = self.perk_state(p, d);
        let mut lore = Vec::new();
        for part in self.perk_about(p, tier).split('¶') {
            let color = if part.starts_with("Зараз:") {
                "green"
            } else if part.starts_with("Далі:") || part.starts_with("Дає:") {
                "dark_aqua"
            } else {
                "gray"
            };
            for l in wrap(part, 44) {
                lore.push((l, color));
            }
        }
        if p.cooldown_seconds > 0 && p.is_skill() {
            lore.push((format!("Перезарядка: {} с", p.cooldown_seconds), "gray"));
        }
        if p.max() > 1 {
            lore.push((format!("Рівень перка: {tier}/{}", p.max()), "white"));
        }
        match next {
            None => {
                lore.push(("Вивчено повністю".into(), "gold"));
                if p.is_item() {
                    lore.push((format!("Клік: отримати ще одну за {} ванільних рівнів", p.item_levels), "green"));
                } else if p.is_skill() {
                    lore.push(("Працює на клавішу (кнопка «Клавіші» внизу)".into(), "dark_gray"));
                }
            }
            Some((cost, need)) => {
                match frame {
                    Frame::Buy => {
                        lore.push((format!("Ціна: {cost} ОП"), "green"));
                        lore.push(("Клік: вивчити".into(), "green"));
                    }
                    Frame::Poor => lore.push((format!("Ціна: {cost} ОП (у тебе {})", d.points), "red")),
                    _ if d.level < need => {
                        lore.push((format!("Потрібен рівень прогресії {need} (у тебе {})", d.level), "red"));
                        lore.push((format!("Ціна: {cost} ОП"), "gray"));
                    }
                    _ => lore.push((format!("Ціна: {cost} ОП (у тебе {})", d.points), "red")),
                }
            }
        }
        let color = match frame {
            Frame::Max => "gold",
            Frame::Owned | Frame::Buy => "green",
            Frame::Poor => "yellow",
            _ => "gray",
        };
        Cell { icon: p.icon.clone(), count: tier.max(1), name: p.name.clone(), color, lore, glint: tier >= p.max(), frame }
    }

    /// The level of `b` the player can buy now (0: none yet).
    pub fn book_level(&self, uuid: u128, b: &Book) -> u32 {
        let tier = altar_of(&b.group).map_or(0, |perk| self.tier(uuid, perk));
        if tier == 0 { 0 } else { b.vanilla_max + tier.min(b.max_extra) }
    }

    fn book_cell(&self, uuid: u128, b: &Book) -> Cell {
        let level = self.book_level(uuid, b);
        let top = b.vanilla_max + b.max_extra;
        let altar = altar_of(&b.group).and_then(|id| self.config.perk(id));
        let mut lore = vec![(format!("Ванільний максимум {}, вівтар дає до {}", roman(b.vanilla_max), roman(top)), "gray")];
        let frame;
        if level == 0 {
            frame = Frame::Locked;
            lore.push((format!("Відкривається перком «{}»", altar.map_or("Вівтар", |p| p.name.as_str())), "red"));
        } else {
            frame = if level >= top { Frame::Max } else { Frame::Owned };
            lore.push((format!("Клік: {} {} за {} ванільних рівнів", b.name, roman(level), b.price_levels), "green"));
            if level < top {
                lore.push((format!("Вищий рівень: наступний рівень перка «{}»", altar.map_or("", |p| p.name.as_str())), "dark_aqua"));
            }
        }
        let color = if level > 0 { "gold" } else { "gray" };
        Cell { icon: "minecraft:enchanted_book".into(), count: level.max(b.vanilla_max).clamp(1, 99), name: b.name.clone(), color, lore, glint: level > 0, frame }
    }

    /// What `page` shows the player. `pack`: their client has the resource pack.
    pub fn menu(&self, uuid: u128, page: Page, pack: bool) -> MenuView {
        let blank = PlayerData::default();
        let d = self.players.get(&uuid).unwrap_or(&blank);
        let rows = page.rows();
        let mut cells: Vec<Option<Cell>> = vec![None; usize::from(rows) * 9];
        let s = &self.config.settings;
        let need = self.config.xp_need(d.level);
        let maxed = d.level >= s.max_level;
        let (label, bar);
        match page {
            Page::Perks | Page::Travel => {
                cells[SLOT_PAGE] = Some(if page == Page::Perks {
                    Cell::plain("minecraft:saddle", "Гілки руху", "aqua", vec![("Вагонетка, кінь, елітри".into(), "gray")])
                } else {
                    Cell::plain("minecraft:totem_of_undying", "Основні гілки", "aqua", vec![("Виживання, PvP, PvE, будівництво, редстоун".into(), "gray")])
                });
                for (row, branch) in page_branches(page).iter().enumerate() {
                    let (name, icon, about) = gui::branch(branch);
                    cells[row * 9] = Some(Cell::plain(icon, name, "aqua", vec![(about.into(), "gray")]));
                    for (i, p) in self.config.perks.iter().filter(|p| p.branch == *branch).enumerate() {
                        cells[row * 9 + 1 + i] = Some(self.perk_cell(p, d));
                    }
                }
                let progress = if maxed { "максимальний рівень".to_owned() } else { format!("{}/{need} досвіду до наступного", d.xp) };
                cells[SLOT_INFO] = Some(Cell::plain(
                    "minecraft:experience_bottle",
                    &format!("Рівень прогресії {}", d.level),
                    "gold",
                    vec![
                        (progress, "white"),
                        (format!("Вільних ОП: {}, у дереві: {}", d.points, self.spent(d)), "white"),
                        ("Рівень не втрачається зі смертю.".into(), "gray"),
                        ("Досвід дають лише досягнення, кожне один раз:".into(), "gray"),
                        ("ванільні, нових структур і модів.".into(), "gray"),
                    ],
                ));
                cells[SLOT_KEYS] = Some(Cell::plain(
                    "minecraft:oak_sign",
                    "Клавіші",
                    "yellow",
                    vec![
                        ("Клієнт Horizon: Налаштування > Моди > Клавіші сервера".into(), "gray"),
                        ("(карта перків K, ривки R і V, дух B, лабораторія L, якір H)".into(), "gray"),
                        ("Ванільний клієнт: G (Швидкі дії) відкриває список,".into(), "gray"),
                        ("F: ривок вперед, Shift+F: ривок вгору".into(), "gray"),
                        (format!("Вівтар: {} на {}", gui::short(&s.altar_block), gui::short(&s.altar_base)), "gray"),
                    ],
                ));
                cells[SLOT_RESPEC] = Some(Cell::plain(
                    "minecraft:grindstone",
                    "Скинути перки",
                    "red",
                    vec![
                        ("Повертає всі ОП з перків".into(), "gray"),
                        (format!("Ціна зараз: {} ванільних рівнів", self.respec_cost(d)), "gray"),
                    ],
                ));
                cells[SLOT_CLOSE] = Some(Cell::plain("minecraft:barrier", "Закрити", "red", Vec::new()));
                label = format!("Рівень {} · ОП {}", d.level, d.points);
                bar = Some(if maxed { 1.0 } else { d.xp as f32 / need as f32 });
            }
            Page::Altar => {
                // Each group on its own row: tools, armour, weapons.
                let mut row_fill = [0usize; 3];
                for b in &self.config.books {
                    let row = ALTARS.iter().position(|(g, _)| *g == b.group).unwrap_or(0);
                    let at = BOOK_FIRST + row * 9 + row_fill[row];
                    if row_fill[row] < 9 && at < SLOT_INFO {
                        cells[at] = Some(self.book_cell(uuid, b));
                        row_fill[row] += 1;
                    }
                }
                cells[4] = Some(Cell::plain(
                    "minecraft:enchanting_table",
                    "Вівтар надчар",
                    "light_purple",
                    vec![
                        ("Книги вище ванільного максимуму.".into(), "gray"),
                        ("Ряди: інструменти, броня, зброя.".into(), "gray"),
                        ("Рівень книг росте з перками вівтарів.".into(), "gray"),
                        ("Клік: купити книгу за ванільні рівні.".into(), "gray"),
                        ("Ковадло накладає їх без обрізання рівня.".into(), "gray"),
                    ],
                ));
                cells[SLOT_CLOSE] = Some(Cell::plain("minecraft:barrier", "Закрити", "red", Vec::new()));
                label = "Вівтар надчар".to_owned();
                bar = None;
            }
            Page::Respec => {
                let cost = self.respec_cost(d);
                cells[SLOT_YES] = Some(Cell::plain(
                    "minecraft:lime_concrete",
                    "Так, скинути",
                    "green",
                    vec![(format!("Повернеться {} ОП", self.spent(d)), "gray"), (format!("Коштує {cost} ванільних рівнів"), "gray")],
                ));
                cells[SLOT_NO] = Some(Cell::plain("minecraft:red_concrete", "Ні, назад", "red", Vec::new()));
                label = "Скинути перки?".to_owned();
                bar = None;
            }
            Page::Lab => {
                let tier = self.tier(uuid, "lab");
                let radius = self.config.perk("lab").map_or(8, |p| lab_radius(p, tier.max(1)));
                let side = radius * 2 + 1;
                let zone = d.zone.as_ref();
                let place = match zone {
                    Some(z) => format!("Зараз: {} {} {} ({})", z.center[0], z.center[1], z.center[2], gui::short(&z.dimension)),
                    None => "Зони ще немає".to_owned(),
                };
                cells[LAB_HERE] = Some(Cell::plain(
                    "minecraft:target",
                    "Поставити зону тут",
                    "aqua",
                    vec![(format!("Куб {side}×{side}×{side} з центром на тобі"), "gray"), (place, "gray")],
                ));
                let on_off = |on: bool| if on { ("Увімкнено", "green") } else { ("Вимкнено", "red") };
                let (blast, blast_c) = on_off(zone.is_some_and(|z| z.no_blast));
                cells[LAB_BLAST] = Some(Cell::plain(
                    "minecraft:tnt",
                    "Вибухи не ламають блоки",
                    blast_c,
                    vec![("ТНТ, кристали, кріпери в зоні ламають лише шкоду, не блоки".into(), "gray"), (blast.into(), blast_c)],
                ));
                let (freeze, freeze_c) = on_off(zone.is_some_and(|z| z.freeze));
                cells[LAB_FREEZE] = Some(Cell::plain(
                    "minecraft:observer",
                    "Заморозити спостерігачі",
                    freeze_c,
                    vec![("Спостерігачі в зоні не дають імпульс: будуй спокійно".into(), "gray"), ("Вимкни, коли механізм готовий".into(), "gray"), (freeze.into(), freeze_c)],
                ));
                cells[LAB_REMOVE] = Some(Cell::plain("minecraft:barrier", "Прибрати зону", "red", Vec::new()));
                cells[LAB_CLOSE] = Some(Cell::plain("minecraft:oak_door", "Закрити", "gray", Vec::new()));
                label = "Лабораторія редстоуну".to_owned();
                bar = None;
            }
            Page::Draft => {
                let draft = d.draft.as_ref();
                let shape = draft.map_or("line", |r| r.shape.as_str());
                let at = |p: Option<[i32; 3]>| p.map_or("не поставлена".to_owned(), |p| format!("{} {} {}", p[0], p[1], p[2]));
                let points = vec![
                    (format!("A: {}", at(draft.and_then(|r| r.a))), "gray"),
                    (format!("B: {}", at(draft.and_then(|r| r.b))), "gray"),
                    ("Shift + ПКМ палицею по блоку ставить точку".into(), "dark_gray"),
                ];
                let pick = |icon: &str, name: &str, id: &str, about: &str| {
                    let mut lore = vec![(about.to_owned(), "gray")];
                    lore.extend(points.iter().cloned());
                    let mut c = Cell::plain(icon, name, if shape == id { "green" } else { "white" }, lore);
                    if shape == id {
                        c.frame = Frame::Max;
                    }
                    c
                };
                cells[DRAFT_LINE] = Some(pick("minecraft:string", "Лінія", "line", "Пряма від A до B"));
                cells[DRAFT_BOX] = Some(pick("minecraft:scaffolding", "Прямокутник", "box", "Ребра коробки між A і B (плоска, коли вони в одному шарі)"));
                cells[DRAFT_CIRCLE] = Some(pick("minecraft:clock", "Коло", "circle", "Центр у A, радіус до B, у шарі A"));
                let tier = self.tier(uuid, "draft");
                let limit = self.config.perk("draft").map_or(0, |p| fill_limit(p, tier));
                cells[DRAFT_FILL] = Some(if limit > 0 {
                    Cell::plain(
                        "minecraft:bucket",
                        "Залити блоками",
                        "aqua",
                        vec![
                            ("Ставить блок з лівої руки в усі порожні місця розмітки".into(), "gray"),
                            ("Прямокутник і коло заливаються повністю".into(), "gray"),
                            (format!("До {limit} блоків за раз, з твого інвентаря"), "gray"),
                        ],
                    )
                } else {
                    Cell::plain("minecraft:bucket", "Заливка", "dark_gray", vec![("Відкривається на II рівні «Кресляра»".into(), "gray")])
                });
                cells[DRAFT_CLEAR] = Some(Cell::plain("minecraft:barrier", "Прибрати розмітку", "red", Vec::new()));
                cells[DRAFT_CLOSE] = Some(Cell::plain("minecraft:oak_door", "Закрити", "gray", Vec::new()));
                label = "Кресляр".to_owned();
                bar = None;
            }
        }
        if !pack {
            // No resource pack: the plain chest look, with panes where the pack draws its own.
            for c in cells.iter_mut().filter(|c| c.is_none()) {
                *c = Some(Cell::filler());
            }
        }
        let frames: Vec<(usize, Frame)> =
            cells.iter().enumerate().filter_map(|(i, c)| c.as_ref().map(|c| (i, c.frame))).filter(|(_, f)| *f != Frame::None).collect();
        MenuView { rows, glyphs: if pack { gui::glyphs(page, &frames, bar) } else { String::new() }, label, cells }
    }

    /// A click on `slot` of `page`: `button` 0 is the left one. `me` is what the server sees of
    /// the player.
    pub fn click(&mut self, uuid: u128, page: Page, slot: usize, button: i32, me: &Seen) -> Clicked {
        let _ = button;
        let mut out = Clicked::default();
        if !self.players.contains_key(&uuid) {
            out.close = true;
            return out;
        }
        match page {
            Page::Perks | Page::Travel => match slot {
                SLOT_CLOSE => out.close = true,
                SLOT_PAGE => out.show = Some(if page == Page::Perks { Page::Travel } else { Page::Perks }),
                SLOT_RESPEC => out.show = Some(Page::Respec),
                _ => {
                    if let Some(id) = self.perk_at(page, slot).map(|p| p.id.clone()) {
                        out.effects = self.buy(uuid, &id, me);
                    }
                }
            },
            Page::Altar => {
                if slot == SLOT_CLOSE {
                    out.close = true;
                } else if slot >= BOOK_FIRST && slot < SLOT_INFO {
                    let (row, col) = ((slot - BOOK_FIRST) / 9, (slot - BOOK_FIRST) % 9);
                    let book = ALTARS.get(row).and_then(|(g, _)| self.config.books.iter().filter(|b| b.group == *g).nth(col)).cloned();
                    if let Some(b) = book {
                        out.effects = self.buy_book(uuid, &b, me);
                    }
                }
            }
            Page::Respec => match slot {
                SLOT_YES => {
                    out.effects = self.respec(uuid, me);
                    out.show = Some(Page::Perks);
                }
                SLOT_NO => out.show = Some(Page::Perks),
                _ => {}
            },
            Page::Lab => {
                out.effects = match slot {
                    LAB_HERE => self.lab_here(uuid, me),
                    LAB_BLAST => self.lab_toggle(uuid, false),
                    LAB_FREEZE => self.lab_toggle(uuid, true),
                    LAB_REMOVE => {
                        let mut e = Vec::new();
                        if self.data_mut(uuid).zone.take().is_some() {
                            e.push(Effect::ActionBar(uuid, "Зону лабораторії прибрано".into()));
                        }
                        e.extend(self.save(uuid));
                        e
                    }
                    LAB_CLOSE => {
                        out.close = true;
                        Vec::new()
                    }
                    _ => Vec::new(),
                };
            }
            Page::Draft => {
                out.effects = match slot {
                    DRAFT_LINE => self.draft_shape_set(uuid, "line"),
                    DRAFT_BOX => self.draft_shape_set(uuid, "box"),
                    DRAFT_CIRCLE => self.draft_shape_set(uuid, "circle"),
                    DRAFT_FILL => {
                        let e = self.draft_fill(uuid);
                        if e.iter().any(|e| matches!(e, Effect::Fill { .. })) {
                            out.close = true;
                        }
                        e
                    }
                    DRAFT_CLEAR => {
                        let mut e = Vec::new();
                        if self.data_mut(uuid).draft.take().is_some() {
                            e.push(Effect::ActionBar(uuid, "Розмітку прибрано".into()));
                        }
                        e.extend(self.save(uuid));
                        e
                    }
                    DRAFT_CLOSE => {
                        out.close = true;
                        Vec::new()
                    }
                    _ => Vec::new(),
                };
            }
        }
        out
    }

    fn lab_here(&mut self, uuid: u128, me: &Seen) -> Vec<Effect> {
        let tier = self.tier(uuid, "lab");
        let Some(p) = self.config.perk("lab").cloned() else {
            return Vec::new();
        };
        if tier == 0 {
            return vec![Effect::ActionBar(uuid, "Спершу вивчи перк «Лабораторія»".into())];
        }
        let radius = lab_radius(&p, tier);
        let center = [me.position[0].floor() as i32, me.position[1].floor() as i32, me.position[2].floor() as i32];
        let (no_blast, freeze) = self.players[&uuid].zone.as_ref().map_or((true, false), |z| (z.no_blast, z.freeze));
        self.data_mut(uuid).zone = Some(Zone { dimension: me.dimension.clone(), center, radius, no_blast, freeze });
        let side = radius * 2 + 1;
        let mut out = vec![Effect::Tell(uuid, format!("Лабораторія тут: куб {side}×{side}×{side} навколо {} {} {}", center[0], center[1], center[2]), Tone::Good)];
        out.extend(self.save(uuid));
        out
    }

    /// "Кресляр": a click with the stick puts point A, then B; the next one starts again.
    pub fn draft_point(&mut self, uuid: u128, dimension: &str, pos: [i32; 3]) -> Vec<Effect> {
        if self.tier(uuid, "draft") == 0 {
            return Vec::new();
        }
        let old = self.players.get(&uuid).and_then(|d| d.draft.clone());
        let shape = old.as_ref().map_or_else(|| "line".to_owned(), |r| r.shape.clone());
        let (draft, which) = match old {
            Some(r) if r.dimension == dimension && r.a.is_some() && r.b.is_none() => (Draft { b: Some(pos), ..r }, "B"),
            _ => (Draft { dimension: dimension.to_owned(), a: Some(pos), b: None, shape }, "A"),
        };
        self.data_mut(uuid).draft = Some(draft);
        let tail = if which == "A" { "ще одна точка B" } else { "ПКМ палицею: форма і заливка" };
        let mut out = vec![Effect::ActionBar(uuid, format!("Кресляр: точка {which} {} {} {} ({tail})", pos[0], pos[1], pos[2]))];
        out.extend(self.save(uuid));
        out
    }

    fn draft_shape_set(&mut self, uuid: u128, shape: &str) -> Vec<Effect> {
        let d = self.data_mut(uuid);
        match d.draft.as_mut() {
            Some(r) => r.shape = shape.to_owned(),
            None => d.draft = Some(Draft { dimension: String::new(), a: None, b: None, shape: shape.to_owned() }),
        }
        self.save(uuid)
    }

    /// "Кресляр" II: the drawing's places to fill, or why not.
    fn draft_fill(&mut self, uuid: u128) -> Vec<Effect> {
        let tier = self.tier(uuid, "draft");
        let limit = self.config.perk("draft").map_or(0, |p| fill_limit(p, tier));
        if limit == 0 {
            return vec![Effect::ActionBar(uuid, "Заливка відкривається на II рівні «Кресляра»".into())];
        }
        let Some(Draft { dimension, a: Some(a), b: Some(b), shape }) = self.players.get(&uuid).and_then(|d| d.draft.clone()) else {
            return vec![Effect::ActionBar(uuid, "Спершу дві точки: Shift + ПКМ палицею".into())];
        };
        let blocks = draft_shape(&shape, a, b, true, limit);
        if blocks.len() > limit {
            return vec![Effect::ActionBar(uuid, format!("Завелика форма: більше {limit} блоків"))];
        }
        let mut blocks = blocks;
        fill_order(&mut blocks);
        vec![Effect::Fill { uuid, dimension, blocks }]
    }

    /// The drawings to show now: player, dimension, points (spread out), A and B.
    pub fn drafts(&self) -> Vec<(u128, String, Vec<[i32; 3]>)> {
        self.players
            .iter()
            .filter(|(u, _)| self.tier(**u, "draft") > 0)
            .filter_map(|(u, d)| {
                let r = d.draft.as_ref()?;
                let a = r.a?;
                let points = match r.b {
                    Some(b) => spread(&draft_shape(&r.shape, a, b, false, 20_000), DRAFT_SHOWN),
                    None => vec![a],
                };
                Some((*u, r.dimension.clone(), points))
            })
            .collect()
    }

    fn lab_toggle(&mut self, uuid: u128, observers: bool) -> Vec<Effect> {
        let Some(mut z) = self.players.get(&uuid).and_then(|d| d.zone.clone()) else {
            return vec![Effect::ActionBar(uuid, "Спершу постав зону (мішень)".into())];
        };
        if observers {
            z.freeze = !z.freeze;
        } else {
            z.no_blast = !z.no_blast;
        }
        self.data_mut(uuid).zone = Some(z);
        self.save(uuid)
    }

    /// The labs that work now: their owners are online and still have the perk.
    pub fn zones(&self) -> Vec<Zone> {
        let max = |uuid: u128| self.config.perk("lab").map_or(0, |p| lab_radius(p, self.tier(uuid, "lab")));
        self.players
            .iter()
            .filter(|(u, _)| self.tier(**u, "lab") > 0)
            .filter_map(|(u, d)| d.zone.clone().map(|z| Zone { radius: z.radius.min(max(*u)), ..z }))
            .collect()
    }

    /// Buys the next tier of perk `id`; for an item perk already learned, one more item.
    pub fn buy(&mut self, uuid: u128, id: &str, me: &Seen) -> Vec<Effect> {
        let Some(p) = self.config.perk(id).cloned() else {
            return Vec::new();
        };
        let d = &self.players[&uuid];
        let tier = d.tier(id);
        if tier >= p.max() {
            if p.is_item() {
                if me.xp_level < p.item_levels as i32 {
                    return vec![Effect::ActionBar(uuid, format!("Потрібно {} ванільних рівнів, у тебе {}", p.item_levels, me.xp_level))];
                }
                let item = if id == "pickaxe3" { Item::Pickaxe3 } else { Item::Shovel3 };
                return vec![Effect::Sell { uuid, levels: p.item_levels, item }];
            }
            return vec![Effect::ActionBar(uuid, format!("«{}» уже вивчено повністю", p.name))];
        }
        let (cost, need) = (p.costs[tier as usize], p.levels[tier as usize]);
        if d.level < need {
            return vec![Effect::ActionBar(uuid, format!("Потрібен рівень прогресії {need}, у тебе {}", d.level))];
        }
        if d.points < cost {
            return vec![Effect::ActionBar(uuid, format!("Потрібно {cost} ОП, у тебе {}", d.points))];
        }
        let d = self.data_mut(uuid);
        d.points -= cost;
        d.perks.insert(id.to_owned(), tier + 1);
        let mut out = perk_effects(uuid, &p, tier + 1);
        let shown = if p.max() > 1 { format!("{} {}", p.name, roman(tier + 1)) } else { p.name.clone() };
        out.push(Effect::Tell(uuid, format!("Вивчено: {shown}"), Tone::Good));
        if id == "relay" && tier == 0 {
            out.push(Effect::Sell { uuid, levels: 0, item: Item::Transmitter });
        }
        if p.is_item() {
            // The first one comes with the perk.
            out.push(Effect::Sell { uuid, levels: 0, item: if id == "pickaxe3" { Item::Pickaxe3 } else { Item::Shovel3 } });
        }
        if p.client.is_some() {
            out.push(Effect::ClientPerks(uuid, self.client_perks(uuid)));
        }
        if id == "anchor"
            && let Some(a) = self.players[&uuid].anchor.clone()
        {
            // A wider anchor where the old one stands.
            let wide = Anchor { radius: self.anchor_radius(uuid), ..a };
            out.push(Effect::Anchor { owner: anchor_owner(uuid), dimension: wide.dimension.clone(), chunks: wide.chunks(), hold: true });
            self.data_mut(uuid).anchor = Some(wide);
        }
        if id == "lab"
            && let Some(z) = self.players[&uuid].zone.clone()
        {
            // A wider lab where the old one stands.
            self.data_mut(uuid).zone = Some(Zone { radius: lab_radius(&p, tier + 1), ..z });
        }
        out.extend(self.save(uuid));
        out
    }

    /// One book of the highest level the player's altar perk opens, for vanilla levels.
    pub fn buy_book(&mut self, uuid: u128, b: &Book, me: &Seen) -> Vec<Effect> {
        let level = self.book_level(uuid, b);
        if level == 0 {
            let name = altar_of(&b.group).and_then(|id| self.config.perk(id)).map_or("Вівтар", |p| p.name.as_str());
            return vec![Effect::ActionBar(uuid, format!("Ця книга відкривається перком «{name}»"))];
        }
        if me.xp_level < b.price_levels as i32 {
            return vec![Effect::ActionBar(uuid, format!("Потрібно {} ванільних рівнів, у тебе {}", b.price_levels, me.xp_level))];
        }
        vec![Effect::Sell { uuid, levels: b.price_levels, item: Item::Book { enchant: b.enchant.clone(), level } }]
    }

    /// Gives every point in perks back, for vanilla levels.
    pub fn respec(&mut self, uuid: u128, me: &Seen) -> Vec<Effect> {
        let d = &self.players[&uuid];
        let (back, cost) = (self.spent(d), self.respec_cost(d));
        if back == 0 {
            return vec![Effect::ActionBar(uuid, "Немає чого скидати".into())];
        }
        if me.xp_level < cost as i32 {
            return vec![Effect::ActionBar(uuid, format!("Потрібно {cost} ванільних рівнів, у тебе {}", me.xp_level))];
        }
        let mut out = vec![Effect::TakeLevels(uuid, cost)];
        for p in &self.config.perks {
            out.extend(perk_effects(uuid, p, 0));
        }
        out.extend(self.spirit_return(uuid, &me.name, "Ти повернувся в тіло"));
        let d = self.data_mut(uuid);
        d.perks.clear();
        d.books.clear();
        d.points += back;
        d.respecs += 1;
        if let Some(a) = d.anchor.take() {
            out.push(Effect::Anchor { owner: anchor_owner(uuid), dimension: a.dimension.clone(), chunks: a.chunks(), hold: false });
        }
        out.push(Effect::ClientPerks(uuid, Vec::new()));
        out.push(Effect::Tell(uuid, format!("Перки скинуто: повернулося {back} ОП"), Tone::Good));
        out.extend(self.save(uuid));
        out
    }
}

// Keys and skills.
impl Engine {
    fn anchor_radius(&self, uuid: u128) -> i32 {
        let per = self.config.perk("anchor").map_or(1.0, |p| p.amount);
        (per * f64::from(self.tier(uuid, "anchor"))).round() as i32
    }

    /// A key, a dialog button or F: `action` is one of [`ACTIONS`] (or `altar`, from a click on
    /// the altar block); `now` counts server ticks.
    pub fn action(&mut self, uuid: u128, action: &str, now: u64, me: &Seen) -> Acted {
        let mut out = Acted::default();
        if !self.players.contains_key(&uuid) {
            return out;
        }
        match action {
            "perks" | "keys" => out.open = Some(Page::Perks),
            "travel" => out.open = Some(Page::Travel),
            "respec" => out.open = Some(Page::Respec),
            "altar" => {
                if ALTARS.iter().any(|(_, p)| self.tier(uuid, p) > 0) {
                    out.open = Some(Page::Altar);
                } else {
                    out.effects.push(Effect::ActionBar(uuid, "Це вівтар надчар: спершу вивчи перк одного з вівтарів".into()));
                }
            }
            "draft" => {
                if self.tier(uuid, "draft") > 0 {
                    out.open = Some(Page::Draft);
                } else {
                    out.effects.push(Effect::ActionBar(uuid, "«Кресляр» ще не вивчений (меню перків)".into()));
                }
            }
            "lab" => {
                if self.tier(uuid, "lab") > 0 {
                    out.open = Some(Page::Lab);
                } else {
                    out.effects.push(Effect::ActionBar(uuid, "«Лабораторія» ще не вивчена (меню перків)".into()));
                }
            }
            "dash_forward" | "dash_up" | "spirit" | "anchor" => {
                let Some(p) = self.config.perk(action).cloned() else {
                    return out;
                };
                if self.tier(uuid, action) == 0 {
                    out.effects.push(Effect::ActionBar(uuid, format!("«{}» ще не вивчено (меню перків)", p.name)));
                    return out;
                }
                let key = (uuid, action.to_owned());
                if let Some(&at) = self.ready_at.get(&key)
                    && now < at
                {
                    out.effects.push(Effect::ActionBar(uuid, format!("{}: ще {} с", p.name, (at - now).div_ceil(20))));
                    return out;
                }
                if p.cooldown_seconds > 0 {
                    self.ready_at.insert(key, now + u64::from(p.cooldown_seconds) * 20);
                }
                out.effects = match action {
                    "dash_forward" => vec![Effect::Dash(uuid, me.name.clone(), "dash_forward")],
                    "dash_up" => vec![Effect::Dash(uuid, me.name.clone(), "dash_up")],
                    "anchor" => self.toggle_anchor(uuid, me),
                    _ => self.toggle_spirit(uuid, now, me),
                };
            }
            _ => {}
        }
        out
    }

    fn toggle_anchor(&mut self, uuid: u128, me: &Seen) -> Vec<Effect> {
        let owner = anchor_owner(uuid);
        if let Some(a) = self.data_mut(uuid).anchor.take() {
            let mut out = vec![
                Effect::Anchor { owner, dimension: a.dimension.clone(), chunks: a.chunks(), hold: false },
                Effect::ActionBar(uuid, "Якір чанків знято".into()),
            ];
            out.extend(self.save(uuid));
            return out;
        }
        if !matches!(me.dimension.as_str(), "minecraft:overworld" | "minecraft:the_nether" | "minecraft:the_end") {
            return vec![Effect::ActionBar(uuid, "Якір тут не працює".into())];
        }
        let chunk = [(me.position[0].floor() as i32) >> 4, (me.position[2].floor() as i32) >> 4];
        let a = Anchor { dimension: me.dimension.clone(), chunk, radius: self.anchor_radius(uuid) };
        let side = a.radius * 2 + 1;
        let mut out = vec![
            Effect::Anchor { owner, dimension: a.dimension.clone(), chunks: a.chunks(), hold: true },
            Effect::Tell(uuid, format!("Якір чанків поставлено: {side}×{side} чанків навколо тебе працюють, поки ти на сервері. Та сама клавіша знімає."), Tone::Good),
        ];
        self.data_mut(uuid).anchor = Some(a);
        out.extend(self.save(uuid));
        out
    }

    /// The player took damage at tick `now`: no spirit for a while.
    pub fn hurt(&mut self, uuid: u128, now: u64) {
        self.hurt_at.insert(uuid, now);
    }

    /// The harmful effects "clear mind" keeps off the player (empty without the perk).
    pub fn cleared(&self, uuid: u128) -> Vec<&'static str> {
        CLEAR_MIND.iter().take(self.tier(uuid, "clear_mind") as usize).flat_map(|l| l.iter().copied()).collect()
    }

    /// "Void walker" and "last stand": whether it saves the player now (at tick `now`); it then
    /// waits out its cooldown.
    pub fn rescue(&mut self, uuid: u128, perk: &str, now: u64) -> bool {
        let tier = self.tier(uuid, perk);
        let Some(p) = self.config.perk(perk) else {
            return false;
        };
        if tier == 0 {
            return false;
        }
        let key = (uuid, perk.to_owned());
        if self.ready_at.get(&key).is_some_and(|at| now < *at) {
            return false;
        }
        let wait = u64::from(rescue_seconds(p, tier)) * 20;
        self.ready_at.insert(key, now + wait);
        true
    }

    /// How many logs one cut fells for the player (0: no lumberjack).
    pub fn timber(&self, uuid: u128) -> usize {
        let tier = self.tier(uuid, "timber");
        self.config.perk("timber").filter(|_| tier > 0).map_or(0, |p| timber_logs(p, tier) as usize)
    }

    fn toggle_spirit(&mut self, uuid: u128, now: u64, me: &Seen) -> Vec<Effect> {
        let name = &me.name;
        if self.players[&uuid].spirit.is_some() {
            return self.spirit_return(uuid, name, "Ти повернувся в тіло");
        }
        if !safe_name(name) {
            return Vec::new();
        }
        if !matches!(me.game_mode, 0 | 2) {
            return vec![Effect::ActionBar(uuid, "Дух працює лише у виживанні".into())];
        }
        let wait = u64::from(self.config.settings.spirit_after_damage_seconds) * 20;
        if let Some(&at) = self.hurt_at.get(&uuid)
            && now < at + wait
        {
            let secs = (at + wait - now).div_ceil(20);
            return vec![Effect::ActionBar(uuid, format!("Після шкоди дух можна викликати через {secs} с"))];
        }
        let body = Body { dimension: me.dimension.clone(), position: me.position, yaw: me.yaw, pitch: me.pitch, game_mode: me.game_mode };
        let [x, y, z] = body.position;
        let tag = body_tag(uuid);
        let stand = format!(
            "execute in {} run summon minecraft:armor_stand {} {} {} {{Rotation:[{}f,0f],Invulnerable:1b,NoGravity:1b,\
             ShowArms:1b,NoBasePlate:1b,Tags:[\"progression_body\",\"{tag}\"],CustomName:\"{name}\",CustomNameVisible:1b,\
             equipment:{{head:{{id:\"minecraft:player_head\",count:1,components:{{\"minecraft:profile\":\"{name}\"}}}}}}}}",
            body.dimension,
            f3(x),
            f3(y),
            f3(z),
            f3(f64::from(body.yaw)),
        );
        self.data_mut(uuid).spirit = Some(body);
        let mut out = vec![
            Effect::Command(stand),
            Effect::Command(format!("gamemode spectator {name}")),
            Effect::ActionBar(uuid, format!("Ти дух: до {} блоків від тіла. Ще раз: назад", self.config.settings.spirit_radius)),
        ];
        out.extend(self.save(uuid));
        out
    }

    /// Back in the body: where it stands, in the mode they had; the stand goes.
    pub fn spirit_return(&mut self, uuid: u128, name: &str, line: &str) -> Vec<Effect> {
        let Some(body) = self.players.get(&uuid).and_then(|d| d.spirit.clone()) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        if safe_name(name) {
            let [x, y, z] = body.position;
            let mode = if body.game_mode == 2 { "adventure" } else { "survival" };
            out.push(Effect::Command(format!(
                "execute in {} run tp {name} {} {} {} {} {}",
                body.dimension,
                f3(x),
                f3(y),
                f3(z),
                f3(f64::from(body.yaw)),
                f3(f64::from(body.pitch))
            )));
            out.push(Effect::Command(format!("gamemode {mode} {name}")));
        }
        out.push(Effect::Command(format!("kill @e[type=minecraft:armor_stand,tag={}]", body_tag(uuid))));
        out.push(Effect::ActionBar(uuid, line.to_owned()));
        self.data_mut(uuid).spirit = None;
        out.extend(self.save(uuid));
        out
    }

    /// Every few ticks: spirits stay near their bodies; one whose mode was changed by someone
    /// else stops being a spirit.
    pub fn spirits(&mut self, online: &[Seen]) -> Vec<Effect> {
        let mut out = Vec::new();
        let r = self.config.settings.spirit_radius;
        for me in online {
            let Some(body) = self.players.get(&me.uuid).and_then(|d| d.spirit.clone()) else {
                continue;
            };
            if me.game_mode != 3 {
                // Not a spectator any more (a command, an admin): the body is theirs again.
                out.extend(self.spirit_return(me.uuid, &me.name, "Ти повернувся в тіло"));
                continue;
            }
            if !safe_name(&me.name) {
                continue;
            }
            let [bx, by, bz] = body.position;
            let [x, y, z] = me.position;
            let (dx, dy, dz) = (x - bx, y - by, z - bz);
            let dist = (dx * dx + dy * dy + dz * dz).sqrt();
            if me.dimension != body.dimension {
                out.push(Effect::Command(format!("execute in {} run tp {} {} {} {}", body.dimension, me.name, f3(bx), f3(by + 1.0), f3(bz))));
            } else if dist > r {
                let k = (r - 1.0) / dist;
                out.push(Effect::Command(format!(
                    "execute in {} run tp {} {} {} {}",
                    body.dimension,
                    me.name,
                    f3(bx + dx * k),
                    f3(by + dy * k),
                    f3(bz + dz * k)
                )));
                out.push(Effect::ActionBar(me.uuid, format!("Дух не відходить далі {r} блоків від тіла")));
            }
        }
        out
    }

    /// Whether the swap-hands key dashes for this player (they own a dash).
    pub fn swap_dashes(&self, uuid: u128) -> bool {
        self.config.settings.swap_hands_dash && ["dash_forward", "dash_up"].iter().any(|k| self.tier(uuid, k) > 0)
    }

    /// The dash F does: up when sneaking (or when it is the only one they own), else forward.
    pub fn swap_dash(&self, uuid: u128, sneaking: bool) -> Option<&'static str> {
        match (self.tier(uuid, "dash_forward") > 0, self.tier(uuid, "dash_up") > 0) {
            (true, true) => Some(if sneaking { "dash_up" } else { "dash_forward" }),
            (true, false) => Some("dash_forward"),
            (false, true) => Some("dash_up"),
            _ => None,
        }
    }
}

// The map of perks a Horizon client draws (the "serverkeys" mod).
impl Engine {
    /// The perk map for the player: lines of tab-separated fields. `T` title, points, open
    /// (1/0), level, experience, experience needed; `N` id, parent (`-` none), x, y, icon, tier,
    /// tiers, state (`branch`, `max`, `owned`, `buy`, `poor`, `locked`), name, about, price (`-`
    /// none), hint. Each branch is a column: its badge at the bottom, its perks climbing above.
    pub fn tree(&self, uuid: u128, open: bool) -> String {
        let clean = |t: &str| t.replace(['\t', '\n', '\r'], " ");
        let blank = PlayerData::default();
        let d = self.players.get(&uuid).unwrap_or(&blank);
        let maxed = d.level >= self.config.settings.max_level;
        let need = if maxed { 0 } else { self.config.xp_need(d.level) };
        let mut out = format!("T\tПрогресія · рівень {}\t{}\t{}\t{}\t{}\t{}\n", d.level, d.points, u8::from(open), d.level, d.xp, need);
        let branches = super::config::BRANCHES;
        for (b, branch) in branches.iter().enumerate() {
            let bx = (b as f32 - (branches.len() as f32 - 1.0) / 2.0) * 4.0;
            let (name, icon, about) = gui::branch(branch);
            let hub = format!("@{branch}");
            out.push_str(&format!("N\t{hub}\t-\t{bx:.2}\t3.00\t{icon}\t0\t0\tbranch\t{}\t{}\t-\t\n", clean(name), clean(about)));
            let perks: Vec<&Perk> = self.config.perks.iter().filter(|p| p.branch == *branch).collect();
            // A constellation: the stars climb from the badge in a loose zigzag, every other
            // branch mirrored, so the branches don't look stamped from one mould.
            let mirror = if b % 2 == 0 { 1.0 } else { -1.0 };
            for (k, p) in perks.iter().enumerate() {
                let (dx, dy) = STAR_STEPS[k % STAR_STEPS.len()];
                let x = bx + dx * mirror;
                let y = 3.0 + dy;
                let parent = match &p.parent {
                    Some(parent) if perks.iter().any(|q| &q.id == parent) => parent.clone(),
                    _ if k > 0 => perks[k - 1].id.clone(),
                    _ => hub.clone(),
                };
                let tier = d.tier(&p.id).min(p.max());
                let (frame, next) = self.perk_state(p, d);
                let state = match frame {
                    Frame::Max => "max",
                    Frame::Buy => "buy",
                    Frame::Poor => "poor",
                    Frame::Locked => "locked",
                    _ if tier > 0 => "owned",
                    _ => "locked",
                };
                let price = next.map_or("-".to_owned(), |(c, _)| c.to_string());
                let hint = match next {
                    Some((_, need)) if d.level < need => format!("Потрібен рівень прогресії {need}"),
                    Some(_) if frame == Frame::Buy => "Клік: вивчити".to_owned(),
                    Some(_) => "Не вистачає ОП".to_owned(),
                    None if p.is_item() => format!("Клік: ще одна за {} ванільних рівнів", p.item_levels),
                    None if p.is_skill() => "Працює на клавішу".to_owned(),
                    None => String::new(),
                };
                let about = self.perk_about(p, tier).replace(['\t', '\n'], " ");
                out.push_str(&format!(
                    "N\t{}\t{}\t{x:.2}\t{y:.2}\t{}\t{tier}\t{}\t{state}\t{}\t{}\t{price}\t{}\n",
                    p.id,
                    parent,
                    p.icon,
                    p.max(),
                    clean(&p.name),
                    clean(&about),
                    clean(&hint)
                ));
            }
        }
        out
    }
}
