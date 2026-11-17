//! The transmitters (`config/remoteredstone/transmitters.json`) and the power they give.

use super::config::Config;
use super::{Dim, Pos, TxKey, coords, dim_code, dim_name};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Transmitter {
    pub owner: u128,
    pub targets: Vec<Pos>,
    /// The signal it last read, 0 to 15: what its targets get.
    pub signal: u8,
}

/// What a click or a command did to a transmitter's targets.
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    Added(Pos),
    Removed(Pos),
}

#[derive(Serialize, Deserialize)]
struct Saved {
    dimension: String,
    pos: Pos,
    /// Hex, as JSON numbers lose a UUID's bits.
    owner: String,
    targets: Vec<Pos>,
    #[serde(default)]
    signal: u8,
}

#[derive(Default)]
pub struct Store {
    pub transmitters: HashMap<TxKey, Transmitter>,
    path: Option<PathBuf>,
}

impl Store {
    /// The transmitters saved in `dir`, and a warning when the file doesn't read (it is then
    /// set aside, not overwritten).
    pub fn open(dir: &Path) -> (Store, Option<String>) {
        let path = dir.join("transmitters.json");
        let mut store = Store { transmitters: HashMap::new(), path: Some(path.clone()) };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return (store, None);
        };
        match serde_json::from_str::<Vec<Saved>>(&text) {
            Ok(list) => {
                for s in list {
                    let (Some(dim), Ok(owner)) = (dim_code(&s.dimension), u128::from_str_radix(&s.owner, 16)) else {
                        continue;
                    };
                    store.transmitters.insert(
                        (dim, s.pos),
                        Transmitter { owner, targets: s.targets, signal: s.signal.min(15) },
                    );
                }
                (store, None)
            }
            Err(e) => {
                let aside = path.with_extension("json.broken");
                let _ = std::fs::rename(&path, &aside);
                (store, Some(format!("{}: {e}; moved to {}", path.display(), aside.display())))
            }
        }
    }

    /// Writes through a temporary file and a rename, so a crash never leaves half a file.
    pub fn save(&self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let mut list: Vec<Saved> = self
            .transmitters
            .iter()
            .map(|((dim, pos), t)| Saved {
                dimension: dim_name(*dim).to_owned(),
                pos: *pos,
                owner: format!("{:x}", t.owner),
                targets: t.targets.clone(),
                signal: t.signal,
            })
            .collect();
        list.sort_by(|a, b| (&a.dimension, a.pos).cmp(&(&b.dimension, b.pos)));
        let text = serde_json::to_string_pretty(&list).map_err(|e| e.to_string())?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// A new transmitter (or the one there, with these targets and owner). Targets that break the
    /// settings' rules are left out.
    pub fn place(&mut self, key: TxKey, owner: u128, targets: &[Pos], config: &Config) -> &Transmitter {
        let mut t = Transmitter { owner, targets: Vec::new(), signal: 0 };
        for p in targets {
            if t.targets.len() < config.max_targets && *p != key.1 && config.in_reach(key.1, *p) && !t.targets.contains(p) {
                t.targets.push(*p);
            }
        }
        self.transmitters.insert(key, t);
        &self.transmitters[&key]
    }

    /// Adds `pos` to the targets, or takes it out when it is there.
    pub fn toggle(&mut self, key: TxKey, pos: Pos, config: &Config) -> Result<Edit, String> {
        let t = self.transmitters.get_mut(&key).ok_or("Передавача тут уже немає")?;
        if let Some(i) = t.targets.iter().position(|p| *p == pos) {
            t.targets.remove(i);
            return Ok(Edit::Removed(pos));
        }
        Self::check(key, t, pos, config)?;
        t.targets.push(pos);
        Ok(Edit::Added(pos))
    }

    /// Adds each position not yet there: what was added, and why the others weren't.
    pub fn add(&mut self, key: TxKey, list: &[Pos], config: &Config) -> Result<(Vec<Pos>, Vec<String>), String> {
        let t = self.transmitters.get_mut(&key).ok_or("Передавача тут уже немає")?;
        let mut added = Vec::new();
        let mut refused = Vec::new();
        for p in list {
            if t.targets.contains(p) {
                continue;
            }
            match Self::check(key, t, *p, config) {
                Ok(()) => {
                    t.targets.push(*p);
                    added.push(*p);
                }
                Err(e) => refused.push(format!("{}: {e}", coords::show(*p))),
            }
        }
        Ok((added, refused))
    }

    /// Takes these positions out: how many were there.
    pub fn remove(&mut self, key: TxKey, list: &[Pos]) -> Result<usize, String> {
        let t = self.transmitters.get_mut(&key).ok_or("Передавача тут уже немає")?;
        let before = t.targets.len();
        t.targets.retain(|p| !list.contains(p));
        Ok(before - t.targets.len())
    }

    fn check(key: TxKey, t: &Transmitter, pos: Pos, config: &Config) -> Result<(), String> {
        if pos == key.1 {
            return Err("передавач не може живити сам себе".into());
        }
        if !config.in_reach(key.1, pos) {
            return Err(format!("далі ніж {} блоків від передавача", config.max_distance));
        }
        if t.targets.len() >= config.max_targets {
            return Err(format!("уже {} цілей, це максимум", config.max_targets));
        }
        Ok(())
    }

    /// Whether `who` (with operator `level`) may change or break the transmitter.
    pub fn may_edit(&self, key: TxKey, who: u128, level: u8, config: &Config) -> bool {
        self.transmitters.get(&key).is_some_and(|t| !config.owner_only || t.owner == who || level >= 2)
    }

    /// The transmitters of one level.
    pub fn in_dimension(&self, dim: Dim) -> Vec<TxKey> {
        let mut keys: Vec<TxKey> = self.transmitters.keys().filter(|k| k.0 == dim).copied().collect();
        keys.sort();
        keys
    }
}

