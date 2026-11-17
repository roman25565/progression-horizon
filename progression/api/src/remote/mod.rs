//! Remote redstone (RemoteRedstone 1.0, now part of progression): a transmitter block (placed from its own item) gives the signal it
//! receives to every block whose coordinates it holds, as a lever beside each of them would.
//! The rules are pure Rust in `core/`, tested on their own (`core-tests/`). Progression owns it:
//! its perk «Дистанційний сигнал» says who may craft, place and edit transmitters, and how many
//! targets how far (`set_limits`); operators may without it, with the settings' limits.
//!
//! Nothing is polled. A transmitter reads its input only when the game tells one of its
//! neighbours changed (`neighbor_changed`), at the end of that level's tick, and only a changed
//! signal touches its targets (`update_signal_neighbors`). The redstone asks every block's signal
//! very often (`block_signal`): with no transmitter powered that is one atomic load, else one
//! lookup in a map of the powered positions.
//!
//! Locks: nothing of the host is called while the state's, the queue's or the power's lock is
//! held (a host call runs the game, which runs this mod's hooks again).

mod core;

use self::core::config::Config;
use self::core::menu::{self, Action, Cell};
use self::core::store::{Edit, Power, Store};
use self::core::{Dim, Pos, TxKey, coords, dim_code, dim_name, step};
use horizon_client::mod_api::*;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Mutex, MutexGuard, RwLock};

const LOG: &str = "horizon::remoteredstone";
/// The item's `minecraft:custom_data` key; its value is the targets (`x y z;x y z`).
const MARK: &str = "remote_redstone";
const DIALOG: &str = "remoteredstone:coords";
const NAME: &str = "Дистанційний редстоун";

const RED: u32 = 0xFF5555;
const GREEN: u32 = 0x55FF55;
const YELLOW: u32 = 0xFFFF55;
const GRAY: u32 = 0xAAAAAA;
const NEED: &str = "Потрібен перк «Дистанційний сигнал» (гілка редстоуну)";

thread_local! {
    /// The transmitter reading its input right now: its own power doesn't count, or a target
    /// beside it would keep it on for ever.
    static READING: std::cell::Cell<Option<TxKey>> = const { std::cell::Cell::new(None) };
}

#[derive(Clone, Copy)]
struct OpenMenu {
    window: i32,
    tx: TxKey,
    page: usize,
}

struct State {
    config: Config,
    store: Store,
    menus: HashMap<u128, OpenMenu>,
    /// The transmitter a player last opened: what `/rr add` and the dialog change.
    editing: HashMap<u128, TxKey>,
    /// Players picking targets by clicking: their transmitter, and the tick the mode ends.
    linking: HashMap<u128, (TxKey, u64)>,
    /// One answer for both hands' turns of one click.
    last_click: HashMap<u128, (u64, Pos)>,
    /// Levels whose transmitters were checked once since the start.
    synced: [bool; 3],
    dirty: bool,
}

/// What waits for the server's tick: most hooks run with one player's state swapped in, where
/// menus can't open and other players can't be told.
#[derive(Default)]
struct Queue {
    opens: Vec<(u128, TxKey, usize)>,
    notes: Vec<(u128, String, u32, bool)>,
    commands: Vec<(u128, String)>,
}

#[derive(Default)]
pub struct Remote {
    /// Players with the progression perk: their most targets and distance per axis (0 none).
    perks: RwLock<HashMap<u128, (usize, i32)>>,
    /// None until the first hook with a host reads the settings.
    state: Mutex<Option<State>>,
    power: RwLock<Power>,
    any_power: AtomicBool,
    /// Every transmitter's position, for the hooks that the game calls very often.
    txs: RwLock<HashSet<TxKey>>,
    any_tx: AtomicBool,
    /// Transmitters to read at their level's tick end; true: the block is loaded, so a
    /// transmitter whose block is gone is removed.
    pending: Mutex<HashMap<TxKey, bool>>,
    any_pending: AtomicBool,
    /// Targets whose neighbours read their signal again at their level's tick end.
    refresh: Mutex<Vec<(Dim, Pos)>>,
    any_refresh: AtomicBool,
    queue: Mutex<Queue>,
    ticks: AtomicU64,
}

fn relock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic while a lock was held must not take every later hook down with it.
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn level_code(d: LevelDimension) -> Dim {
    match d {
        LevelDimension::Overworld => 0,
        LevelDimension::Nether => 1,
        LevelDimension::End => 2,
    }
}

fn normal(face: Direction) -> Pos {
    match face {
        Direction::Down => [0, -1, 0],
        Direction::Up => [0, 1, 0],
        Direction::North => [0, 0, -1],
        Direction::South => [0, 0, 1],
        Direction::West => [-1, 0, 0],
        Direction::East => [1, 0, 0],
    }
}

fn at(p: &BlockPos) -> Pos {
    [p.x, p.y, p.z]
}

