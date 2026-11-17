//! Progression for Horizon: the glue between Horizon's hooks and the rules in `core/`. Each
//! hook asks the engine, and the [`Effect`]s it returns are carried out here. The rules are pure
//! Rust, tested on their own (`core-tests/`).
//!
//! Only on a dedicated server (`NativeAuthority::DedicatedServer`).
//!
//! Locks: nothing of the host is called while the engine's, the queue's or the clients' lock is
//! held (a host call may run another of this mod's hooks before it returns).

mod core;
mod remote;
mod travel;

use crate::core::engine::{self, Effect, Engine, Item, Page, Seen, Tone};
use crate::core::store::Zone;
use crate::core::gui::{self, Cell, MenuView};
use horizon_client::mod_api::*;
use horizon_client::text::Style;
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(Progression {
        engine: Mutex::new(None),
        queue: Mutex::new(Queue::default()),
        menus: Mutex::new(HashMap::new()),
        clients: Mutex::new(Clients::default()),
        hunters: Mutex::new(HashMap::new()),
        bodies: Mutex::new(HashMap::new()),
        heads: Mutex::new(HashSet::new()),
        divers: Mutex::new(HashSet::new()),
        zones: Mutex::new(Vec::new()),
        fills: Mutex::new(Vec::new()),
        clicked: Mutex::new(HashMap::new()),
        pack: Mutex::new(None),
        active: AtomicBool::new(false),
        pvp: AtomicU32::new(1.5f32.to_bits()),
        ticks: AtomicU32::new(0),
        random: AtomicU64::new(0x9E37_79B9_7F4A_7C15),
        remote: remote::Remote::default(),
        rides: Mutex::new(travel::Rides::default()),
    })
}

/// The resource pack with the menus' pictures (`tools/make_pack.py` makes it).
static PACK: &[u8] = include_bytes!("../data/progression.zip");
/// The pack's own id for clients: a new pack with it takes the old one's place.
const PACK_ID: u128 = 0x7072_6f67_7265_7373_696f_6e5f_6775_6931;

/// What waits for the next tick: most hooks run with a player's state swapped in, where server
/// commands can't run and other players can't be told.
#[derive(Default)]
struct Queue {
    effects: Vec<Effect>,
    joined: Vec<(u128, String)>,
    reapply: Vec<u128>,
    /// Keys, dialog buttons, the altar block: (player, action).
    actions: Vec<(u128, String)>,
    /// Swap-hands presses that dash (F, Shift+F).
    swaps: Vec<u128>,
    /// Players the void walker saves: back to their last ground.
    falls: Vec<u128>,
    /// Players the last stand kept alive.
    stands: Vec<u128>,
    /// Trees to fell: player, dimension, the cut log, how many logs at most.
    fells: Vec<(u128, String, [i32; 3], usize)>,
    /// "Каменяр": player, dimension, the block to turn into the next of its family.
    masons: Vec<(u128, String, [i32; 3])>,
    /// "Кельма": player, dimension, the block they placed, the item it came from.
    trowels: Vec<(u128, String, [i32; 3], String)>,
    /// Players getting on or off a horse or a minecart.
    rides: Vec<travel::Ride>,
    /// Rocket boosts in a glide ("Ракетник").
    boosts: Vec<u128>,
    /// Horses just bred: foal, parent, partner.
    foals: Vec<[u128; 3]>,
    /// Players who opened a mount's inventory: player, dimension.
    mounts: Vec<(u128, String)>,
}

/// The place next to `pos` on its `face` side: where a block put on that face would go.
fn beside(pos: [i32; 3], face: [i32; 3]) -> [i32; 3] {
    if face.iter().map(|v| v.abs()).sum::<i32>() == 1 {
        [pos[0] + face[0], pos[1] + face[1], pos[2] + face[2]]
    } else {
        pos
    }
}

/// A "Кресляр" fill at work: the places left, a few each tick.
struct FillJob {
    uuid: u128,
    dimension: String,
    item: String,
    blocks: Vec<[i32; 3]>,
    placed: usize,
}

/// Blocks the fills place in one tick, all players together: a few hundred a second, so neither
/// the light engine nor the network gets a burst.
const FILL_PER_TICK: usize = 32;
/// Blocks that fall on a head ("Міцна голова" takes their damage away).
/// What "Міцна голова" keeps off: falling blocks, stalagmites, a wall around the head, a crowd.
const FALLING: [&str; 6] = [
    "minecraft:falling_anvil",
    "minecraft:falling_block",
    "minecraft:falling_stalactite",
    "minecraft:stalagmite",
    "minecraft:in_wall",
    "minecraft:cramming",
];

/// The shared client mod's channel ("Клавіші сервера", `serverkeys`): each message is
/// `<server mod id>:<action>`, the name of the key that was pressed.
const KEYS_CHANNEL: &str = "serverkeys:action";
/// To the client mod: a push for the local player (three f32, little-endian).
const PUSH_CHANNEL: &str = "serverkeys:push";
/// To the client mod: the client-side perks the player owns, comma separated.
const PERKS_CHANNEL: &str = "serverkeys:perks";
/// To the client mod (0.3 and later): the perk map it draws as a screen (`Engine::tree`).
const TREE_CHANNEL: &str = "serverkeys:tree";
/// The stats of the horse whose inventory a player opened, for the client mod to draw.
pub(crate) const MOUNT_CHANNEL: &str = "serverkeys:mount";

/// The anvil's prior-work penalty never grows past this on what it makes here, so a book from
/// the altar always fits under the level limit.
const MAX_PENALTY: u32 = 15;

/// What the mod knows of players' clients and bodies between ticks.
#[derive(Default)]
struct Clients {
    /// Players on a Horizon client with the server keys mod (they said hello).
    horizon: HashSet<u128>,
    /// Players whose client loaded the mod's resource pack.
    pack: HashSet<u128>,
    /// Players holding Shift (for Shift+F).
    sneaking: HashSet<u128>,
    /// Health at the last tick, to notice damage (no spirit right after it).
    health: HashMap<u128, f32>,
    /// The client-side perks last sent to each Horizon player.
    perks_sent: HashMap<u128, Vec<String>>,
    /// Players whose client mod draws the perk map (it said `hello:3` or later).
    tree: HashSet<u128>,
    /// Where each player last stood on solid ground (for the void walker).
    ground: HashMap<u128, (String, [f64; 3])>,
    /// The effects each alchemist had at the last look: id -> (amplifier, ticks left).
    effects: HashMap<u128, HashMap<String, (u8, i32)>>,
}

/// A menu the mod has open for a player.
#[derive(Clone)]
struct OpenMenu {
    window: i32,
    page: Page,
    /// The title it was opened with: a new one needs the menu opened again.
    title: String,
}

/// Where clients get the pack.
#[derive(Clone)]
struct PackOffer {
    url: String,
    sha1: String,
    required: bool,
}

struct Progression {
    /// Remote redstone (the transmitters); the "relay" perk gives the right to them.
    remote: remote::Remote,
    /// The travel branches' state (`travel.rs`).
    rides: Mutex<travel::Rides>,
    /// Opened at the first tick of a dedicated server.
    engine: Mutex<Option<Engine>>,
    queue: Mutex<Queue>,
    menus: Mutex<HashMap<u128, OpenMenu>>,
    clients: Mutex<Clients>,
    /// Entity id of each online player with the "hunter" perk -> their damage multiplier on mobs.
    hunters: Mutex<HashMap<i32, f32>>,
    /// Entity id -> player, for the hooks that only know entities.
    bodies: Mutex<HashMap<i32, u128>>,
    /// Entity ids of the players with "Міцна голова".
    heads: Mutex<HashSet<i32>>,
    /// Entity ids of the players with "Дитя води" II: they don't drown.
    divers: Mutex<HashSet<i32>>,
    /// "Кресляр" fills at work.
    fills: Mutex<Vec<FillJob>>,
    /// The last block click each player made with a tool of a perk (tick, place): a click
    /// comes once for each hand.
    clicked: Mutex<HashMap<u128, (u32, [i32; 3])>>,
    /// The redstone labs at work (read by the blast and block tick hooks).
    zones: Mutex<Vec<Zone>>,
    pack: Mutex<Option<PackOffer>>,
    active: AtomicBool,
    /// The PvP multiplier as f32 bits (the damage hook has no host).
    pvp: AtomicU32,
    ticks: AtomicU32,
    random: AtomicU64,
}

fn relock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic while a lock was held must not take every later hook down with it.
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn color(tone: Tone) -> u32 {
    match tone {
        Tone::Info => 0xFFFF55,
        Tone::Good => 0x55FF55,
        Tone::Bad => 0xFF5555,
    }
}

