//! The server's settings (`config/progression/progression.toml`) and the advancement weights
//! (`config/progression/advancements.toml`, made by `tools/gen_datapack.py`).

use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};

/// The settings file the mod writes on the first start.
pub const DEFAULT_CONFIG: &str = include_str!("../../data/progression.toml");

/// The rows of the perk menu, top to bottom.
pub const BRANCHES: [&str; 8] = ["survival", "pvp", "pve", "build", "redstone", "cart", "horse", "elytra"];
/// The first page of the perk menu shows this many branches, the travel page the rest.
pub const MAIN_BRANCHES: usize = 5;
/// Perks a branch row can show (the first column is the branch's own badge).
pub const ROW_PERKS: usize = 8;
/// Books the altar menu can show.
pub const MAX_BOOKS: usize = 18;

/// The settings' layout this version writes; an older file is set aside for the new defaults.
pub const CONFIG_VERSION: u32 = 8;

/// What the code does for a perk beyond its attributes.
pub const SPECIALS: [&str; 36] = [
    "wisdom", "hunter", "trophies", "scholar", "dash_forward", "dash_up", "spirit", "anchor", "pickaxe3", "shovel3",
    "clear_mind", "void_walker", "alchemist", "last_stand", "timber", "lab", "altar_tools", "altar_armor",
    "hard_head", "trowel", "mason", "draft", "relay", "cart_speed", "cart_drive", "cart_return", "horse_speed",
    "horse_descent", "horse_terrain", "horse_call", "horse_leaves", "horse_bags", "elytra_wear", "elytra_rockets",
    "elytra_wall", "elytra_glide",
];

/// The three altars, by the book group each opens.
pub const ALTARS: [(&str, &str); 3] = [("tools", "altar_tools"), ("armor", "altar_armor"), ("weapons", "altar_weapons")];

/// The altar perk of a book group.
pub fn altar_of(group: &str) -> Option<&'static str> {
    ALTARS.iter().find(|(g, _)| *g == group).map(|(_, p)| *p)
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub settings: Settings,
    /// Advancement id -> weight, winning over advancements.toml.
    #[serde(default)]
    pub advancement_points: BTreeMap<String, u32>,
    #[serde(default, rename = "perk")]
    pub perks: Vec<Perk>,
    #[serde(default, rename = "book")]
    pub books: Vec<Book>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    /// The layout of this file ([`CONFIG_VERSION`]).
    pub version: u32,
    pub max_level: u32,
    /// Experience from level n to n + 1: `xp_base + xp_step * n`.
    pub xp_base: u32,
    pub xp_step: u32,
    pub points_per_level: u32,
    /// Every `bonus_every` levels gives `bonus_points` more.
    pub bonus_every: u32,
    pub bonus_points: u32,
    pub respec_levels_per_point: u32,
    pub pvp_damage_multiplier: f32,
    pub swap_hands_dash: bool,
    pub dash_forward_speed: f32,
    pub dash_forward_lift: f32,
    pub dash_up_speed: f32,
    pub spirit_radius: f64,
    pub spirit_after_damage_seconds: u32,
    pub default_advancement_points: u32,
    pub advancement_xp_per_point: u32,
    /// The altar: this block standing on `altar_base`.
    pub altar_block: String,
    pub altar_base: String,
    /// The anvil never asks more levels than this (40 and more shows "Too Expensive").
    pub anvil_max_cost: i32,
    /// Chunks an anchor holds while its owner is offline too.
    pub anchor_offline: bool,
    /// The resource pack: served by the mod itself on `pack_port` (0: off), at
    /// `http://<pack_host>:<pack_port>/`, or `pack_url` when the pack is hosted elsewhere.
    pub pack_port: u16,
    pub pack_host: String,
    pub pack_url: String,
    pub pack_required: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            version: CONFIG_VERSION,
            max_level: 50,
            xp_base: 200,
            xp_step: 40,
            points_per_level: 2,
            bonus_every: 5,
            bonus_points: 3,
            respec_levels_per_point: 1,
            pvp_damage_multiplier: 1.5,
            swap_hands_dash: true,
            dash_forward_speed: 1.0,
            dash_forward_lift: 0.25,
            dash_up_speed: 1.3,
            spirit_radius: 32.0,
            spirit_after_damage_seconds: 10,
            default_advancement_points: 1,
            advancement_xp_per_point: 120,
            altar_block: "minecraft:enchanting_table".into(),
            altar_base: "minecraft:amethyst_block".into(),
            anvil_max_cost: 39,
            anchor_offline: false,
            pack_port: 25580,
            pack_host: "127.0.0.1".into(),
            pack_url: String::new(),
            pack_required: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Also {
    pub attribute: String,
    pub per_level: f64,
    #[serde(default = "add_value")]
    pub operation: String,
}

fn add_value() -> String {
    "add_value".into()
}

