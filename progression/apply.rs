// Progression for Horizon: sets the attribute modifiers the mod asked for
// (`horizon_client::progression_attrs`) on each player, with that player's state in place, as
// `/attribute` does for the player who runs it (Horizon's `/attribute` doesn't reach other players).

use super::Game;

/// A player not placed yet (joining) is waited for this many ticks.
const WAIT: u32 = 400;

pub(super) fn apply(game: &mut Game<'_>) {
    if game.lan_acting.is_some() {
        return;
    }
    let ops = horizon_client::progression_attrs::take();
    if ops.is_empty() {
        return;
    }
    let Some(mut host) = game.lan.take() else {
        horizon_client::progression_attrs::put_back(ops);
        return;
    };
    let mut later = Vec::new();
    for mut op in ops {
        let conn = host
            .hub
            .guests
            .iter()
            .find(|g| g.placed && g.uuid == op.uuid)
            .map(|g| g.conn);
        let Some(conn) = conn else {
            op.waited += 1;
            if op.waited < WAIT {
                later.push(op);
            }
            continue;
        };
        let _ = game.as_guest(&mut host, conn, |g| {
            let Some(p) = g.player.as_mut() else {
                return;
            };
            // set_modifier replaces a modifier of the same id (and keeps the hearts unless the
            // new maximum is lower).
            let result = match &op.add {
                Some((amount, operation)) => p.set_modifier(&op.attribute, &op.id, *amount, operation, true),
                None => p.set_modifier(&op.attribute, &op.id, 0.0, "add_value", false),
            };
            if let Err(e) = result {
                log::warn!("[Progression] {} {}: {e:?}", op.attribute, op.id);
            }
        });
    }
    game.lan = Some(host);
    horizon_client::progression_attrs::put_back(later);
}

thread_local! {
    /// Inside the extra track steps of a minecart: they don't ask for more.
    static EXTRA: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// "Розгін": what forward adds to a ridden minecart's speed a tick, and the speed it stops at
/// (blocks a tick; a track step takes at most 0.4, and a ridden cart 0.75 of its speed).
const DRIVE_PUSH: f64 = 0.04;
const DRIVE_TOP: f64 = 0.6;

/// After a server player's minecart ticked with them (`tick_cart_riding`, as that player):
/// "Розгін" speeds it up while forward is held, "Швидкі колії" ticks it again, each time a whole
/// vanilla step along the rails, so it goes faster and still takes the curves.
pub(super) fn cart(game: &mut Game<'_>, keys: horizon_client::sprint::Keys, ridden: bool) -> bool {
    if !ridden || EXTRA.with(|e| e.get()) {
        return ridden;
    }
    let (Some(uuid), Some(id)) = (game.lan_acting_uuid, game.riding) else {
        return ridden;
    };
    let Some((steps, drive)) = horizon_client::progression_attrs::cart(uuid) else {
        return ridden;
    };
    if drive && keys.forward {
        let yaw = game.yaw.to_radians();
        if let Some(i) = game.carts.iter().position(|c| c.id == id) {
            let c = &mut game.carts[i];
            let h = c.velocity.x.hypot(c.velocity.z);
            if h < 0.01 {
                // From a standstill: where the rider looks; the rails take what lies along them.
                c.velocity.x = -f64::from(yaw.sin()) * 0.1;
                c.velocity.z = f64::from(yaw.cos()) * 0.1;
            } else if h < DRIVE_TOP {
                let k = (h + DRIVE_PUSH).min(DRIVE_TOP) / h;
                c.velocity.x *= k;
                c.velocity.z *= k;
            }
        }
    }
    if steps > 1 {
        EXTRA.with(|e| e.set(true));
        // Sneaking gets out on the first step only.
        let calm = horizon_client::sprint::Keys { shift: false, ..keys };
        for _ in 1..steps {
            if game.riding != Some(id) {
                break;
            }
            game.tick_cart_riding(calm);
        }
        EXTRA.with(|e| e.set(false));
    }
    ridden
}