fn line(text: &str, tone: Tone) -> Text {
    Text::colored(text.to_owned(), color(tone))
}

fn stack(cell: &Cell) -> ItemStack {
    let mut s = ItemStack::new(&cell.icon, cell.count.clamp(1, 99));
    let data = s.data_mut();
    data.set_loot_component(
        "minecraft:custom_name",
        &serde_json::json!({ "text": cell.name, "italic": false, "color": cell.color }),
    );
    let lore: Vec<serde_json::Value> = cell
        .lore
        .iter()
        .filter(|(t, _)| !t.is_empty())
        .map(|(t, c)| serde_json::json!({ "text": t, "italic": false, "color": c }))
        .collect();
    if !lore.is_empty() {
        data.set_loot_component("minecraft:lore", &serde_json::Value::Array(lore));
    }
    if cell.glint {
        data.set_loot_component("minecraft:enchantment_glint_override", &serde_json::Value::Bool(true));
    }
    // No attack damage or other default lines under the name.
    data.set_loot_component(
        "minecraft:tooltip_display",
        &serde_json::json!({ "hidden_components": ["minecraft:attribute_modifiers", "minecraft:enchantments"] }),
    );
    s
}

fn items(view: &MenuView) -> Vec<Option<ItemStack>> {
    view.cells.iter().map(|c| c.as_ref().map(stack)).collect()
}

/// A menu's title: the pack's glyphs (white, or the title's grey would tint the pictures), then
/// the readable part in the usual font.
fn title(view: &MenuView) -> Text {
    if view.glyphs.is_empty() {
        return Text::literal(view.label.clone());
    }
    let picture = Style { color: Some(0xFFFFFF), font: Some(gui::FONT.to_owned()), ..Style::default() };
    let words = Style { color: Some(0xFFE9A8), font: Some("minecraft:default".to_owned()), ..Style::default() };
    Text::literal(String::new())
        .append(Text::styled(view.glyphs.clone(), picture))
        .append(Text::styled(view.label.clone(), words))
}

/// The toolkit's (kirca) 3x3 tool, as that mod makes and knows it.
fn multibreak(item: &str, name: &str) -> ItemStack {
    let mut s = ItemStack::new(item, 1);
    let data = s.data_mut();
    data.set_loot_component("minecraft:custom_data", &serde_json::json!({ "kirca": "multibreak" }));
    data.set_loot_component("minecraft:custom_name", &serde_json::json!({ "text": name, "italic": false, "color": "gold" }));
    data.set_loot_component(
        "minecraft:lore",
        &serde_json::json!([
            { "text": "Бульдозер", "italic": false, "color": "gold" },
            { "text": "Ламає блоки 3x3", "italic": false, "color": "gray" }
        ]),
    );
    s
}

fn item_stack(item: &Item) -> ItemStack {
    match item {
        Item::Pickaxe3 => multibreak("minecraft:diamond_pickaxe", "Multibreak Pickaxe"),
        Item::Shovel3 => multibreak("minecraft:diamond_shovel", "Multibreak Shovel"),
        Item::Book { enchant, level } => {
            let mut s = ItemStack::new("minecraft:enchanted_book", 1);
            s.enchant(enchant, *level);
            s
        }
        Item::Transmitter => ItemStack::new("minecraft:redstone_lamp", 1),
    }
}

fn levels_of(stack: &ItemStack) -> Vec<(String, u32)> {
    stack.crafting_enchantments().iter().map(|(e, l)| (e.to_string(), *l)).collect()
}

fn set_levels(stack: &mut ItemStack, levels: Vec<(String, u32)>) {
    stack.set_crafting_enchantments(levels.into_iter().map(|(e, l)| (Arc::<str>::from(e.as_str()), l)).collect());
}

/// Serves the pack to whoever asks, whatever they ask for.
fn serve_pack(port: u16) -> std::io::Result<()> {
    let listener = std::net::TcpListener::bind(("0.0.0.0", port))?;
    std::thread::Builder::new().name("progression pack".into()).spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut conn) = conn else { continue };
            let _ = conn.set_read_timeout(Some(std::time::Duration::from_secs(5)));
            let _ = conn.set_write_timeout(Some(std::time::Duration::from_secs(30)));
            // The request's first part is enough: every path gets the pack.
            let mut request = [0u8; 2048];
            let _ = conn.read(&mut request);
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/zip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                PACK.len()
            );
            let _ = conn.write_all(head.as_bytes()).and_then(|()| conn.write_all(PACK)).and_then(|()| conn.flush());
        }
    })?;
    Ok(())
}