fn stack(cell: &Cell) -> ItemStack {
    let mut s = ItemStack::new(&cell.icon, cell.count.clamp(1, 64));
    let data = s.data_mut();
    data.set_loot_component(
        "minecraft:custom_name",
        &serde_json::json!({ "text": cell.name, "italic": false, "color": cell.color }),
    );
    let lore: Vec<serde_json::Value> = cell
        .lore
        .iter()
        .map(|(t, c)| serde_json::json!({ "text": t, "italic": false, "color": c }))
        .collect();
    if !lore.is_empty() {
        data.set_loot_component("minecraft:lore", &serde_json::Value::Array(lore));
    }
    if cell.glint {
        data.set_loot_component("minecraft:enchantment_glint_override", &serde_json::Value::Bool(true));
    }
    s
}

/// The components that make a block item a transmitter: the same in the recipe and in a drop.
fn components(targets: &[Pos]) -> serde_json::Value {
    let mut lore = vec![
        serde_json::json!({ "text": "Живить блоки за координатами", "italic": false, "color": "gray" }),
        serde_json::json!({ "text": "ПКМ по поставленому: меню", "italic": false, "color": "dark_gray" }),
    ];
    if !targets.is_empty() {
        lore.push(serde_json::json!({ "text": format!("Цілей: {}", targets.len()), "italic": false, "color": "gold" }));
    }
    serde_json::json!({
        "minecraft:custom_data": { MARK: coords::encode(targets) },
        "minecraft:custom_name": { "text": NAME, "italic": false, "color": "red" },
        "minecraft:lore": lore,
        "minecraft:enchantment_glint_override": true,
    })
}

fn transmitter_item(block: &str, targets: &[Pos]) -> ItemStack {
    let mut s = ItemStack::new(block, 1);
    let data = s.data_mut();
    if let serde_json::Value::Object(map) = components(targets) {
        for (id, value) in &map {
            data.set_loot_component(id, value);
        }
    }
    s
}

/// A transmitter item's targets; None for any other stack.
fn marked(s: &ItemStack) -> Option<Vec<Pos>> {
    let tag = s.data()?.other("minecraft:custom_data")?.get(MARK)?;
    Some(match tag {
        Tag::String(text) => coords::decode(&text.to_string_lossy()),
        _ => Vec::new(),
    })
}

fn recipe(block: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "minecraft:crafting_shaped",
        "category": "redstone",
        "pattern": [" E ", "RLR", " R "],
        "key": { "E": "minecraft:ender_pearl", "R": "minecraft:redstone", "L": block },
        "result": { "id": block, "count": 1, "components": components(&[]) }
    })
}

fn dialog() -> serde_json::Value {
    serde_json::json!({
        "type": "minecraft:notice",
        "title": { "text": NAME, "color": "red" },
        "body": [{
            "type": "minecraft:plain_message",
            "width": 300,
            "contents": { "text": "Координати блоків: x y z. Кілька блоків через ; (1 64 2; 5 64 2). ~ рахує від передавача: ~ ~1 ~ це блок над ним." }
        }],
        "inputs": [{
            "type": "minecraft:text",
            "key": "pos",
            "label": { "text": "x y z" },
            "width": 300,
            "max_length": 1000
        }],
        "action": {
            "label": { "text": "Додати" },
            "action": { "type": "minecraft:dynamic/run_command", "template": "rr add $(pos)" }
        },
        "can_close_with_escape": true,
        "after_action": "close"
    })
}

const HELP: [&str; 7] = [
    "ПКМ по передавачу: меню з координатами (Shift+ПКМ: поставити блок поруч)",
    "/rr add x y z [; x y z]: додати цілі (~ від передавача)",
    "/rr remove x y z: прибрати ціль",
    "/rr list, /rr clear: список, прибрати всі",
    "/rr link: вибирати цілі ПКМ по блоках, /rr done: готово",
    "/rr menu: знову відкрити останній передавач",
    "/rr give [гравець]: дати передавач (оператори)",
];

impl Remote {
    /// A new transmitter with no targets, of the configured block.
    pub fn transmitter(&self) -> ItemStack {
        let block = self.state().as_ref().map_or_else(|| "minecraft:redstone_lamp".to_owned(), |s| s.config.block.clone());
        transmitter_item(&block, &[])
    }

    /// The perk's tier changed (or the player came): their limits, None without the perk.
    pub fn set_limits(&self, uuid: u128, limits: Option<(usize, i32)>) {
        let mut p = self.perks.write().unwrap_or_else(|p| p.into_inner());
        match limits {
            Some(l) => p.insert(uuid, l),
            None => p.remove(&uuid),
        };
    }

    fn limits(&self, uuid: u128) -> Option<(usize, i32)> {
        self.perks.read().unwrap_or_else(|p| p.into_inner()).get(&uuid).copied()
    }

