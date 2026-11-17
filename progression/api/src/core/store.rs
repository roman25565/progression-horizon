//! Each player's progress, one JSON file per player in `config/progression/players/`.
//! Only the server writes these files, so players can't change their progress.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlayerData {
    /// The name they last had (for admins reading the files).
    pub name: String,
    /// The progression level (never lost on death) and the experience toward the next one.
    pub level: u32,
    pub xp: u64,
    /// All the experience they ever earned.
    pub total_xp: u64,
    /// Points they can spend now.
    pub points: u32,
    /// Perk id -> tier bought.
    pub perks: BTreeMap<String, u32>,
    /// Enchantment id -> levels above vanilla's top opened at the altar.
    pub books: BTreeMap<String, u32>,
    /// Advancements already rewarded.
    pub quests: BTreeSet<String>,
    /// How many times they reset their perks.
    pub respecs: u32,
    /// Where their body stays while they are a spirit (kept on disk, so a crash or a logout
    /// brings them back to it).
    pub spirit: Option<Body>,
    /// Their chunk anchor.
    pub anchor: Option<Anchor>,
    /// Their redstone lab zone.
    pub zone: Option<Zone>,
    /// Their drawing ("Кресляр"): two points and a shape.
    pub draft: Option<Draft>,
    /// Their horse ("Поклик", "Сідельні сумки"): the last one they rode.
    pub horse: Option<Horse>,
    /// The saddlebags' slots as `ItemStack.CODEC` SNBT ("-" empty). Kept through a respec.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub bag: Vec<String>,
    /// The perk layout their perks were bought under (an older one is refunded).
    pub version: u32,
    /// Progression 0.x files: points ever earned, turned into experience once.
    #[serde(skip_serializing_if = "is_zero")]
    pub earned: u32,
    #[serde(skip_serializing)]
    pub chains: BTreeMap<String, u32>,
    #[serde(skip_serializing)]
    pub skills: BTreeSet<String>,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Anchor {
    pub dimension: String,
    pub chunk: [i32; 2],
    pub radius: i32,
}

impl Anchor {
    pub fn chunks(&self) -> Vec<[i32; 2]> {
        let r = self.radius.clamp(0, 4);
        let mut out = Vec::new();
        for dx in -r..=r {
            for dz in -r..=r {
                out.push([self.chunk[0] + dx, self.chunk[1] + dz]);
            }
        }
        out
    }
}

/// A player's horse: its UUID, and where it was last seen.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Horse {
    /// The entity's UUID, as hex.
    pub uuid: String,
    pub kind: String,
    pub dimension: String,
    pub pos: [f64; 3],
}

/// A drawing between two points: `line`, `box` (the edges of the box between them) or
/// `circle` (around `a`, through `b`'s distance, in `a`'s layer).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Draft {
    pub dimension: String,
    pub a: Option<[i32; 3]>,
    pub b: Option<[i32; 3]>,
    #[serde(default = "line")]
    pub shape: String,
}

fn line() -> String {
    "line".to_owned()
}

/// A redstone lab: a cube of `radius` blocks around `center` where blasts break nothing and
/// (when `freeze`) observers don't fire.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Zone {
    pub dimension: String,
    pub center: [i32; 3],
    pub radius: i32,
    #[serde(default = "yes")]
    pub no_blast: bool,
    #[serde(default)]
    pub freeze: bool,
}

fn yes() -> bool {
    true
}

impl Zone {
    pub fn contains(&self, dimension: &str, pos: [i32; 3]) -> bool {
        self.dimension == dimension && (0..3).all(|i| (pos[i] - self.center[i]).abs() <= self.radius)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Body {
    pub dimension: String,
    pub position: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    /// 0 survival, 2 adventure: what they go back to.
    pub game_mode: i32,
}

impl PlayerData {
    /// The tier of a perk they own (0: none).
    pub fn tier(&self, perk: &str) -> u32 {
        self.perks.get(perk).copied().unwrap_or(0)
    }
    /// Levels above vanilla's top they opened for an enchantment.
    pub fn book(&self, enchant: &str) -> u32 {
        self.books.get(enchant).copied().unwrap_or(0)
    }
}

pub fn uuid_hex(uuid: u128) -> String {
    format!("{uuid:032x}")
}

pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: &Path) -> Store {
        Store { dir: dir.join("players") }
    }

    fn path(&self, uuid: u128) -> PathBuf {
        self.dir.join(format!("{}.json", uuid_hex(uuid)))
    }

    /// The player's progress; a fresh one when there is no file. A file that can't be read is
    /// kept aside as `<uuid>.json.broken-<n>` so nothing is lost, and the error is returned.
    pub fn load(&self, uuid: u128) -> (PlayerData, Option<String>) {
        let path = self.path(uuid);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (PlayerData::default(), None),
            Err(e) => return (PlayerData::default(), Some(format!("{}: {e}", path.display()))),
        };
        match serde_json::from_str(&text) {
            Ok(d) => (d, None),
            Err(e) => {
                let mut n = 1;
                let aside = loop {
                    let p = path.with_extension(format!("json.broken-{n}"));
                    if !p.exists() {
                        break p;
                    }
                    n += 1;
                };
                let _ = std::fs::rename(&path, &aside);
                (PlayerData::default(), Some(format!("{}: {e} (kept as {})", path.display(), aside.display())))
            }
        }
    }

    /// Writes through a temporary file and a rename, so a crash never leaves half a file.
    pub fn save(&self, uuid: u128, data: &PlayerData) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| format!("{}: {e}", self.dir.display()))?;
        let path = self.path(uuid);
        let tmp = path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(data).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Every saved player's uuid (for admin commands on players who are offline).
    pub fn find_by_name(&self, name: &str) -> Option<(u128, PlayerData)> {
        let entries = std::fs::read_dir(&self.dir).ok()?;
        for e in entries.flatten() {
            let file = e.file_name();
            let Some(stem) = file.to_str().and_then(|f| f.strip_suffix(".json")) else {
                continue;
            };
            let Ok(uuid) = u128::from_str_radix(stem, 16) else {
                continue;
            };
            let (d, err) = self.load(uuid);
            if err.is_none() && d.name.eq_ignore_ascii_case(name) {
                return Some((uuid, d));
            }
        }
        None
    }
}