impl Progression {
    fn active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }

    fn engine(&self) -> MutexGuard<'_, Option<Engine>> {
        relock(&self.engine)
    }

    fn now(&self) -> u64 {
        u64::from(self.ticks.load(Ordering::Relaxed))
    }

    fn queue(&self, effects: Vec<Effect>) {
        if !effects.is_empty() {
            relock(&self.queue).effects.extend(effects);
        }
    }

    /// 0 to 1.
    fn chance(&self) -> f64 {
        let mut x = self.random.load(Ordering::Relaxed);
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.random.store(x, Ordering::Relaxed);
        (x >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Carries out `effects` now: only where commands may run (the tick, a menu click).
    fn apply(&self, host: &mut dyn Host, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Tell(uuid, text, tone) => {
                    host.tell(uuid, line(&text, tone));
                }
                Effect::ActionBar(uuid, text) => {
                    host.action_bar(uuid, line(&text, Tone::Info));
                }
                Effect::Broadcast(text) => host.broadcast(line(&text, Tone::Good)),
                Effect::Console(text) => log::info!(target: "horizon::progression", "{text}"),
                Effect::Command(c) => {
                    if !host.run_command(&c) {
                        log::debug!(target: "horizon::progression", "[Progression] the server refused: {c}");
                    }
                }
                Effect::Attribute(uuid, attribute, id, add) => {
                    horizon_client::progression_attrs::push(uuid, &attribute, &id, add);
                }
                Effect::Dash(uuid, name, kind) => self.dash(host, uuid, &name, kind),
                Effect::ClientPerks(uuid, perks) => {
                    let horizon = relock(&self.clients).horizon.contains(&uuid);
                    if horizon && host.net_send_to_player(uuid, PERKS_CHANNEL, perks.join(",").as_bytes()).is_ok() {
                        relock(&self.clients).perks_sent.insert(uuid, perks);
                    }
                }
                Effect::Sell { uuid, levels, item } => {
                    let stack = match item {
                        Item::Transmitter => self.remote.transmitter(),
                        other => item_stack(&other),
                    };
                    if host.give_stack(uuid, stack).is_some() {
                        host.tell(uuid, line("Немає місця в інвентарі: нічого не списано", Tone::Bad));
                    } else if levels > 0
                        && let Ok(n) = i32::try_from(levels)
                    {
                        host.give_levels(uuid, -n);
                    }
                }
                Effect::TakeLevels(uuid, levels) => {
                    if let Ok(n) = i32::try_from(levels) {
                        host.give_levels(uuid, -n);
                    }
                }
                Effect::Fill { uuid, dimension, blocks } => {
                    let held = host.player_equipment(uuid, EquipmentSlot::OffHand).map(|s| s.item.to_string());
                    let busy = relock(&self.fills).iter().any(|j| j.uuid == uuid);
                    match held.filter(|item| host.default_state(item).is_some()) {
                        _ if busy => {
                            host.action_bar(uuid, line("Попередня заливка ще йде", Tone::Bad));
                        }
                        Some(item) => relock(&self.fills).push(FillJob { uuid, dimension, item, blocks, placed: 0 }),
                        None => {
                            host.action_bar(uuid, line("Візьми в ліву руку блоки, якими заливати", Tone::Bad));
                        }
                    }
                }
                Effect::Anchor { owner, dimension, chunks, hold } => {
                    let mut refused = 0;
                    for chunk in chunks {
                        if !host.force_chunk_in(&owner, &dimension, chunk, hold) {
                            refused += 1;
                        }
                    }
                    if refused > 0 && hold {
                        log::warn!(target: "horizon::progression", "[Progression] {owner}: {refused} chunks of {dimension} were not held");
                    }
                }
            }
        }
    }

    /// A push from the player's own Horizon client (straight and as far as the settings say),
    /// else the wind-charge trick, which any client feels.
    fn dash(&self, host: &mut dyn Host, uuid: u128, name: &str, kind: &str) {
        let horizon = relock(&self.clients).horizon.contains(&uuid);
        let me = host.server_players().into_iter().find(|p| p.uuid == uuid);
        let settings = self.engine().as_ref().map(|e| e.config.settings.clone());
        if horizon
            && let (Some(me), Some(st)) = (me, settings)
        {
            let push = if kind == "dash_up" {
                [0.0, st.dash_up_speed, 0.0]
            } else {
                let yaw = me.yaw.to_radians();
                [-yaw.sin() * st.dash_forward_speed, st.dash_forward_lift, yaw.cos() * st.dash_forward_speed]
            };
            let mut data = Vec::with_capacity(12);
            for v in push {
                data.extend_from_slice(&v.to_le_bytes());
            }
            if host.net_send_to_player(uuid, PUSH_CHANNEL, &data).is_ok() {
                return;
            }
        }
        if let Some(c) = engine::dash_command(name, uuid, kind)
            && !host.run_command(&c)
        {
            log::debug!(target: "horizon::progression", "[Progression] the server refused: {c}");
        }
    }

    fn online(host: &dyn Host) -> Vec<(u128, String)> {
        host.server_players().into_iter().map(|p| (p.uuid, p.name)).collect()
    }

    fn seen_of(p: &NativeServerPlayer) -> Seen {
        Seen {
            uuid: p.uuid,
            name: p.name.clone(),
            dimension: p.dimension.clone(),
            position: p.position,
            yaw: p.yaw,
            pitch: p.pitch,
            game_mode: p.game_mode,
            xp_level: p.xp_level,
        }
    }

    /// Opens `page` for the player, in place of whatever menu they had open.
    fn show(&self, host: &mut dyn Host, uuid: u128, page: Page) {
        let pack = relock(&self.clients).pack.contains(&uuid);
        let view = match self.engine().as_ref() {
            Some(e) => e.menu(uuid, page, pack),
            None => return,
        };
        let name = format!("{}{}", view.glyphs, view.label);
        let open = relock(&self.menus).get(&uuid).cloned();
        if let Some(open) = open
            && open.page.rows() == view.rows
            && open.title == name
        {
            host.set_menu_items(uuid, items(&view));
            relock(&self.menus).insert(uuid, OpenMenu { window: open.window, page, title: name });
            return;
        }
        // No lock held: opening closes the old menu, and its `menu_closed` comes in here.
        if let Some(window) = host.open_menu(uuid, view.rows, title(&view), items(&view)) {
            relock(&self.menus).insert(uuid, OpenMenu { window, page, title: name });
        }
    }

    /// Where clients get the pack, and the mod's own server for it when the settings ask.
    fn offer_pack(&self, e: &Engine) {
        let s = &e.config.settings;
        let offer = if !s.pack_url.is_empty() {
            Some(PackOffer { url: s.pack_url.clone(), sha1: String::new(), required: s.pack_required })
        } else if s.pack_port != 0 {
            match serve_pack(s.pack_port) {
                Ok(()) => {
                    let url = format!("http://{}:{}/progression.zip", s.pack_host, s.pack_port);
                    log::info!(target: "horizon::progression", "[Progression] the resource pack is served at {url}");
                    Some(PackOffer { url, sha1: crate::core::sha1::sha1_hex(PACK), required: s.pack_required })
                }
                Err(err) => {
                    log::warn!(target: "horizon::progression", "[Progression] can't serve the resource pack on port {}: {err}; menus stay plain", s.pack_port);
                    None
                }
            }
        } else {
            None
        };
        *relock(&self.pack) = offer;
    }

    fn open_engine(&self, host: &mut dyn Host) -> bool {
        if self.engine().is_some() {
            return true;
        }
        let dir = match host.config_dir() {
            Ok(dir) => dir.join("progression"),
            Err(e) => {
                log::error!(target: "horizon::progression", "[Progression] no config folder: {e}");
                return false;
            }
        };
        let online = Self::online(host);
        let (mut e, log) = Engine::open(&dir);
        for l in log {
            log::info!(target: "horizon::progression", "{l}");
        }
        self.pvp.store(e.pvp_multiplier().to_bits(), Ordering::Relaxed);
        self.offer_pack(&e);
        // Players already online (a mod loaded late): as if they joined now.
        let mut effects = Vec::new();
        for (uuid, name) in &online {
            effects.extend(e.join(*uuid, name));
        }
        *self.engine() = Some(e);
        self.active.store(true, Ordering::Relaxed);
        self.apply(host, effects);
        true
    }

    /// The command of a key, a dialog button or the chat: the action it names.
    fn command_action(mut words: std::str::SplitWhitespace<'_>) -> &'static str {
        match (words.next().map(str::to_ascii_lowercase).as_deref(), words.next()) {
            (Some("respec"), _) => "respec",
            (Some("spirit"), _) => "spirit",
            (Some("lab"), _) => "lab",
            (Some("anchor"), _) => "anchor",
            (Some("horse"), _) => "horse",
            (Some("bags"), _) => "bags",
            (Some("travel"), _) => "travel",
            (Some("dash"), Some(w)) if w.eq_ignore_ascii_case("up") => "dash_up",
            (Some("dash"), _) => "dash_forward",
            _ => "perks",
        }
    }
}

impl Hooks for Progression {
    fn server_tick(&self, host: &mut dyn Host, event: &mut ServerTickEvent<'_>) {
        self.remote.server_tick(host, event);
        if event.context.authority != NativeAuthority::DedicatedServer {
            return;
        }
        if !matches!(event.phase, ServerTickPhase::ServerTickEnd { .. }) {
            return;
        }
        if !self.open_engine(host) {
            return;
        }
        let q = std::mem::take(&mut *relock(&self.queue));
        let tick = self.ticks.fetch_add(1, Ordering::Relaxed);
        let now = u64::from(tick);
        let players = host.server_players();
        let seen: Vec<Seen> = players.iter().map(Self::seen_of).collect();
        let mut effects = q.effects;
        {
            let mut guard = self.engine();
            let e = guard.as_mut().expect("opened");
            for (uuid, name) in &q.joined {
                effects.extend(e.join(*uuid, name));
            }
            for uuid in &q.reapply {
                effects.extend(e.reapply(*uuid));
            }
            // The transmitters' limits follow the perk (learned, reset, or a player came).
            if tick % 20 == 0 || !q.joined.is_empty() {
                let uuids: Vec<u128> = players.iter().map(|p| p.uuid).collect();
                for (uuid, limits) in e.relays(&uuids) {
                    self.remote.set_limits(uuid, limits);
                }
            }
            if tick % 1200 == 1199 {
                effects.extend(e.save_dirty());
            }
        }
        self.apply(host, effects);
        // The pack, to those who just joined.
        let offer = relock(&self.pack).clone();
        if let Some(offer) = offer {
            let prompt = Text::literal("Меню прогресії з картинками (без пакета вони прості)");
            for (uuid, _) in &q.joined {
                host.send_resource_pack(*uuid, PACK_ID, &offer.url, &offer.sha1, Some(&prompt), offer.required);
            }
        }
        // Damage since the last tick: no spirit for a while.
        let hurt: Vec<u128> = {
            let mut c = relock(&self.clients);
            let mut hurt = Vec::new();
            for p in &players {
                if let Some(old) = c.health.insert(p.uuid, p.health)
                    && p.health < old - 0.01
                {
                    hurt.push(p.uuid);
                }
            }
            hurt
        };
        {
            let mut effects = Vec::new();
            if let Some(e) = self.engine().as_mut() {
                for uuid in hurt {
                    e.hurt(uuid, now);
                }
                if tick % 10 == 0 {
                    effects.extend(e.spirits(&seen));
                    // Client-side perks that changed (bought, a respec): to Horizon clients.
                    {
                        let c = relock(&self.clients);
                        for uuid in &c.horizon {
                            let perks = e.client_perks(*uuid);
                            if c.perks_sent.get(uuid) != Some(&perks) {
                                effects.push(Effect::ClientPerks(*uuid, perks));
                            }
                        }
                    }
                    // Who hits mobs harder, by entity id (the damage hook knows no more).
                    let mut hunters = HashMap::new();
                    for p in &players {
                        let bonus = e.bonus(p.uuid, "hunter");
                        if bonus > 0.0 {
                            hunters.insert(p.entity_id, 1.0 + bonus as f32);
                        }
                    }
                    *relock(&self.hunters) = hunters;
                    *relock(&self.bodies) = players.iter().map(|p| (p.entity_id, p.uuid)).collect();
                    *relock(&self.heads) = players.iter().filter(|p| e.tier(p.uuid, "hard_head") > 0).map(|p| p.entity_id).collect();
                    *relock(&self.divers) = players.iter().filter(|p| e.tier(p.uuid, "swimming") >= 2).map(|p| p.entity_id).collect();
                    *relock(&self.zones) = e.zones();
                }
            }
            self.apply(host, effects);
        }
        // Where each player last stood on solid ground, for the void walker.
        {
            let mut c = relock(&self.clients);
            for p in &players {
                if p.on_ground && matches!(p.game_mode, 0 | 2) {
                    c.ground.insert(p.uuid, (p.dimension.clone(), p.position));
                }
            }
        }
        if tick % 5 == 0 {
            self.tend_effects(host, &players);
        }
        self.rescue_and_fell(host, q.falls, q.stands, q.fells);
        self.mason_and_trowel(host, &players, q.masons, q.trowels);
        self.fill(host, &players);
        if tick % 10 == 0 {
            self.show_drafts(host, &players);
        }
        self.travel_tick(host, &players, tick, q.rides, q.boosts);
        self.foals(host, q.foals);
        if !q.mounts.is_empty() {
            let horizon: Vec<u128> = q.mounts.iter().map(|m| m.0).filter(|u| relock(&self.clients).horizon.contains(u)).collect();
            self.mount_stats(host, &players, q.mounts, &horizon);
        }
        let mut actions = q.actions;
        for uuid in q.swaps {
            let sneaking = relock(&self.clients).sneaking.contains(&uuid);
            let dash = self.engine().as_ref().and_then(|e| e.swap_dash(uuid, sneaking));
            if let Some(dash) = dash {
                actions.push((uuid, dash.to_owned()));
            }
        }
        let mut open = Vec::new();
        for (uuid, action) in actions {
            let Some(me) = seen.iter().find(|p| p.uuid == uuid) else { continue };
            // The horse's own: they need the host, not the engine.
            if matches!(action.as_str(), "horse" | "bags") {
                if let Some(p) = players.iter().find(|p| p.uuid == uuid) {
                    if action == "horse" {
                        self.call_horse(host, p);
                    } else {
                        self.open_bags(host, p);
                    }
                }
                continue;
            }
            let acted = match self.engine().as_mut() {
                Some(e) if action == "hello" => engine::Acted { effects: vec![Effect::ClientPerks(uuid, e.client_perks(uuid))], open: None },
                Some(e) if action.starts_with("buy:") => {
                    let effects = e.buy(uuid, &action[4..], me);
                    // The map shows the new tiers and points.
                    let tree = e.tree(uuid, false);
                    let _ = host.net_send_to_player(uuid, TREE_CHANNEL, tree.as_bytes());
                    engine::Acted { effects, open: None }
                }
                Some(e) => e.action(uuid, &action, now, me),
                None => continue,
            };
            self.apply(host, acted.effects);
            if let Some(page) = acted.open {
                open.push((uuid, page));
            }
        }
        for (uuid, page) in open {
            // A Horizon client with the map gets that in place of the chest menu.
            if matches!(page, Page::Perks | Page::Travel) && relock(&self.clients).tree.contains(&uuid) {
                let tree = self.engine().as_ref().map(|e| e.tree(uuid, true));
                if let Some(tree) = tree
                    && host.net_send_to_player(uuid, TREE_CHANNEL, tree.as_bytes()).is_ok()
                {
                    continue;
                }
            }
            self.show(host, uuid, page);
        }
    }