    /// Whether a player may make and edit transmitters: the perk, or an operator.
    fn allowed(&self, uuid: u128, permission: u8) -> bool {
        permission >= 2 || self.limits(uuid).is_some()
    }

    /// The settings with a transmitter owner's limits from the perk.
    fn rules(&self, config: &Config, owner: u128) -> Config {
        let mut c = config.clone();
        if let Some((targets, distance)) = self.limits(owner) {
            c.max_targets = targets.clamp(1, 1024);
            c.max_distance = distance.max(0);
        }
        c
    }

    fn state(&self) -> MutexGuard<'_, Option<State>> {
        relock(&self.state)
    }

    /// Reads the settings and the transmitters once: false when the config folder is unknown.
    fn open(&self, host: &dyn Host) -> bool {
        let mut g = self.state();
        if g.is_some() {
            return true;
        }
        let dir = match host.config_dir() {
            Ok(dir) => dir.join("remoteredstone"),
            Err(e) => {
                log::error!(target: LOG, "[RemoteRedstone] no config folder: {e}");
                return false;
            }
        };
        let (config, warning) = Config::load(&dir);
        if let Some(w) = warning {
            log::warn!(target: LOG, "[RemoteRedstone] {w}");
        }
        let (store, warning) = Store::open(&dir);
        if let Some(w) = warning {
            log::warn!(target: LOG, "[RemoteRedstone] {w}");
        }
        log::info!(target: LOG, "[RemoteRedstone] {} transmitters, block {}", store.transmitters.len(), config.block);
        {
            let power = Power::of(&store);
            self.any_power.store(!power.is_empty(), Relaxed);
            *self.power.write().unwrap_or_else(|p| p.into_inner()) = power;
            let keys: HashSet<TxKey> = store.transmitters.keys().copied().collect();
            self.any_tx.store(!keys.is_empty(), Relaxed);
            *self.txs.write().unwrap_or_else(|p| p.into_inner()) = keys;
        }
        *g = Some(State {
            config,
            store,
            menus: HashMap::new(),
            editing: HashMap::new(),
            linking: HashMap::new(),
            last_click: HashMap::new(),
            synced: [false; 3],
            dirty: false,
        });
        true
    }

    fn is_tx(&self, key: TxKey) -> bool {
        self.any_tx.load(Relaxed) && self.txs.read().unwrap_or_else(|p| p.into_inner()).contains(&key)
    }

    fn set_txs(&self, key: TxKey, present: bool) {
        let mut txs = self.txs.write().unwrap_or_else(|p| p.into_inner());
        if present {
            txs.insert(key);
        } else {
            txs.remove(&key);
        }
        self.any_tx.store(!txs.is_empty(), Relaxed);
    }

    fn set_power(&self, key: TxKey, targets: &[Pos], signal: u8) {
        let mut p = self.power.write().unwrap_or_else(|p| p.into_inner());
        p.set(key, targets, signal);
        self.any_power.store(!p.is_empty(), Relaxed);
    }

    fn check_later(&self, key: TxKey, verify: bool) {
        let mut p = relock(&self.pending);
        let v = p.entry(key).or_insert(verify);
        *v |= verify;
        self.any_pending.store(true, Relaxed);
    }

    fn refresh_later(&self, dim: Dim, list: impl IntoIterator<Item = Pos>) {
        let mut r = relock(&self.refresh);
        r.extend(list.into_iter().map(|p| (dim, p)));
        self.any_refresh.store(!r.is_empty(), Relaxed);
    }

    fn note(&self, uuid: u128, text: impl Into<String>, color: u32) {
        relock(&self.queue).notes.push((uuid, text.into(), color, false));
    }

    fn bar(&self, uuid: u128, text: impl Into<String>, color: u32) {
        relock(&self.queue).notes.push((uuid, text.into(), color, true));
    }

    /// A transmitter's targets changed from `before`: the power follows, the blocks that gained
    /// or lost it are refreshed at their level's tick end, and the file is written.
    fn edited(&self, key: TxKey, before: &[Pos]) {
        let (after, signal) = {
            let mut g = self.state();
            let Some(s) = g.as_mut() else {
                return;
            };
            let Some(t) = s.store.transmitters.get(&key) else {
                return;
            };
            let out = (t.targets.clone(), t.signal);
            if let Err(e) = s.store.save() {
                log::warn!(target: LOG, "[RemoteRedstone] can't save: {e}");
            }
            out
        };
        if signal == 0 {
            return;
        }
        self.set_power(key, &after, signal);
        let changed: Vec<Pos> = before
            .iter()
            .filter(|p| !after.contains(p))
            .chain(after.iter().filter(|p| !before.contains(p)))
            .copied()
            .collect();
        self.refresh_later(key.0, changed);
    }

    /// The transmitter is gone: its targets lose its power.
    fn forget(&self, key: TxKey) {
        let gone = {
            let mut g = self.state();
            let Some(s) = g.as_mut() else {
                return;
            };
            let Some(t) = s.store.transmitters.remove(&key) else {
                return;
            };
            s.menus.retain(|_, m| m.tx != key);
            s.editing.retain(|_, k| *k != key);
            s.linking.retain(|_, (k, _)| *k != key);
            if let Err(e) = s.store.save() {
                log::warn!(target: LOG, "[RemoteRedstone] can't save: {e}");
            }
            t
        };
        self.set_txs(key, false);
        self.set_power(key, &[], 0);
        if gone.signal > 0 {
            self.refresh_later(key.0, gone.targets);
        }
    }

    /// Reads one transmitter's input; the level is in place. `verify`: its block is loaded, so a
    /// different block there means the transmitter is gone.
    fn check(&self, host: &mut dyn Host, key: TxKey, verify: bool) {
        let block = match self.state().as_ref() {
            Some(s) if s.store.transmitters.contains_key(&key) => s.config.block.clone(),
            _ => return,
        };
        if host.block(key.1) != block {
            if verify {
                self.forget(key);
            }
            return;
        }
        READING.with(|r| r.set(Some(key)));
        let signal = host.neighbor_signal(dim_name(key.0), key.1).clamp(0, 15) as u8;
        READING.with(|r| r.set(None));
        let targets = {
            let mut g = self.state();
            let Some(s) = g.as_mut() else {
                return;
            };
            let Some(t) = s.store.transmitters.get_mut(&key) else {
                return;
            };
            if t.signal == signal {
                return;
            }
            t.signal = signal;
            s.dirty = true;
            t.targets.clone()
        };
        self.set_power(key, &targets, signal);
        for pos in targets {
            host.update_signal_neighbors(dim_name(key.0), pos);
        }
    }

    /// Opens (or turns the page of) a transmitter's menu.
    fn show(&self, host: &mut dyn Host, uuid: u128, key: TxKey, page: usize) {
        let (items, page, same) = {
            let g = self.state();
            let Some(s) = g.as_ref() else {
                return;
            };
            let Some(t) = s.store.transmitters.get(&key) else {
                return;
            };
            let page = page.min(menu::pages(t) - 1);
            let items: Vec<Option<ItemStack>> =
                menu::cells(t, key.1, page, self.rules(&s.config, t.owner).max_targets).iter().map(|c| c.as_ref().map(stack)).collect();
            (items, page, s.menus.get(&uuid).copied().filter(|m| m.tx == key))
        };
        if let Some(open) = same {
            host.set_menu_items(uuid, items);
            if let Some(s) = self.state().as_mut() {
                s.menus.insert(uuid, OpenMenu { page, ..open });
            }
            return;
        }
        // No lock held: opening closes the old menu, and its `menu_closed` comes in here.
        if let Some(window) = host.open_menu(uuid, menu::ROWS, Text::literal(menu::TITLE.to_owned()), items)
            && let Some(s) = self.state().as_mut()
        {
            s.menus.insert(uuid, OpenMenu { window, tx: key, page });
            s.editing.insert(uuid, key);
        }
    }

    fn command(&self, host: &mut dyn Host, uuid: u128, line: &str) {
        let Some(me) = host.server_players().into_iter().find(|p| p.uuid == uuid) else {
            return;
        };
        let line = line.trim();
        let (word, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let rest = rest.trim();
        let word = word.to_ascii_lowercase();
        if word == "give" {
            if me.permission < 2 {
                return self.note(uuid, "Лише для операторів", RED);
            }
            let to = if rest.is_empty() {
                Some(me.clone())
            } else {
                host.server_players().into_iter().find(|p| p.name.eq_ignore_ascii_case(rest))
            };
            let Some(to) = to else {
                return self.note(uuid, format!("Гравця {rest} немає на сервері"), RED);
            };
            let block = self.state().as_ref().map(|s| s.config.block.clone()).unwrap_or_default();
            if let Some(left) = host.give_stack(to.uuid, transmitter_item(&block, &[])) {
                let _ = left;
                return self.note(uuid, format!("У {} немає місця в інвентарі", to.name), RED);
            }
            return self.note(uuid, format!("{} отримав передавач", to.name), GREEN);
        }
        if matches!(word.as_str(), "help" | "?") {
            for l in HELP {
                self.note(uuid, l, YELLOW);
            }
            return;
        }
        let mut g = self.state();
        let Some(s) = g.as_mut() else {
            return;
        };
        let editing = s.editing.get(&uuid).copied().filter(|k| s.store.transmitters.contains_key(k));
        let Some(key) = editing else {
            drop(g);
            self.note(uuid, "Спочатку відкрий передавач: ПКМ по ньому. /rr help: команди", RED);
            return;
        };
        if !s.store.may_edit(key, uuid, me.permission, &s.config) {
            drop(g);
            return self.note(uuid, "Це чужий передавач", RED);
        }
        if !self.allowed(uuid, me.permission) {
            drop(g);
            return self.note(uuid, NEED, RED);
        }
        let rules = self.rules(&s.config, s.store.transmitters[&key].owner);
        let before = s.store.transmitters[&key].targets.clone();
        match word.as_str() {
            "" | "menu" => {
                drop(g);
                relock(&self.queue).opens.push((uuid, key, 0));
            }
            "add" | "remove" => {
                let list = match coords::parse(rest, key.1) {
                    Ok(list) => list,
                    Err(e) => {
                        drop(g);
                        return self.note(uuid, e, RED);
                    }
                };
                let mut lines = Vec::new();
                if word == "add" {
                    match s.store.add(key, &list, &rules) {
                        Ok((added, refused)) => {
                            let n = s.store.transmitters[&key].targets.len();
                            lines.push((format!("Додано {}; цілей {n} з {}", added.len(), rules.max_targets), GREEN));
                            lines.extend(refused.into_iter().map(|r| (r, RED)));
                        }
                        Err(e) => lines.push((e, RED)),
                    }
                } else {
                    match s.store.remove(key, &list) {
                        Ok(n) => lines.push((format!("Прибрано {n}"), GREEN)),
                        Err(e) => lines.push((e, RED)),
                    }
                }
                drop(g);
                for (l, c) in lines {
                    self.note(uuid, l, c);
                }
                self.edited(key, &before);
                // Back to the menu the dialog was opened from.
                relock(&self.queue).opens.push((uuid, key, usize::MAX));
            }
            "list" => {
                let t = &s.store.transmitters[&key];
                let mut lines = vec![(format!("Передавач {}, сигнал {}, цілей {}:", coords::show(key.1), t.signal, t.targets.len()), YELLOW)];
                lines.extend(t.targets.iter().map(|p| (format!("  {}", coords::show(*p)), GRAY)));
                drop(g);
                for (l, c) in lines {
                    self.note(uuid, l, c);
                }
            }
            "clear" => {
                let all = before.clone();
                let _ = s.store.remove(key, &all);
                drop(g);
                self.note(uuid, "Усі цілі прибрано", GREEN);
                self.edited(key, &before);
            }
            "link" => {
                let end = self.ticks.load(Relaxed) + u64::from(s.config.link_seconds) * 20;
                s.linking.insert(uuid, (key, end));
                drop(g);
                self.bar(uuid, "ПКМ по блоках: додати або прибрати. ПКМ по передавачу: готово", YELLOW);
            }
            "done" => {
                let was = s.linking.remove(&uuid).is_some();
                drop(g);
                self.bar(uuid, if was { "Вибір блоків завершено" } else { "Режим вибору не був увімкнений" }, GRAY);
            }
            _ => {
                drop(g);
                for l in HELP {
                    self.note(uuid, l, YELLOW);
                }
            }
        }
    }
}