/// One perk: `costs[i]` points buy tier i + 1 once the player's level is `levels[i]`.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Perk {
    pub id: String,
    pub branch: String,
    pub name: String,
    pub icon: String,
    #[serde(default)]
    pub about: String,
    pub costs: Vec<u32>,
    pub levels: Vec<u32>,
    #[serde(default)]
    pub attribute: Option<String>,
    #[serde(default)]
    pub per_level: f64,
    #[serde(default = "add_value")]
    pub operation: String,
    #[serde(default)]
    pub also: Vec<Also>,
    /// What each tier adds for a special perk (percent, chunks...), for the menu's lines.
    #[serde(default)]
    pub amount: f64,
    #[serde(default)]
    pub cooldown_seconds: u32,
    /// Item perks: vanilla levels one more copy costs.
    #[serde(default)]
    pub item_levels: u32,
    /// Told to Horizon clients when owned (`lava_vision`).
    #[serde(default)]
    pub client: Option<String>,
    /// The perk the map draws it after (none: the one before it in its branch).
    #[serde(default)]
    pub parent: Option<String>,
}

impl Perk {
    pub fn max(&self) -> u32 {
        self.costs.len() as u32
    }
    pub fn modifier_id(&self) -> String {
        format!("progression:{}", self.id)
    }
    pub fn extra_id(&self, n: usize) -> String {
        format!("progression:{}_{}", self.id, n + 2)
    }
    /// Every attribute the perk sets, with its amount a tier.
    pub fn attributes(&self) -> impl Iterator<Item = (&str, f64)> {
        self.attribute
            .as_deref()
            .map(|a| (a, self.per_level))
            .into_iter()
            .chain(self.also.iter().map(|x| (x.attribute.as_str(), x.per_level)))
    }
    pub fn is_skill(&self) -> bool {
        matches!(self.id.as_str(), "dash_forward" | "dash_up" | "spirit" | "anchor" | "lab")
    }
    /// Bought once, no tiers.
    pub fn single(&self) -> bool {
        matches!(self.id.as_str(), "dash_forward" | "dash_up" | "spirit" | "pickaxe3" | "shovel3")
    }
    pub fn is_item(&self) -> bool {
        matches!(self.id.as_str(), "pickaxe3" | "shovel3")
    }
}

/// An enchantment whose books go past vanilla's top level at the altar.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Book {
    pub enchant: String,
    pub name: String,
    /// Vanilla's top level.
    pub vanilla_max: u32,
    /// `tools`, `armor` or `weapons`: each tier of that group's altar perk adds a level.
    pub group: String,
    /// How far above vanilla's top this book can go (protection: 1).
    pub max_extra: u32,
    /// Vanilla levels one book costs.
    pub price_levels: u32,
}

impl Book {
    pub fn key(&self) -> &str {
        self.enchant.split_once(':').map_or(&self.enchant, |(_, p)| p)
    }
}

/// The book group an altar perk opens.
pub fn altar_group(perk: &str) -> Option<&'static str> {
    ALTARS.iter().find(|(_, p)| *p == perk).map(|(g, _)| *g)
}

/// The `[settings] version` of a settings text: 0 when it has none, None when it isn't TOML.
pub fn file_version(text: &str) -> Option<u32> {
    let table: toml::Table = toml::from_str(text).ok()?;
    let v = table.get("settings").and_then(|s| s.get("version")).and_then(|v| v.as_integer()).unwrap_or(0);
    Some(u32::try_from(v).unwrap_or(0))
}

const OPERATIONS: [&str; 3] = ["add_value", "add_multiplied_base", "add_multiplied_total"];

fn item_id(s: &str) -> bool {
    s.split_once(':').is_some_and(|(ns, path)| {
        !ns.is_empty()
            && !path.is_empty()
            && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | ':' | '.' | '/' | '-'))
    })
}