    fn server_stopping(&self, host: &mut dyn Host, event: &mut ServerStoppingEvent) {
        self.remote.server_stopping(host, event);
        if let Some(e) = self.engine().as_mut() {
            for effect in e.save_dirty() {
                if let Effect::Console(t) = effect {
                    log::warn!(target: "horizon::progression", "{t}");
                }
            }
        }
    }

    /// The Quick Actions key (G by default, rebindable in Controls) opens the mod's dialog on every
    /// client, vanilla or Horizon: a dialog in the `minecraft:quick_actions` tag.
    fn data_pack_files(&self, host: &dyn Host, event: &mut DataPackFilesEvent<'_>) {
        self.remote.data_pack_files(host, event);
        let button = |label: &str, tip: &str, command: &str| {
            serde_json::json!({
                "label": label,
                "tooltip": tip,
                "width": 150,
                "action": { "type": "minecraft:run_command", "command": command }
            })
        };
        let dialog = serde_json::json!({
            "type": "minecraft:multi_action",
            "title": { "text": "Прогресія", "color": "gold" },
            "body": { "type": "minecraft:plain_message", "contents": "Досягнення й справи дають досвід прогресії, рівні дають очки перків (ОП)" },
            "columns": 2,
            "external_title": "Прогресія",
            "can_close_with_escape": true,
            "after_action": "close",
            "actions": [
                button("Перки", "Дерево перків, рівень і досвід", "progress"),
                button("Скинути перки", "Повернути всі ОП за ванільні рівні", "progress respec"),
                button("Ривок вперед", "Якщо вивчено; або F", "progress dash forward"),
                button("Ривок вгору", "Якщо вивчено; або Shift+F", "progress dash up"),
                button("Дух", "Вийти з тіла або повернутися (якщо вивчено)", "progress spirit"),
                button("Лабораторія", "Зона для тестів редстоуну (якщо вивчено)", "progress lab"),
                button("Якір чанків", "Поставити або зняти (якщо вивчено)", "progress anchor"),
                button("Гілки руху", "Вагонетка, кінь, елітри", "progress travel"),
                button("Покликати коня", "Твій кінь прибіжить (якщо вивчено)", "progress horse"),
                button("Сідельні сумки", "Біля свого коня (якщо вивчено)", "progress bags"),
            ]
        });
        let tag = serde_json::json!({ "replace": false, "values": ["progression:menu"] });
        event.files.push(("dialog/progression:menu".to_owned(), dialog.to_string().into_bytes()));
        event.files.push(("tags/minecraft:dialog/quick_actions".to_owned(), tag.to_string().into_bytes()));
    }

    fn register_commands(&self, host: &mut dyn Host, event: &mut RegisterCommandsEvent<'_>) {
        self.remote.register_commands(host, event);
        // So the chat box completes them. `player_command` answers them before the game parses.
        for usage in [
            "progress",
            "progress respec",
            "progress dash forward",
            "progress dash up",
            "progress spirit",
            "progress lab",
            "progress anchor",
            "progress travel",
            "progress horse",
            "progress bags",
            "pr",
            "perks",
        ] {
            event.commands.push(NativeCommand::new(usage));
        }
        event.commands.push(NativeCommand::op("progressadmin [args:text]", 2));
    }

    fn player_command(&self, host: &mut dyn Host, event: &mut PlayerCommandEvent<'_>) -> Flow {
        if matches!(self.remote.player_command(host, event), Flow::Cancel) {
            return Flow::Cancel;
        }
        if !self.active() {
            return Flow::Continue;
        }
        let mut words = event.command.split_whitespace();
        let word = words.next().unwrap_or("").to_ascii_lowercase();
        if !matches!(word.as_str(), "progress" | "pr" | "perks") {
            return Flow::Continue;
        }
        let action = Self::command_action(words);
        relock(&self.queue).actions.push((event.player, action.to_owned()));
        Flow::Cancel
    }

    fn perform_command(&self, host: &mut dyn Host, event: &mut PerformCommandEvent<'_>) -> Flow {
        let mut words = event.command.split_whitespace();
        if !words.next().is_some_and(|w| w.eq_ignore_ascii_case("progressadmin")) {
            return Flow::Continue;
        }
        let uuid = event.player.uuid;
        if event.player.permission < 2 {
            host.tell(uuid, line("Немає дозволу", Tone::Bad));
            return Flow::Cancel;
        }
        let args: Vec<&str> = words.collect();
        let online = Self::online(host);
        let result = self.engine().as_mut().map(|e| e.admin(&args, &online));
        match result {
            Some((lines, effects)) => {
                for l in lines {
                    host.tell(uuid, line(&l, Tone::Info));
                }
                self.after_admin(effects);
            }
            None => {
                host.tell(uuid, line("[Progression] ще не готовий", Tone::Bad));
            }
        }
        Flow::Cancel
    }

    fn perform_console_command(&self, host: &mut dyn Host, event: &mut PerformConsoleCommandEvent<'_>) -> Flow {
        let mut words = event.command.split_whitespace();
        if !words.next().is_some_and(|w| w.eq_ignore_ascii_case("progressadmin")) {
            return Flow::Continue;
        }
        let args: Vec<&str> = words.collect();
        let online = Self::online(host);
        let result = self.engine().as_mut().map(|e| e.admin(&args, &online));
        match result {
            Some((lines, effects)) => {
                for l in lines {
                    host.feedback(Text::literal(l));
                }
                self.after_admin(effects);
            }
            None => host.feedback(Text::literal("[Progression] not ready yet")),
        }
        Flow::Cancel
    }