impl Hooks for Remote {
    fn level_tick_pre(&self, host: &mut dyn Host, event: &mut LevelTickPreEvent) {
        let _ = event;
        // Before the level's redstone runs, so the saved power is there from the first tick.
        self.open(&*host);
    }

    /// The level is in place: read the transmitters told of, refresh the blocks whose power
    /// changed.
    fn level_tick_post(&self, host: &mut dyn Host, event: &mut LevelTickPostEvent) {
        let Some(dim) = dim_code(event.dimension) else {
            return;
        };
        if !self.open(&*host) {
            return;
        }
        // Once after the start: what changed while the server was off (blocks in chunks not
        // loaded read as air and are left for their next change).
        let first = {
            let mut g = self.state();
            match g.as_mut() {
                Some(s) if !s.synced[usize::from(dim)] => {
                    s.synced[usize::from(dim)] = true;
                    Some(s.store.in_dimension(dim))
                }
                _ => None,
            }
        };
        if let Some(keys) = first {
            for key in keys {
                self.check_later(key, false);
            }
            let powered = self.power.read().unwrap_or_else(|p| p.into_inner()).powered_in(dim);
            self.refresh_later(dim, powered);
        }
        if self.any_pending.load(Relaxed) {
            let due: Vec<(TxKey, bool)> = {
                let mut p = relock(&self.pending);
                let due: Vec<(TxKey, bool)> = p.iter().filter(|(k, _)| k.0 == dim).map(|(k, v)| (*k, *v)).collect();
                for (k, _) in &due {
                    p.remove(k);
                }
                self.any_pending.store(!p.is_empty(), Relaxed);
                due
            };
            for (key, verify) in due {
                self.check(host, key, verify);
            }
        }
        if self.any_refresh.load(Relaxed) {
            let due: Vec<Pos> = {
                let mut r = relock(&self.refresh);
                let mut due = Vec::new();
                r.retain(|(d, p)| {
                    if *d == dim {
                        due.push(*p);
                        false
                    } else {
                        true
                    }
                });
                self.any_refresh.store(!r.is_empty(), Relaxed);
                due
            };
            let mut seen = HashSet::new();
            for pos in due {
                if seen.insert(pos) {
                    host.update_signal_neighbors(dim_name(dim), pos);
                }
            }
        }
    }

