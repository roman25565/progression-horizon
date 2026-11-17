// Progression for Horizon: attribute modifiers waiting to be set on players. It lives in the
// client library (`horizon_client::progression_attrs`); the mod's trait package pushes here, and
// `progression_glue` (in the game) sets them at the next server tick, as each player.

pub struct Op {
    pub uuid: u128,
    pub attribute: String,
    pub id: String,
    /// `Some((amount, operation))` adds after removing; `None` only removes.
    pub add: Option<(f64, String)>,
    /// Ticks waited for a player who isn't placed yet (joining).
    pub waited: u32,
}

static QUEUE: std::sync::Mutex<Vec<Op>> = std::sync::Mutex::new(Vec::new());

fn queue() -> std::sync::MutexGuard<'static, Vec<Op>> {
    QUEUE.lock().unwrap_or_else(|p| p.into_inner())
}

/// Sets (or with `add` None takes away) the modifier `id` of `attribute` on the player `uuid`.
pub fn push(uuid: u128, attribute: &str, id: &str, add: Option<(f64, String)>) {
    let mut q = queue();
    if q.len() < 4096 {
        q.push(Op { uuid, attribute: attribute.to_owned(), id: id.to_owned(), add, waited: 0 });
    }
}

pub fn take() -> Vec<Op> {
    std::mem::take(&mut *queue())
}

pub fn put_back(mut ops: Vec<Op>) {
    if ops.is_empty() {
        return;
    }
    let mut q = queue();
    ops.append(&mut q);
    *q = ops;
}

/// Minecart riders with "Швидкі колії" or "Розгін": player, track steps a tick, forward drives.
static CARTS: std::sync::Mutex<Vec<(u128, u32, bool)>> = std::sync::Mutex::new(Vec::new());

/// Every rider's minecart perks (the mod sets the whole list now and then).
pub fn set_carts(list: Vec<(u128, u32, bool)>) {
    *CARTS.lock().unwrap_or_else(|p| p.into_inner()) = list;
}

/// A player's minecart perks: track steps a tick and whether forward drives.
pub fn cart(uuid: u128) -> Option<(u32, bool)> {
    CARTS.lock().unwrap_or_else(|p| p.into_inner()).iter().find(|c| c.0 == uuid).map(|c| (c.1, c.2))
}