    fn player_logged_in(&self, host: &mut dyn Host, event: &mut PlayerLoggedInEvent<'_>) {
        let _ = host;
        relock(&self.queue).joined.push((event.player, event.name.to_owned()));
    }

    fn player_logged_out(&self, host: &mut dyn Host, event: &mut PlayerLoggedOutEvent<'_>) {
        let _ = host;
        relock(&self.menus).remove(&event.player);
        {
            let mut c = relock(&self.clients);
            c.horizon.remove(&event.player);
            c.tree.remove(&event.player);
            c.ground.remove(&event.player);
            c.effects.remove(&event.player);
            c.pack.remove(&event.player);
            c.sneaking.remove(&event.player);
            c.health.remove(&event.player);
            c.perks_sent.remove(&event.player);
        }
        let effects = match self.engine().as_mut() {
            Some(e) => e.leave(event.player),
            None => return,
        };
        self.queue(effects);
    }

    fn player_respawned(&self, host: &mut dyn Host, event: &mut PlayerRespawnedEvent<'_>) {
        let _ = host;
        relock(&self.queue).reapply.push(event.player);
    }

    fn player_advancement_done(&self, host: &mut dyn Host, event: &mut PlayerAdvancementDoneEvent<'_>) {
        let _ = host;
        let effects = match self.engine().as_mut() {
            Some(e) => e.advancement(event.player, event.advancement),
            None => return,
        };
        self.queue(effects);
    }

    /// "Кельма": a block placed with Shift becomes a random one of the same shape from the hotbar.
    fn place_block(&self, host: &mut dyn Host, event: &mut PlaceBlockEvent<'_>) {
        self.remote.place_block(host, event);
        if !self.active() {
            return;
        }
        let Some(me) = host.acting_player() else {
            return;
        };
        if !me.sneaking || !self.engine().as_ref().is_some_and(|e| e.tier(me.uuid, "trowel") > 0) {
            return;
        }
        relock(&self.queue).trowels.push((me.uuid, me.dimension.clone(), event.pos, event.stack.item.to_string()));
    }

    /// The altar: its block on its base opens the altar menu for those who learned it.
    fn right_click_block(&self, host: &mut dyn Host, event: &mut RightClickBlockEvent) -> Flow {
        if matches!(self.remote.right_click_block(host, event), Flow::Cancel) {
            return Flow::Cancel;
        }
        if !self.active() {
            return Flow::Continue;
        }
        let Some(me) = host.acting_player() else {
            return Flow::Continue;
        };
        // "Каменяр" (Shift, empty hand) and "Кресляр" (a stick).
        let held = host.held_stack(me.uuid).map(|s| s.item.to_string());
        let tool = match held.as_deref() {
            None if me.sneaking => Some("mason"),
            Some("minecraft:stick") => Some("draft"),
            _ => None,
        };
        let tool = tool.filter(|perk| self.engine().as_ref().is_some_and(|e| e.tier(me.uuid, perk) > 0));
        if let Some(perk) = tool {
            let tick = self.ticks.load(Ordering::Relaxed);
            let again = relock(&self.clicked).insert(me.uuid, (tick, event.pos)) == Some((tick, event.pos));
            if !again {
                match (perk, me.sneaking) {
                    ("mason", _) => relock(&self.queue).masons.push((me.uuid, me.dimension.clone(), event.pos)),
                    (_, true) => {
                        let effects = match self.engine().as_mut() {
                            Some(e) => e.draft_point(me.uuid, &me.dimension, beside(event.pos, event.face)),
                            None => Vec::new(),
                        };
                        self.queue(effects);
                    }
                    _ => relock(&self.queue).actions.push((me.uuid, "draft".to_owned())),
                }
            }
            return Flow::Cancel;
        }
        if me.sneaking {
            return Flow::Continue;
        }
        let Some((altar, base)) = self.engine().as_ref().map(|e| (e.config.settings.altar_block.clone(), e.config.settings.altar_base.clone())) else {
            return Flow::Continue;
        };
        let pos = event.pos;
        if host.block(pos) != altar || host.block([pos[0], pos[1] - 1, pos[2]]) != base {
            return Flow::Continue;
        }
        let learned = self.engine().as_ref().is_some_and(|e| crate::core::config::ALTARS.iter().any(|(_, p)| e.tier(me.uuid, p) > 0));
        {
            // Once, though each hand clicks.
            let mut q = relock(&self.queue);
            if !q.actions.iter().any(|(u, a)| *u == me.uuid && a == "altar") {
                q.actions.push((me.uuid, "altar".to_owned()));
            }
        }
        if learned { Flow::Cancel } else { Flow::Continue }
    }

    /// The anvil keeps levels above vanilla's top, lets the four protections share armour, and
    /// never asks for more levels than the settings say.
    fn anvil_update(&self, host: &mut dyn Host, event: &mut AnvilUpdateEvent<'_>) -> Flow {
        let _ = host;
        let Some((max_cost, tops)) = self.engine().as_ref().map(|e| {
            let tops: HashMap<String, u32> = e.config.books.iter().map(|b| (b.enchant.clone(), b.vanilla_max)).collect();
            (e.config.settings.anvil_max_cost, tops)
        }) else {
            return Flow::Continue;
        };
        let left = levels_of(event.left);
        let right = event.right.map(levels_of).unwrap_or_default();
        match event.vanilla_output {
            Some(vanilla) => {
                if let Some(levels) = engine::anvil_levels(&left, &right, &levels_of(vanilla)) {
                    let mut out = vanilla.clone();
                    set_levels(&mut out, levels);
                    *event.output = Some(out);
                }
            }
            // Vanilla gives nothing at 40 levels and more.
            None if event.vanilla_xp_cost >= 40 => {
                let Some(other) = event.right else {
                    return Flow::Continue;
                };
                let top = |id: &str| tops.get(id).copied().unwrap_or(0);
                if let Some(levels) = engine::anvil_fallback(&left, &right, other.item == event.left.item, &top) {
                    let mut out = event.left.clone();
                    set_levels(&mut out, levels);
                    out.data_mut().repair_cost = (event.left.repair_cost() * 2 + 1).min(MAX_PENALTY);
                    *event.output = Some(out);
                    event.material_cost = 1;
                }
            }
            None => {}
        }
        if let Some(out) = event.output.as_mut()
            && out.repair_cost() > MAX_PENALTY
        {
            out.data_mut().repair_cost = MAX_PENALTY;
        }
        if event.output.is_some() && (event.xp_cost > max_cost || event.vanilla_xp_cost >= 40) {
            event.xp_cost = max_cost.max(1);
        }
        Flow::Continue
    }

    /// "Trophies": a mob killed by its owner may drop one more of each thing.
    fn loot_drops(&self, host: &mut dyn Host, event: &mut LootDropsEvent<'_>) {
        self.remote.loot_drops(host, event);
        if !self.active() || event.context.used {
            return;
        }
        let (Some((_, kind)), Some(player)) = (event.context.entity, event.context.player) else {
            return;
        };
        if kind == "minecraft:player" {
            return;
        }
        let chance = self.engine().as_ref().map_or(0.0, |e| e.bonus(player, "trophies"));
        if chance <= 0.0 {
            return;
        }
        // Not what the mob wore or carried, and not a boss's one prize.
        let plain = |s: &ItemStack| {
            s.count > 0
                && s.damage == 0
                && s.extra.is_none()
                && !matches!(&*s.item, "minecraft:nether_star" | "minecraft:dragon_egg" | "minecraft:bow" | "minecraft:crossbow" | "minecraft:trident" | "minecraft:shield" | "minecraft:saddle")
                && !["_helmet", "_chestplate", "_leggings", "_boots", "_sword", "_axe", "_pickaxe", "_shovel", "_hoe", "_spear", "_horse_armor"]
                    .iter()
                    .any(|end| s.item.ends_with(end))
        };
        let more: Vec<ItemStack> = event
            .drops
            .iter()
            .filter(|s| plain(s))
            .filter(|_| self.chance() < chance)
            .map(|s| ItemStack::new(&s.item, 1))
            .collect();
        for s in more {
            event.drops.push(s);
        }
    }