    fn server_tick(&self, host: &mut dyn Host, event: &mut ServerTickEvent) {
        if !matches!(event.phase, ServerTickPhase::ServerTickEnd { .. }) {
            return;
        }
        let tick = self.ticks.fetch_add(1, Relaxed) + 1;
        if !self.open(&*host) {
            return;
        }
        let commands = std::mem::take(&mut relock(&self.queue).commands);
        for (uuid, line) in commands {
            self.command(host, uuid, &line);
        }
        let expired = {
            let mut g = self.state();
            let Some(s) = g.as_mut() else {
                return;
            };
            let mut expired = Vec::new();
            s.linking.retain(|uuid, (_, end)| {
                let keep = *end > tick;
                if !keep {
                    expired.push(*uuid);
                }
                keep
            });
            // Signals change often (a clock): they are written every 10 seconds at most.
            if s.dirty && tick % 200 == 0 {
                s.dirty = false;
                if let Err(e) = s.store.save() {
                    log::warn!(target: LOG, "[RemoteRedstone] can't save: {e}");
                }
            }
            expired
        };
        for uuid in expired {
            self.bar(uuid, "Вибір блоків завершено (час вийшов)", GRAY);
        }
        let q = std::mem::take(&mut *relock(&self.queue));
        for (uuid, key, page) in q.opens {
            let page = if page == usize::MAX {
                self.state().as_ref().and_then(|s| s.menus.get(&uuid)).map_or(0, |m| m.page)
            } else {
                page
            };
            self.show(host, uuid, key, page);
        }
        for (uuid, text, color, bar) in q.notes {
            let line = Text::colored(text, color);
            if bar {
                host.action_bar(uuid, line);
            } else {
                host.tell(uuid, line);
            }
        }
    }

