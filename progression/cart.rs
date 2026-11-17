// After a server player's minecart ticked with them: "Розгін" and "Швидкі колії".
fn progression_cart(game: &mut crate::game::Game<'_>, keys: Keys, value: bool) -> bool {
    crate::game::progression_glue::cart(game, keys, value)
}