fn word(s: &str) -> bool {
    !s.is_empty() && s.len() <= 32 && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

impl Config {
    pub fn parse(text: &str) -> Result<Config, String> {
        let c: Config = toml::from_str(text).map_err(|e| format!("progression.toml: {e}"))?;
        c.check()?;
        Ok(c)
    }

    fn check(&self) -> Result<(), String> {
        let s = &self.settings;
        if s.max_level == 0 || s.max_level > 1000 {
            return Err("settings.max_level: 1 to 1000".into());
        }
        if s.xp_base == 0 {
            return Err("settings.xp_base: at least 1".into());
        }
        if !s.pvp_damage_multiplier.is_finite() || !(0.0..=100.0).contains(&s.pvp_damage_multiplier) {
            return Err("settings.pvp_damage_multiplier: 0 to 100".into());
        }
        for (k, v) in [("dash_forward_speed", s.dash_forward_speed), ("dash_forward_lift", s.dash_forward_lift), ("dash_up_speed", s.dash_up_speed)] {
            if !v.is_finite() || !(0.0..=4.0).contains(&v) {
                return Err(format!("settings.{k}: 0 to 4"));
            }
        }
        if !s.spirit_radius.is_finite() || !(4.0..=512.0).contains(&s.spirit_radius) {
            return Err("settings.spirit_radius: 4 to 512".into());
        }
        if !item_id(&s.altar_block) || !item_id(&s.altar_base) {
            return Err("settings.altar_block / altar_base: block ids like minecraft:enchanting_table".into());
        }
        if !(1..=1000).contains(&s.anvil_max_cost) {
            return Err("settings.anvil_max_cost: 1 to 1000".into());
        }
        if s.pack_host.chars().any(|c| c.is_whitespace() || c == '/') {
            return Err("settings.pack_host: a host name or address only".into());
        }
        let mut ids = HashSet::new();
        let mut rows: BTreeMap<&str, usize> = BTreeMap::new();
        for p in &self.perks {
            if !word(&p.id) {
                return Err(format!("perk id {:?}: lowercase letters, digits and _", p.id));
            }
            if !ids.insert(p.id.as_str()) {
                return Err(format!("perk {}: listed twice", p.id));
            }
            if !BRANCHES.contains(&p.branch.as_str()) {
                return Err(format!("perk {}: branch is one of {}", p.id, BRANCHES.join(", ")));
            }
            let n = rows.entry(p.branch.as_str()).or_default();
            *n += 1;
            if *n > ROW_PERKS {
                return Err(format!("branch {}: at most {ROW_PERKS} perks", p.branch));
            }
            if p.costs.is_empty() || p.costs.len() > 10 || p.costs.len() != p.levels.len() {
                return Err(format!("perk {}: costs and levels need 1 to 10 entries each, the same number", p.id));
            }
            if p.levels.windows(2).any(|w| w[1] < w[0]) {
                return Err(format!("perk {}: levels must not go down", p.id));
            }
            if !item_id(&p.icon) {
                return Err(format!("perk {}: icon is an item id", p.id));
            }
            if p.attribute.is_none() && !SPECIALS.contains(&p.id.as_str()) && altar_group(&p.id).is_none() && p.client.is_none() {
                return Err(format!("perk {}: needs an attribute (or one of the built-in ids: {})", p.id, SPECIALS.join(", ")));
            }
            for (a, per, op) in p
                .attribute
                .iter()
                .map(|a| (a, p.per_level, &p.operation))
                .chain(p.also.iter().map(|x| (&x.attribute, x.per_level, &x.operation)))
            {
                if !item_id(a) || !per.is_finite() || !OPERATIONS.contains(&op.as_str()) {
                    return Err(format!("perk {}: attribute {a}: an id, a number, and one of {}", p.id, OPERATIONS.join(", ")));
                }
            }
            if p.single() && p.costs.len() != 1 {
                return Err(format!("perk {}: one tier only", p.id));
            }
            if p.client.as_deref().is_some_and(|c| !word(c)) {
                return Err(format!("perk {}: client is a word", p.id));
            }
        }
        if self.books.len() > MAX_BOOKS {
            return Err(format!("at most {MAX_BOOKS} books"));
        }
        let mut seen = HashSet::new();
        for b in &self.books {
            if !item_id(&b.enchant) || !seen.insert(b.enchant.as_str()) {
                return Err(format!("book {:?}: an enchantment id, listed once", b.enchant));
            }
            if altar_of(&b.group).is_none() {
                return Err(format!("book {}: group is tools, armor or weapons", b.enchant));
            }
            if !(1..=10).contains(&b.max_extra) {
                return Err(format!("book {}: max_extra 1 to 10", b.enchant));
            }
            if b.vanilla_max == 0 || b.vanilla_max > 250 {
                return Err(format!("book {}: vanilla_max 1 to 250", b.enchant));
            }
        }
        Ok(())
    }

    pub fn perk(&self, id: &str) -> Option<&Perk> {
        self.perks.iter().find(|p| p.id == id)
    }

    pub fn book(&self, enchant: &str) -> Option<&Book> {
        self.books.iter().find(|b| b.enchant == enchant)
    }

    /// Experience from `level` to the next one.
    pub fn xp_need(&self, level: u32) -> u64 {
        u64::from(self.settings.xp_base) + u64::from(self.settings.xp_step) * u64::from(level)
    }

    /// Points reaching `level` gives.
    pub fn points_at(&self, level: u32) -> u32 {
        let s = &self.settings;
        let bonus = if s.bonus_every > 0 && level % s.bonus_every == 0 { s.bonus_points } else { 0 };
        s.points_per_level + bonus
    }

    /// Points a player has earned by `level`, from level 0.
    pub fn points_by(&self, level: u32) -> u32 {
        (1..=level).map(|l| self.points_at(l)).sum()
    }

    /// What the whole tree costs: every perk tier.
    pub fn tree_cost(&self) -> u32 {
        self.perks.iter().flat_map(|p| p.costs.iter()).sum()
    }
}

/// `advancements.toml`: `[points]` advancement id -> weight, made from the game's own
/// advancements (task 1, goal 3, challenge 5).
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Rewards {
    #[serde(default)]
    pub points: BTreeMap<String, u32>,
}

impl Rewards {
    pub fn parse(text: &str) -> Result<Rewards, String> {
        toml::from_str(text).map_err(|e| format!("advancements.toml: {e}"))
    }
}

/// Whether an advancement can ever be rewarded: recipe unlocks and the tabs' roots never are.
pub fn rewardable(id: &str) -> bool {
    let path = id.split_once(':').map_or(id, |(_, p)| p);
    !(path.starts_with("recipes/") || path.ends_with("/root") || path.is_empty())
}