    fn server_stopping(&self, host: &mut dyn Host, event: &mut ServerStoppingEvent) {
        let _ = (host, event);
        if let Some(s) = self.state().as_mut()
            && let Err(e) = s.store.save()
        {
            log::warn!(target: LOG, "[RemoteRedstone] can't save: {e}");
        }
    }

    fn data_pack_files(&self, host: &dyn Host, event: &mut DataPackFilesEvent) {
        if !self.open(host) {
            return;
        }
        let (block, with_recipe) = match self.state().as_ref() {
            Some(s) => (s.config.block.clone(), s.config.recipe),
            None => return,
        };
        if with_recipe {
            event.files.push(("recipe/remoteredstone:transmitter".to_owned(), recipe(&block).to_string().into_bytes()));
        }
        event.files.push((format!("dialog/{DIALOG}"), dialog().to_string().into_bytes()));
    }

    fn register_commands(&self, host: &mut dyn Host, event: &mut RegisterCommandsEvent) {
        let _ = host;
        // So the chat box completes them; `player_command` answers them before the game parses.
        for usage in ["rr [args:text]", "remoteredstone [args:text]"] {
            event.commands.push(NativeCommand::new(usage));
        }
    }

    fn player_command(&self, host: &mut dyn Host, event: &mut PlayerCommandEvent) -> Flow {
        let _ = host;
        let line = event.command.trim();
        let (word, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        if !matches!(word.to_ascii_lowercase().as_str(), "rr" | "remoteredstone") {
            return Flow::Continue;
        }
        relock(&self.queue).commands.push((event.player, rest.to_owned()));
        Flow::Cancel
    }

    /// A transmitter's item placed: a transmitter there, with the item's targets.
    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent) {
        let Some(targets) = marked(event.stack) else {
            return;
        };
        let Some(me) = host.acting_player() else {
            return;
        };
        if !self.allowed(me.uuid, me.permission) {
            // Only from the off hand: the main hand's click is refused before (`right_click_block`).
            return self.bar(me.uuid, NEED, RED);
        }
        let Some(dim) = dim_code(&me.dimension) else {
            return self.note(me.uuid, "Передавач працює лише в Overworld, Nether і End", RED);
        };
        let key = (dim, event.pos);
        let n = {
            let mut g = self.state();
            let Some(s) = g.as_mut() else {
                return;
            };
            let rules = self.rules(&s.config, me.uuid);
            let n = s.store.place(key, me.uuid, &targets, &rules).targets.len();
            s.editing.insert(me.uuid, key);
            if let Err(e) = s.store.save() {
                log::warn!(target: LOG, "[RemoteRedstone] can't save: {e}");
            }
            n
        };
        self.set_txs(key, true);
        self.check_later(key, false);
        let tail = if n > 0 { format!(", цілей з предмета: {n}") } else { String::new() };
        self.bar(me.uuid, format!("Передавач поставлено{tail}. ПКМ по ньому: меню"), GREEN);
    }