/// Which positions get power, from which transmitter and how much: what the redstone asks, very
/// often, so it is kept ready and changed one transmitter at a time.
#[derive(Default, Debug)]
pub struct Power {
    at: HashMap<(Dim, Pos), Vec<(TxKey, u8)>>,
    /// Each powering transmitter's targets, as they are in `at`.
    from: HashMap<TxKey, Vec<Pos>>,
}

impl Power {
    /// The power from every transmitter of the store.
    pub fn of(store: &Store) -> Power {
        let mut p = Power::default();
        for (key, t) in &store.transmitters {
            p.set(*key, &t.targets, t.signal);
        }
        p
    }

    /// Transmitter `key` now gives `signal` to `targets` (0, or no targets: it gives nothing).
    pub fn set(&mut self, key: TxKey, targets: &[Pos], signal: u8) {
        if let Some(old) = self.from.remove(&key) {
            for pos in old {
                if let Some(list) = self.at.get_mut(&(key.0, pos)) {
                    list.retain(|(k, _)| *k != key);
                    if list.is_empty() {
                        self.at.remove(&(key.0, pos));
                    }
                }
            }
        }
        if signal == 0 || targets.is_empty() {
            return;
        }
        for pos in targets {
            self.at.entry((key.0, *pos)).or_default().push((key, signal.min(15)));
        }
        self.from.insert(key, targets.to_vec());
    }

    /// The strongest signal any transmitter but `except` gives the block at `pos`.
    pub fn signal_at(&self, dim: Dim, pos: Pos, except: Option<TxKey>) -> u8 {
        self.at
            .get(&(dim, pos))
            .map_or(0, |list| list.iter().filter(|(k, _)| Some(*k) != except).map(|(_, s)| *s).max().unwrap_or(0))
    }

    pub fn is_empty(&self) -> bool {
        self.at.is_empty()
    }

    /// Every powered position of a level.
    pub fn powered_in(&self, dim: Dim) -> Vec<Pos> {
        self.at.keys().filter(|(d, _)| *d == dim).map(|(_, p)| *p).collect()
    }
}
