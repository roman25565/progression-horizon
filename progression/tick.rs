// Each server tick: the attribute modifiers the mod asked for.
fn progression_tick(game: &mut crate::game::Game<'_>) {
    crate::game::progression_glue::apply(game);
}