    /// Someone else's transmitter stays when `owner_only` says so.
    fn destroy_block(&self, host: &mut dyn Host, event: &mut DestroyBlockEvent) -> Flow {
        if !self.any_tx.load(Relaxed) {
            return Flow::Continue;
        }
        let Some(me) = host.acting_player() else {
            return Flow::Continue;
        };
        let Some(dim) = dim_code(&me.dimension) else {
            return Flow::Continue;
        };
        let key = (dim, event.pos);
        if !self.is_tx(key) {
            return Flow::Continue;
        }
        let allowed = self.state().as_ref().is_none_or(|s| s.store.may_edit(key, me.uuid, me.permission, &s.config));
        if allowed {
            // Its drop (`loot_drops`) keeps the targets; the block change removes it.
            return Flow::Continue;
        }
        self.bar(me.uuid, "Це чужий передавач", RED);
        Flow::Cancel
    }

    /// A broken transmitter drops its own item, which keeps the targets.
    fn loot_drops(&self, host: &mut dyn Host, event: &mut LootDropsEvent) {
        let _ = host;
        let Some((pos, _)) = event.context.block else {
            return;
        };
        let Some(dim) = dim_code(event.context.dimension) else {
            return;
        };
        if !self.is_tx((dim, pos)) {
            return;
        }
        let (block, targets) = match self.state().as_ref() {
            Some(s) => match s.store.transmitters.get(&(dim, pos)) {
                Some(t) => (s.config.block.clone(), t.targets.clone()),
                None => return,
            },
            None => return,
        };
        if let Some(drop) = event.drops.iter_mut().find(|s| &*s.item == block.as_str() && s.count > 0) {
            *drop = transmitter_item(&block, &targets);
        }
    }