    /// "Scholar": more vanilla experience from every orb.
    fn player_xp_change(&self, host: &mut dyn Host, event: &mut PlayerXpChangeEvent<'_>) -> Flow {
        let _ = host;
        if event.amount > 0 && self.active() {
            let bonus = self.engine().as_ref().map_or(0.0, |e| e.bonus(event.player, "scholar"));
            if bonus > 0.0 {
                let more = (f64::from(event.amount) * (1.0 + bonus)).round();
                event.amount = if more >= f64::from(i32::MAX) { i32::MAX } else { more as i32 };
            }
        }
        Flow::Continue
    }

    /// Whether the client took the pack: with it the menus get their pictures.
    fn resource_pack_status(&self, host: &mut dyn Host, event: &mut ResourcePackStatusEvent<'_>) {
        let _ = host;
        let mut c = relock(&self.clients);
        match event.status {
            0 => {
                c.pack.insert(event.player);
            }
            // Accepted and downloaded come before it is loaded.
            3 | 4 => {}
            _ => {
                c.pack.remove(&event.player);
            }
        }
    }

    /// A key of the shared client mod (Horizon clients only).
    fn net_from_client(&self, host: &mut dyn Host, event: &mut NetFromClientEvent<'_>) {
        let _ = host;
        if event.channel != KEYS_CHANNEL || event.data.len() > 64 {
            return;
        }
        let Ok(text) = std::str::from_utf8(event.data) else {
            return;
        };
        if let Some(version) = text.strip_prefix("hello") {
            // "hello" (0.2), "hello:3" (0.3 and later: draws the perk map).
            let map = version.strip_prefix(':').and_then(|v| v.parse::<u32>().ok()).is_some_and(|v| v >= 3);
            {
                let mut c = relock(&self.clients);
                c.horizon.insert(event.player);
                if map {
                    c.tree.insert(event.player);
                } else {
                    c.tree.remove(&event.player);
                }
            }
            relock(&self.queue).actions.push((event.player, "hello".to_owned()));
        } else if let Some(id) = text.strip_prefix("progression:buy:")
            && !id.is_empty()
            && id.len() <= 32
            && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            relock(&self.clients).horizon.insert(event.player);
            relock(&self.queue).actions.push((event.player, format!("buy:{id}")));
        } else if let Some(action) = text.strip_prefix("progression:")
            && engine::ACTIONS.contains(&action)
        {
            relock(&self.clients).horizon.insert(event.player);
            relock(&self.queue).actions.push((event.player, action.to_owned()));
        }
    }

    fn player_toggle_sneak(&self, host: &mut dyn Host, event: &mut PlayerToggleSneakEvent<'_>) {
        let _ = host;
        let mut c = relock(&self.clients);
        if event.sneaking {
            c.sneaking.insert(event.player);
        } else {
            c.sneaking.remove(&event.player);
        }
    }

    /// F (swap hands) dashes for a player who owns a dash; Shift+F dashes up.
    fn player_swap_hands(&self, host: &mut dyn Host, event: &mut PlayerSwapHandsEvent<'_>) -> Flow {
        let _ = host;
        let dashes = self.engine().as_ref().is_some_and(|e| e.swap_dashes(event.player));
        if !dashes {
            return Flow::Continue;
        }
        relock(&self.queue).swaps.push(event.player);
        Flow::Cancel
    }

    fn menu_clicked(&self, host: &mut dyn Host, event: &mut MenuClickedEvent) {
        self.remote.menu_clicked(host, event);
        let Some(open) = relock(&self.menus).get(&event.uuid).cloned().filter(|m| m.window == event.window) else {
            return;
        };
        let Ok(slot) = usize::try_from(event.slot) else {
            return;
        };
        // Only a plain left or right click counts (not shift, number keys, drags, double clicks).
        if event.mode != 0 {
            self.show(host, event.uuid, open.page);
            return;
        }
        let Some(me) = host.server_players().iter().find(|p| p.uuid == event.uuid).map(Self::seen_of) else {
            return;
        };
        let clicked = match self.engine().as_mut() {
            Some(e) => e.click(event.uuid, open.page, slot, i32::from(event.button), &me),
            None => return,
        };
        self.apply(host, clicked.effects);
        if clicked.close {
            host.close_menu(event.uuid);
        } else {
            // The same page again even when nothing changed: the client's copy of a clicked slot
            // must be put back.
            self.show(host, event.uuid, clicked.show.unwrap_or(open.page));
        }
    }

    fn menu_closed(&self, host: &mut dyn Host, event: &mut MenuClosedEvent) {
        self.remote.menu_closed(host, event);
        let mut menus = relock(&self.menus);
        if menus.get(&event.uuid).is_some_and(|m| m.window == event.window) {
            menus.remove(&event.uuid);
        }
    }

    /// "Last stand": a lethal blow leaves its owner alive, once in a while.
    fn living_death(&self, host: &mut dyn Host, event: &mut LivingDeathEvent<'_>) -> Flow {
        let _ = host;
        if event.kind != "minecraft:player" || !self.active() {
            return Flow::Continue;
        }
        let Ok(id) = i32::try_from(event.entity) else {
            return Flow::Continue;
        };
        let Some(uuid) = relock(&self.bodies).get(&id).copied() else {
            return Flow::Continue;
        };
        let now = self.now();
        let saved = self.engine().as_mut().is_some_and(|e| e.rescue(uuid, "last_stand", now));
        if !saved {
            return Flow::Continue;
        }
        relock(&self.queue).stands.push(uuid);
        Flow::Cancel
    }

    /// "Lumberjack": an axe's cut on a log fells the tree (not while sneaking).
    fn destroy_block(&self, host: &mut dyn Host, event: &mut DestroyBlockEvent) -> Flow {
        if matches!(self.remote.destroy_block(host, event), Flow::Cancel) {
            return Flow::Cancel;
        }
        if !self.active() {
            return Flow::Continue;
        }
        let Some(me) = host.acting_player() else {
            return Flow::Continue;
        };
        if me.sneaking || !matches!(me.game_mode, 0 | 2) {
            return Flow::Continue;
        }
        let max = self.engine().as_ref().map_or(0, |e| e.timber(me.uuid));
        if max == 0 || !engine::is_log_block(&host.block(event.pos)) {
            return Flow::Continue;
        }
        if !host.held_stack(me.uuid).is_some_and(|s| s.item.ends_with("_axe")) {
            return Flow::Continue;
        }
        relock(&self.queue).fells.push((me.uuid, me.dimension, event.pos, max));
        Flow::Continue
    }

    /// The redstone lab: blasts inside it break no blocks.
    fn explosion_detonate(&self, host: &mut dyn Host, event: &mut ExplosionDetonateEvent<'_>) {
        self.remote.explosion_detonate(host, event);
        let zones: Vec<Zone> = relock(&self.zones).iter().filter(|z| z.no_blast && z.dimension == event.dimension).cloned().collect();
        if zones.is_empty() {
            return;
        }
        event.blocks.retain(|p| !zones.iter().any(|z| z.contains(event.dimension, *p)));
    }

    /// The redstone lab: frozen observers don't pulse.
    fn tick_block(&self, level: &mut dyn LevelWriteHost, event: &mut TickBlockEvent) -> Flow {
        let Ok(zones) = self.zones.try_lock() else {
            return Flow::Continue;
        };
        if !zones.iter().any(|z| z.freeze) {
            return Flow::Continue;
        }
        let dimension = match event.dimension {
            LevelDimension::Overworld => "minecraft:overworld",
            LevelDimension::Nether => "minecraft:the_nether",
            LevelDimension::End => "minecraft:the_end",
        };
        let pos = [event.pos.x, event.pos.y, event.pos.z];
        if !zones.iter().any(|z| z.freeze && z.contains(dimension, pos)) {
            return Flow::Continue;
        }
        drop(zones);
        // An observer about to pulse: it faces any way (up too), is off, and is neither a piston
        // (extended), a lightning rod (waterlogged) nor a dropper (triggered).
        let state = event.state;
        let observer = level.state_with_property(state, "facing", "up").is_some()
            && level.state_with_property(state, "powered", "false") == Some(state)
            && level.state_with_property(state, "extended", "false").is_none()
            && level.state_with_property(state, "waterlogged", "false").is_none();
        if observer { Flow::Cancel } else { Flow::Continue }
    }

    /// A horse's inventory: its speed, jump and health go to the player at the tick.
    fn player_container_open(&self, host: &mut dyn Host, event: &mut PlayerContainerOpenEvent<'_>) {
        let _ = host;
        if event.menu.starts_with("horizon:mount")
            && let Ok(mut q) = self.queue.try_lock()
        {
            q.mounts.push((event.player, event.dimension.to_owned()));
        }
    }

    /// A foal: its speed is set at the tick, from its parents', with twice vanilla's top.
    fn entity_breed(&self, host: &mut dyn Host, event: &mut EntityBreedEvent) {
        let Some(child) = event.child else { return };
        let mut ids = [0u128; 3];
        for (slot, id) in ids.iter_mut().zip([child, event.parent, event.partner]) {
            let Some(e) = host.entity(id) else { return };
            if host.entity_kind(e).as_deref() != Some("minecraft:horse") {
                return;
            }
            let Some(uuid) = host.entity_uuid(e) else { return };
            *slot = uuid;
        }
        if let Ok(mut q) = self.queue.try_lock() {
            q.foals.push(ids);
        }
    }

    /// A player got on or off a horse or a minecart: done at the tick (commands can't run here).
    fn entity_mount(&self, host: &mut dyn Host, event: &mut EntityMountEvent<'_>) -> Flow {
        let horse = crate::core::engine::HORSES.contains(&event.vehicle_kind);
        if !(horse || event.vehicle_kind == "minecraft:minecart") || event.kind != "minecraft:player" {
            return Flow::Continue;
        }
        let Some(me) = host.acting_player() else {
            return Flow::Continue;
        };
        if u32::try_from(me.entity_id).ok() != Some(event.entity) {
            return Flow::Continue;
        }
        let Some(vehicle) = host.entity(event.vehicle).and_then(|v| host.entity_uuid(v)) else {
            return Flow::Continue;
        };
        relock(&self.queue).rides.push(travel::Ride {
            uuid: me.uuid,
            dimension: event.dimension.to_owned(),
            vehicle,
            kind: event.vehicle_kind.to_owned(),
            mounting: event.mounting,
        });
        Flow::Continue
    }

    fn entity_toggle_glide(&self, host: &mut dyn Host, event: &mut EntityToggleGlideEvent<'_>) {
        let _ = host;
        self.glide(event.player, event.gliding);
    }

    fn player_elytra_boost(&self, host: &mut dyn Host, event: &mut PlayerElytraBoostEvent<'_>) {
        let _ = host;
        relock(&self.queue).boosts.push(event.player);
    }

    /// The saddlebags closed: their slots stay in them.
    fn menu_input_closed(&self, host: &mut dyn Host, event: &mut MenuInputClosedEvent<'_>) {
        let _ = host;
        self.bags_closed(event.uuid, event.window, event.slots.as_mut_slice());
    }

    // Remote redstone's own hooks.
    fn level_tick_pre(&self, host: &mut dyn Host, event: &mut LevelTickPreEvent) {
        self.remote.level_tick_pre(host, event);
    }

    fn level_tick_post(&self, host: &mut dyn Host, event: &mut LevelTickPostEvent) {
        self.remote.level_tick_post(host, event);
    }

    fn block_signal(&self, level: &dyn LevelHost, event: &mut BlockSignalEvent) {
        self.remote.block_signal(level, event);
    }

    fn neighbor_changed(&self, level: &dyn LevelHost, event: &mut NeighborChangedEvent) {
        self.remote.neighbor_changed(level, event);
    }

    fn block_changed(&self, level: &dyn LevelHost, event: &mut BlockChangedEvent) {
        self.remote.block_changed(level, event);
    }

    fn piston_move(&self, level: &dyn LevelHost, event: &mut PistonMoveEvent) -> Flow {
        self.remote.piston_move(level, event)
    }

    fn living_incoming_damage(&self, event: &mut LivingIncomingDamageEvent<'_>) -> Flow {
        // "М'яка посадка".
        if self.travel_spares(event.entity, &event.damage_type) {
            return Flow::Cancel;
        }
        if event.kind == "minecraft:player" && event.damage_type == "minecraft:out_of_world" {
            // "Void walker": the void doesn't kill; the tick puts them back on their ground.
            if let Ok(bodies) = self.bodies.try_lock()
                && let Some(uuid) = bodies.get(&event.entity).copied()
            {
                drop(bodies);
                if self.queue.try_lock().is_ok_and(|q| q.falls.contains(&uuid)) {
                    return Flow::Cancel;
                }
                let now = self.now();
                let saved = self.engine.try_lock().is_ok_and(|mut e| e.as_mut().is_some_and(|e| e.rescue(uuid, "void_walker", now)));
                if saved && let Ok(mut q) = self.queue.try_lock() {
                    q.falls.push(uuid);
                    return Flow::Cancel;
                }
            }
            return Flow::Continue;
        }
        if event.kind == "minecraft:player"
            && FALLING.iter().any(|t| *t == event.damage_type)
            && self.heads.try_lock().is_ok_and(|h| h.contains(&event.entity))
        {
            // "Міцна голова".
            return Flow::Cancel;
        }
        if event.kind == "minecraft:player"
            && event.damage_type == "minecraft:drown"
            && self.divers.try_lock().is_ok_and(|h| h.contains(&event.entity))
        {
            // "Дитя води" II.
            return Flow::Cancel;
        }
        if event.kind == "minecraft:player" {
            // A player's melee hit on another player.
            if event.damage_type == "minecraft:player_attack" {
                let m = f32::from_bits(self.pvp.load(Ordering::Relaxed));
                if m.is_finite() && m >= 0.0 && m != 1.0 {
                    event.amount *= m;
                }
            }
        } else if event.causing != 0
            && let Ok(hunters) = self.hunters.try_lock()
            && let Some(m) = hunters.get(&event.causing)
        {
            event.amount *= *m;
        }
        Flow::Continue
    }
}