    /// The menu (right click), or a target picked in the click mode.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        let Some(me) = host.acting_player() else {
            return Flow::Continue;
        };
        let Some(dim) = dim_code(&me.dimension) else {
            return Flow::Continue;
        };
        let pos = event.pos;
        let key = (dim, pos);
        let tick = self.ticks.load(Relaxed);
        let allowed = self.allowed(me.uuid, me.permission);
        if !allowed && host.held_stack(me.uuid).is_some_and(|s| marked(&s).is_some()) && (me.sneaking || !self.is_tx(key)) {
            // A transmitter in hand would be placed.
            self.bar(me.uuid, NEED, RED);
            return Flow::Cancel;
        }
        let mut g = self.state();
        let Some(s) = g.as_mut() else {
            return Flow::Continue;
        };
        // Each hand asks: once per click.
        let again = s.last_click.insert(me.uuid, (tick, pos)) == Some((tick, pos));
        if let Some((tx, _)) = s.linking.get(&me.uuid).copied() {
            if tx.0 != dim || !s.store.transmitters.contains_key(&tx) {
                s.linking.remove(&me.uuid);
            } else {
                if again {
                    return Flow::Cancel;
                }
                if pos == tx.1 {
                    s.linking.remove(&me.uuid);
                    drop(g);
                    self.bar(me.uuid, "Вибір блоків завершено", GREEN);
                    relock(&self.queue).opens.push((me.uuid, tx, 0));
                    return Flow::Cancel;
                }
                if !allowed {
                    s.linking.remove(&me.uuid);
                    drop(g);
                    self.bar(me.uuid, NEED, RED);
                    return Flow::Cancel;
                }
                let before = s.store.transmitters[&tx].targets.clone();
                let end = tick + u64::from(s.config.link_seconds) * 20;
                s.linking.insert(me.uuid, (tx, end));
                let rules = self.rules(&s.config, s.store.transmitters[&tx].owner);
                let result = s.store.toggle(tx, pos, &rules);
                let count = s.store.transmitters[&tx].targets.len();
                let max = rules.max_targets;
                drop(g);
                match result {
                    Ok(Edit::Added(p)) => self.bar(me.uuid, format!("+ {}  ({count} з {max})", coords::show(p)), GREEN),
                    Ok(Edit::Removed(p)) => self.bar(me.uuid, format!("− {}  ({count} з {max})", coords::show(p)), YELLOW),
                    Err(e) => self.bar(me.uuid, e, RED),
                }
                self.edited(tx, &before);
                return Flow::Cancel;
            }
        }
        if me.sneaking || !self.is_tx(key) {
            return Flow::Continue;
        }
        if again {
            return Flow::Cancel;
        }
        if !s.store.may_edit(key, me.uuid, me.permission, &s.config) {
            drop(g);
            self.bar(me.uuid, "Це чужий передавач", RED);
            return Flow::Cancel;
        }
        if !allowed {
            drop(g);
            self.bar(me.uuid, NEED, RED);
            return Flow::Cancel;
        }
        s.editing.insert(me.uuid, key);
        drop(g);
        relock(&self.queue).opens.push((me.uuid, key, 0));
        Flow::Cancel
    }

    /// A transmitter in the crafting grid's result only for a player with the perk.
    fn menu_clicked(&self, host: &mut dyn Host, event: &mut MenuClickedEvent) {
        let found = {
            let g = self.state();
            let Some(s) = g.as_ref() else {
                return;
            };
            let Some(open) = s.menus.get(&event.uuid).copied().filter(|m| m.window == event.window) else {
                return;
            };
            s.store
                .transmitters
                .get(&open.tx)
                .map(|t| (open, menu::click(t, open.page, event.slot, event.button, event.mode), t.targets.clone()))
        };
        let Some((open, action, before)) = found else {
            host.close_menu(event.uuid);
            return;
        };
        let uuid = event.uuid;
        match action {
            Action::Remove(p) => {
                if let Some(s) = self.state().as_mut() {
                    let _ = s.store.remove(open.tx, &[p]);
                }
                self.edited(open.tx, &before);
                self.show(host, uuid, open.tx, open.page);
            }
            Action::Clear => {
                if let Some(s) = self.state().as_mut() {
                    let _ = s.store.remove(open.tx, &before);
                }
                self.edited(open.tx, &before);
                self.show(host, uuid, open.tx, 0);
            }
            Action::Page(page) => self.show(host, uuid, open.tx, page),
            Action::Coords => {
                let name = host.server_players().into_iter().find(|p| p.uuid == uuid).map(|p| p.name);
                host.close_menu(uuid);
                if let Some(name) = name
                    && !host.run_command(&format!("dialog show {name} {DIALOG}"))
                {
                    log::warn!(target: LOG, "[RemoteRedstone] couldn't show the dialog to {name}");
                }
            }
            Action::Link => {
                if let Some(s) = self.state().as_mut() {
                    let end = self.ticks.load(Relaxed) + u64::from(s.config.link_seconds) * 20;
                    s.linking.insert(uuid, (open.tx, end));
                }
                host.close_menu(uuid);
                host.action_bar(
                    uuid,
                    Text::colored("ПКМ по блоках: додати або прибрати. ПКМ по передавачу: готово".to_owned(), YELLOW),
                );
            }
            // The same page again: the client's copy of the clicked slot must be put back.
            Action::Nothing => self.show(host, uuid, open.tx, open.page),
        }
    }

    fn menu_closed(&self, host: &mut dyn Host, event: &mut MenuClosedEvent) {
        let _ = host;
        if let Some(s) = self.state().as_mut()
            && s.menus.get(&event.uuid).is_some_and(|m| m.window == event.window)
        {
            s.menus.remove(&event.uuid);
        }
    }

    // The hooks below are asked from inside the level's update, very often: atomics first, no host.

    /// The redstone asks the signal the block at `pos` gives the block on its `face`: a target
    /// gets its transmitter's signal from every side, weak and direct, as from a lever on it (a
    /// lamp lights, a stone block powers the dust beside it).
    fn block_signal(&self, level: &dyn LevelHost, event: &mut BlockSignalEvent) {
        let _ = level;
        if !self.any_power.load(Relaxed) {
            return;
        }
        let reader = step(at(&event.pos), normal(event.face));
        let except = READING.with(|r| r.get());
        let signal = self.power.read().unwrap_or_else(|p| p.into_inner()).signal_at(level_code(event.dimension), reader, except);
        if i32::from(signal) > event.signal {
            event.signal = i32::from(signal);
        }
    }

    /// Something beside a transmitter changed: read it at the tick's end.
    fn neighbor_changed(&self, level: &dyn LevelHost, event: &mut NeighborChangedEvent) {
        let _ = level;
        let key = (level_code(event.dimension), at(&event.pos));
        if self.is_tx(key) {
            self.check_later(key, true);
        }
    }

    /// The transmitter's own block changed (lit, broken, replaced): check it at the tick's end.
    fn block_changed(&self, level: &dyn LevelHost, event: &mut BlockChangedEvent) {
        let _ = level;
        let key = (level_code(event.dimension), at(&event.pos));
        if self.is_tx(key) {
            self.check_later(key, true);
        }
    }

    /// Pistons don't move transmitters (their coordinates would be lost).
    fn piston_move(&self, level: &dyn LevelHost, event: &mut PistonMoveEvent) -> Flow {
        let _ = level;
        if !self.any_tx.load(Relaxed) {
            return Flow::Continue;
        }
        let dim = level_code(event.dimension);
        if event.push.iter().chain(event.destroy.iter()).any(|p| self.is_tx((dim, at(p)))) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }

    /// Blasts spare transmitters.
    fn explosion_detonate(&self, host: &mut dyn Host, event: &mut ExplosionDetonateEvent) {
        let _ = host;
        if !self.any_tx.load(Relaxed) {
            return;
        }
        let Some(dim) = dim_code(event.dimension) else {
            return;
        };
        let txs = self.txs.read().unwrap_or_else(|p| p.into_inner());
        event.blocks.retain(|p| !txs.contains(&(dim, *p)));
    }
}