impl Progression {
    /// "Clear mind" takes harmful effects off; "alchemist" makes new good ones last longer.
    fn tend_effects(&self, host: &mut dyn Host, players: &[NativeServerPlayer]) {
        let wanted: Vec<(u128, Vec<&'static str>, u32)> = match self.engine().as_ref() {
            Some(e) => players
                .iter()
                .map(|p| (p.uuid, e.cleared(p.uuid), e.tier(p.uuid, "alchemist")))
                .filter(|(_, cleared, alchemy)| !cleared.is_empty() || *alchemy > 0)
                .collect(),
            None => return,
        };
        for (uuid, cleared, alchemy) in wanted {
            let Some(effects) = host.effects(uuid) else { continue };
            for fx in &effects {
                if cleared.contains(&fx.id.as_str()) {
                    host.remove_effect(uuid, &fx.id);
                }
            }
            if alchemy == 0 {
                continue;
            }
            let mut boosted = Vec::new();
            {
                let mut c = relock(&self.clients);
                let seen = c.effects.entry(uuid).or_default();
                let mut now_seen = HashMap::new();
                for fx in &effects {
                    // New since the last look, stronger, or longer than it was: just given.
                    let fresh = seen.get(&fx.id).is_none_or(|(amp, left)| fx.amplifier > *amp || fx.duration > *left + 10);
                    let mut shown = (fx.amplifier, fx.duration);
                    if fresh && let Some((amp, duration)) = engine::alchemy(alchemy, &fx.id, fx.amplifier, fx.duration) {
                        boosted.push(NativeEffect { amplifier: amp, duration, ..fx.clone() });
                        shown = (amp, duration);
                    }
                    now_seen.insert(fx.id.clone(), shown);
                }
                *seen = now_seen;
            }
            for fx in boosted {
                if host.add_effect(uuid, &fx) && fx.duration >= 20 * 20 {
                    // Said once, so the longer time is seen right away (the timer then runs down).
                    let secs = fx.duration / 20;
                    let text = format!("Алхімік: {} на {}:{:02}", engine::effect_name(&fx.id), secs / 60, secs % 60);
                    host.action_bar(uuid, line(&text, Tone::Good));
                }
            }
        }
    }

    /// The void walker's way back, the last stand's shield, and the lumberjack's trees.
    fn rescue_and_fell(&self, host: &mut dyn Host, falls: Vec<u128>, stands: Vec<u128>, fells: Vec<(u128, String, [i32; 3], usize)>) {
        for uuid in falls {
            let ground = relock(&self.clients).ground.get(&uuid).cloned();
            let Some((dimension, pos)) = ground else { continue };
            let me = host.server_players().into_iter().find(|p| p.uuid == uuid);
            let (yaw, pitch) = me.map_or((0.0, 0.0), |p| (p.yaw, p.pitch));
            host.teleport_player(uuid, &dimension, [pos[0], pos[1] + 0.5, pos[2]], yaw, pitch);
            host.add_effect(uuid, &NativeEffect { id: "minecraft:slow_falling".into(), amplifier: 0, duration: 200, ambient: false, particles: true });
            host.tell(uuid, line("Безодня тебе не взяла: «Хода над безоднею» повернула на тверду землю", Tone::Good));
        }
        for uuid in stands {
            host.add_effect(uuid, &NativeEffect { id: "minecraft:absorption".into(), amplifier: 1, duration: 200, ambient: false, particles: true });
            host.add_effect(uuid, &NativeEffect { id: "minecraft:regeneration".into(), amplifier: 1, duration: 100, ambient: false, particles: true });
            host.tell(uuid, line("«Остання стійка»: смертельний удар не вбив тебе", Tone::Good));
            let name = host.server_players().into_iter().find(|p| p.uuid == uuid).map(|p| p.name);
            if let Some(name) = name.filter(|n| engine::safe_name(n)) {
                host.run_command(&format!("execute at {name} run playsound minecraft:item.totem.use player @a ~ ~ ~ 1 1.2"));
            }
        }
        for (uuid, dimension, start, max) in fells {
            let Some(level) = host.level(&dimension) else { continue };
            let logs = engine::fell_tree(start, max, |p| host.level_block(level, p).is_some_and(|b| engine::is_log_block(&b)));
            for p in &logs {
                host.run_command(&format!("execute in {dimension} run setblock {} {} {} minecraft:air destroy", p[0], p[1], p[2]));
            }
            if !logs.is_empty() {
                host.action_bar(uuid, line(&format!("Лісоруб: {} колод", logs.len() + 1), Tone::Info));
            }
        }
    }

    /// "Каменяр" turns blocks into the next of their family; "Кельма" swaps placed blocks.
    fn mason_and_trowel(&self, host: &mut dyn Host, players: &[NativeServerPlayer], masons: Vec<(u128, String, [i32; 3])>, trowels: Vec<(u128, String, [i32; 3], String)>) {
        for (uuid, dimension, p) in masons {
            let Some(level) = host.level(&dimension) else { continue };
            let Some(state) = host.level_block_state(level, p) else { continue };
            let next = {
                let h: &dyn Host = &*host;
                engine::mason_next(&state, |id| h.default_state(id).is_some())
            };
            let Some(next) = next else {
                host.action_bar(uuid, line("«Каменяр» не знає, на що перетворити цей блок", Tone::Info));
                continue;
            };
            if host.run_command(&format!("execute in {dimension} run setblock {} {} {} {next} replace", p[0], p[1], p[2])) {
                host.run_command(&format!(
                    "execute in {dimension} run playsound minecraft:ui.stonecutter.take_result block @a {} {} {} 0.8 1",
                    p[0], p[1], p[2]
                ));
            }
        }
        for (uuid, dimension, p, item) in trowels {
            let Some(me) = players.iter().find(|m| m.uuid == uuid) else { continue };
            if !engine::safe_name(&me.name) {
                continue;
            }
            let Some(level) = host.level(&dimension) else { continue };
            let Some(state) = host.level_block_state(level, p) else { continue };
            // Still the block they placed (a block item, not seeds or redstone dust).
            if state.split('[').next() != Some(item.as_str()) {
                continue;
            }
            let hotbar = host.player_stacks(uuid).unwrap_or_default();
            let mut choices: Vec<(String, String)> = Vec::new();
            for s in hotbar.iter().take(9).flatten() {
                let id = s.item.to_string();
                if choices.iter().any(|(c, _)| *c == id) || host.default_state(&id).is_none() {
                    continue;
                }
                if let Some(next) = engine::trowel_state(&state, &id) {
                    choices.push((id, next));
                }
            }
            if choices.is_empty() {
                continue;
            }
            // The placed block itself is one of the draws.
            let pick = (self.chance() * (choices.len() + 1) as f64) as usize;
            let Some((id, next)) = choices.get(pick).cloned() else { continue };
            let creative = me.game_mode == 1;
            if !creative && !host.run_command(&format!("clear {} {id} 1", me.name)) {
                continue;
            }
            if host.run_command(&format!("execute in {dimension} run setblock {} {} {} {next} replace", p[0], p[1], p[2])) {
                if !creative {
                    host.run_command(&format!("give {} {item} 1", me.name));
                }
            } else if !creative {
                host.run_command(&format!("give {} {id} 1", me.name));
            }
        }
    }

    /// "Кресляр" II: places the fills' blocks, a few each tick, from the player's inventory. One
    /// fill works at a time and they take turns; a row of blocks is one `fill ... strict`, which
    /// sends no neighbour updates (no observers firing, no sand falling, no redstone around).
    fn fill(&self, host: &mut dyn Host, players: &[NativeServerPlayer]) {
        let mut jobs = std::mem::take(&mut *relock(&self.fills));
        if jobs.is_empty() {
            return;
        }
        let mut job = jobs.remove(0);
        let more = (|| -> bool {
            let job = &mut job;
            let Some(me) = players.iter().find(|m| m.uuid == job.uuid) else { return false };
            if !engine::safe_name(&me.name) {
                return false;
            }
            let Some(level) = host.level(&job.dimension) else { return false };
            let take = job.blocks.len().min(FILL_PER_TICK);
            let batch: Vec<[i32; 3]> = job.blocks.drain(..take).collect();
            let empty: Vec<[i32; 3]> = batch
                .into_iter()
                .filter(|p| host.level_block(level, *p).is_some_and(|b| matches!(b.as_str(), "minecraft:air" | "minecraft:cave_air")))
                .collect();
            let creative = me.game_mode == 1;
            let have = if creative {
                empty.len()
            } else {
                let carried: u32 = host
                    .player_stacks(job.uuid)
                    .unwrap_or_default()
                    .iter()
                    .flatten()
                    .chain(host.player_equipment(job.uuid, EquipmentSlot::OffHand).as_ref())
                    .filter(|s| s.item.to_string() == job.item)
                    .map(|s| s.count)
                    .sum();
                empty.len().min(carried as usize)
            };
            if have > 0 && !creative && !host.run_command(&format!("clear {} {} {have}", me.name, job.item)) {
                return false;
            }
            for (from, to) in engine::runs(&empty[..have]) {
                let cmd = format!(
                    "execute in {} run fill {} {} {} {} {} {} {} strict",
                    job.dimension, from[0], from[1], from[2], to[0], to[1], to[2], job.item
                );
                if host.run_command(&cmd) {
                    job.placed += (to[0] - from[0] + 1) as usize;
                }
            }
            let out = have < empty.len();
            if out || job.blocks.is_empty() {
                let text = if out { format!("Кресляр: залито {} блоків, блоки скінчились", job.placed) } else { format!("Кресляр: залито {} блоків", job.placed) };
                host.action_bar(job.uuid, line(&text, if out { Tone::Bad } else { Tone::Good }));
                return false;
            }
            true
        })();
        // The one that worked goes to the back: the next player's fill goes next tick.
        if more {
            jobs.push(job);
        }
        relock(&self.fills).extend(jobs);
    }

    /// "Кресляр": each drawing as particles only its owner sees.
    fn show_drafts(&self, host: &mut dyn Host, players: &[NativeServerPlayer]) {
        let drafts = match self.engine().as_ref() {
            Some(e) => e.drafts(),
            None => return,
        };
        for (uuid, dimension, points) in drafts {
            let Some(me) = players.iter().find(|m| m.uuid == uuid && m.dimension == dimension) else { continue };
            if !engine::safe_name(&me.name) {
                continue;
            }
            for (i, p) in points.iter().enumerate() {
                // Dust stays where it is put (an end rod's spark drifts away from its block).
                let particle = if i == 0 { "minecraft:dust{color:[0.3,1.0,0.3],scale:1.6}" } else { "minecraft:dust{color:[0.4,0.85,1.0],scale:1.2}" };
                host.run_command(&format!(
                    "execute in {dimension} run particle {particle} {}.5 {}.5 {}.5 0 0 0 0 1 force {}",
                    p[0], p[1], p[2], me.name
                ));
            }
        }
    }

    /// After an admin command: the multiplier may have been reloaded; the rest waits for the tick.
    fn after_admin(&self, effects: Vec<Effect>) {
        if let Some(e) = self.engine().as_ref() {
            self.pvp.store(e.pvp_multiplier().to_bits(), Ordering::Relaxed);
        }
        let mut lines = Vec::new();
        let mut rest = Vec::new();
        for e in effects {
            match e {
                Effect::Console(t) => lines.push(t),
                other => rest.push(other),
            }
        }
        for l in lines {
            log::info!(target: "horizon::progression", "{l}");
        }
        self.queue(rest);
    }
}
