use progression_core_tests::core::config::{BRANCHES, Config, DEFAULT_CONFIG, ROW_PERKS, rewardable};
use progression_core_tests::core::engine::*;
use progression_core_tests::core::gui::{self, Frame};
use progression_core_tests::core::sha1::sha1_hex;
use std::path::PathBuf;

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("progression-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

const U: u128 = 0x1234;

fn engine(tag: &str) -> (Engine, PathBuf) {
    let dir = tmp(tag);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("advancements.toml"), "[points]\n\"minecraft:story/mine_stone\" = 1\n\"minecraft:nether/all_effects\" = 5\n").unwrap();
    let (mut e, _) = Engine::open(&dir);
    e.join(U, "Roman");
    (e, dir)
}

fn me(levels: i32) -> Seen {
    Seen { uuid: U, name: "Roman".into(), dimension: "minecraft:overworld".into(), position: [100.5, 64.0, -20.5], xp_level: levels, ..Seen::default() }
}

fn level(e: &mut Engine, n: u32) {
    e.admin(&["level", "Roman", &n.to_string()], &[(U, "Roman".into())]);
}

#[test]
fn the_default_settings_are_valid_and_the_tree_is_affordable() {
    let c = Config::parse(DEFAULT_CONFIG).unwrap();
    // Everyone can learn everything by the top level.
    assert!(c.points_by(c.settings.max_level) >= c.tree_cost(), "{} < {}", c.points_by(c.settings.max_level), c.tree_cost());
    // ...but not long before it.
    assert!(c.points_by(c.settings.max_level * 3 / 4) < c.tree_cost());
    for b in BRANCHES {
        let n = c.perks.iter().filter(|p| p.branch == b).count();
        assert!((1..=ROW_PERKS).contains(&n), "{b}: {n}");
    }
    // Every perk's last tier opens by the top level.
    for p in &c.perks {
        assert!(*p.levels.last().unwrap() <= c.settings.max_level, "{}", p.id);
    }
}

#[test]
fn no_perk_gives_a_beacon_or_potion_effect() {
    let c = Config::parse(DEFAULT_CONFIG).unwrap();
    let banned = ["movement_speed", "attack_damage", "mining_efficiency", "block_break_speed", "jump_strength", "attack_speed", "luck"];
    for p in &c.perks {
        for a in p.attribute.iter().chain(p.also.iter().map(|x| &x.attribute)) {
            assert!(!banned.iter().any(|b| a.ends_with(b)), "{}: {a}", p.id);
        }
    }
    let (mut e, dir) = engine("noeffects");
    level(&mut e, 50);
    let ids: Vec<String> = e.config.perks.iter().map(|p| p.id.clone()).collect();
    let mut all = Vec::new();
    for id in ids {
        for _ in 0..6 {
            all.extend(e.buy(U, &id, &me(100)));
        }
    }
    all.extend(e.reapply(U));
    assert!(!all.iter().any(|x| matches!(x, Effect::Command(c) if c.contains("effect give"))));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn experience_raises_the_level_and_gives_points() {
    let (mut e, dir) = engine("xp");
    assert_eq!(e.data(U).unwrap().level, 0);
    e.add_xp(U, 199, false);
    assert_eq!((e.data(U).unwrap().level, e.data(U).unwrap().xp), (0, 199));
    let out = e.add_xp(U, 1, false);
    let d = e.data(U).unwrap();
    assert_eq!((d.level, d.xp, d.points), (1, 0, 4));
    assert!(out.iter().any(|x| matches!(x, Effect::Tell(_, t, Tone::Good) if t.contains("Рівень прогресії 1"))));
    // Several levels at once, with the bonus of level 5: 240+280+320+360 = 1200.
    e.add_xp(U, 1200, false);
    let d = e.data(U).unwrap();
    assert_eq!((d.level, d.points), (5, 4 * 5 + 5));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_level_stops_at_the_top() {
    let (mut e, dir) = engine("top");
    e.add_xp(U, 10_000_000, false);
    let d = e.data(U).unwrap();
    assert_eq!((d.level, d.xp), (50, 0));
    assert_eq!(d.points, e.config.points_by(50));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_level_is_kept_on_disk_not_in_the_player() {
    let (mut e, dir) = engine("keep");
    e.add_xp(U, 500, false);
    let before = e.data(U).unwrap().clone();
    // A death changes nothing here; a leave and a join read the same progress back.
    e.leave(U);
    let (mut e, _) = Engine::open(&dir);
    e.join(U, "Roman");
    assert_eq!(e.data(U).unwrap().level, before.level);
    assert_eq!(e.data(U).unwrap().points, before.points);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn only_advancements_give_experience_and_each_only_once() {
    let (mut e, dir) = engine("sources");
    // A datapack's or a mod's advancement (a new structure) counts like a vanilla task.
    e.advancement(U, "ctov:explore/village");
    assert_eq!(e.data(U).unwrap().total_xp, 120);
    e.advancement(U, "ctov:explore/village");
    assert_eq!(e.data(U).unwrap().total_xp, 120);
    // Recipe unlocks and a tab's root give nothing.
    e.advancement(U, "minecraft:recipes/misc/stick");
    e.advancement(U, "minecraft:story/root");
    assert_eq!(e.data(U).unwrap().total_xp, 120);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_advancement_counts_once_by_its_weight() {
    let (mut e, dir) = engine("adv");
    e.advancement(U, "minecraft:nether/all_effects");
    assert_eq!(e.data(U).unwrap().total_xp, 600);
    e.advancement(U, "minecraft:nether/all_effects");
    assert_eq!(e.data(U).unwrap().total_xp, 600);
    e.advancement(U, "minecraft:end/kill_dragon");
    assert_eq!(e.data(U).unwrap().total_xp, 600 + 25 * 120);
    e.advancement(U, "minecraft:recipes/misc/stick");
    assert_eq!(e.data(U).unwrap().total_xp, 3600);
    assert!(!rewardable("minecraft:story/root"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_perk_needs_its_level_and_its_points() {
    let (mut e, dir) = engine("gate");
    level(&mut e, 1);
    // Level 1, 4 points: health I (level 1, 2 points) yes, fall I (level 3) no.
    let out = e.buy(U, "fall", &me(0));
    assert!(matches!(&out[0], Effect::ActionBar(_, t) if t.contains("рівень прогресії 3")));
    let out = e.buy(U, "health", &me(0));
    assert!(out.contains(&Effect::Attribute(U, "minecraft:max_health".into(), "progression:health".into(), Some((4.0, "add_value".into())))));
    assert_eq!(e.data(U).unwrap().points, 2);
    // Health II opens at level 8.
    level(&mut e, 7);
    let out = e.buy(U, "health", &me(0));
    assert!(matches!(&out[0], Effect::ActionBar(_, t) if t.contains("рівень прогресії 8")));
    level(&mut e, 8);
    e.buy(U, "health", &me(0));
    assert_eq!(e.tier(U, "health"), 2);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_3x3_tools_come_with_the_perk_and_more_cost_vanilla_levels() {
    let (mut e, dir) = engine("tools");
    level(&mut e, 20);
    let out = e.buy(U, "pickaxe3", &me(0));
    assert!(out.contains(&Effect::Sell { uuid: U, levels: 0, item: Item::Pickaxe3 }));
    let out = e.buy(U, "pickaxe3", &me(10));
    assert!(matches!(&out[0], Effect::ActionBar(_, t) if t.contains("25 ванільних")));
    let out = e.buy(U, "pickaxe3", &me(25));
    assert_eq!(out, vec![Effect::Sell { uuid: U, levels: 25, item: Item::Pickaxe3 }]);
    let out = e.buy(U, "shovel3", &me(0));
    assert!(out.contains(&Effect::Sell { uuid: U, levels: 0, item: Item::Shovel3 }));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn each_altar_tier_raises_its_group_of_books_by_one() {
    let (mut e, dir) = engine("books");
    level(&mut e, 50);
    let b = e.config.book("minecraft:sharpness").unwrap().clone();
    // No altar perk: nothing, and the altar block doesn't open.
    assert!(matches!(&e.buy_book(U, &b, &me(100))[0], Effect::ActionBar(..)));
    assert!(e.action(U, "altar", 0, &me(0)).open.is_none());
    // A tools altar opens the altar, but not weapon books.
    e.buy(U, "altar_tools", &me(0));
    assert_eq!(e.action(U, "altar", 0, &me(0)).open, Some(Page::Altar));
    assert!(matches!(&e.buy_book(U, &b, &me(100))[0], Effect::ActionBar(..)));
    e.buy(U, "altar_weapons", &me(0));
    assert_eq!(
        e.buy_book(U, &b, &me(40)),
        vec![Effect::Sell { uuid: U, levels: 40, item: Item::Book { enchant: "minecraft:sharpness".into(), level: 6 } }]
    );
    assert!(matches!(&e.buy_book(U, &b, &me(39))[0], Effect::ActionBar(..)));
    e.buy(U, "altar_weapons", &me(0));
    e.buy(U, "altar_weapons", &me(0));
    assert!(matches!(&e.buy_book(U, &b, &me(40))[0], Effect::Sell { item: Item::Book { level: 8, .. }, .. }));
    // Protection stops at V whatever the armour altar's tier.
    let p = e.config.book("minecraft:protection").unwrap().clone();
    for _ in 0..3 {
        e.buy(U, "altar_armor", &me(0));
    }
    assert_eq!(e.tier(U, "altar_armor"), 3);
    assert!(matches!(&e.buy_book(U, &p, &me(100))[0], Effect::Sell { item: Item::Book { level: 5, .. }, .. }));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_anvil_keeps_levels_above_vanillas_top() {
    let l = |v: &[(&str, u32)]| v.iter().map(|(e, n)| ((*e).to_owned(), *n)).collect::<Vec<_>>();
    // Sword with Sharpness III and a Sharpness VII book: vanilla says V.
    let out = anvil_levels(&l(&[("minecraft:sharpness", 3)]), &l(&[("minecraft:sharpness", 7)]), &l(&[("minecraft:sharpness", 5)]));
    assert_eq!(out, Some(l(&[("minecraft:sharpness", 7)])));
    // Two Sharpness VI swords: vanilla would cut to V.
    let out = anvil_levels(&l(&[("minecraft:sharpness", 6)]), &l(&[("minecraft:sharpness", 6)]), &l(&[("minecraft:sharpness", 5)]));
    assert_eq!(out, Some(l(&[("minecraft:sharpness", 6)])));
    // A plain combine is vanilla's own.
    assert_eq!(anvil_levels(&l(&[("minecraft:sharpness", 4)]), &l(&[("minecraft:sharpness", 4)]), &l(&[("minecraft:sharpness", 5)])), None);
    // Protections share armour.
    let out = anvil_levels(&l(&[("minecraft:protection", 5)]), &l(&[("minecraft:fire_protection", 4)]), &l(&[("minecraft:protection", 4)]));
    assert_eq!(out, Some(l(&[("minecraft:protection", 5), ("minecraft:fire_protection", 4)])));
    // ...but a protection book doesn't land on a sword.
    assert_eq!(anvil_levels(&l(&[("minecraft:sharpness", 5)]), &l(&[("minecraft:fire_protection", 4)]), &l(&[("minecraft:sharpness", 5)])), None);
}

#[test]
fn a_respec_gives_the_points_back_for_levels() {
    let (mut e, dir) = engine("respec");
    level(&mut e, 10);
    let had = e.data(U).unwrap().points;
    e.buy(U, "health", &me(0));
    e.buy(U, "health", &me(0));
    e.buy(U, "step", &me(0));
    let spent = e.spent(e.data(U).unwrap());
    assert_eq!(spent, 6);
    assert!(matches!(&e.respec(U, &me(5))[0], Effect::ActionBar(..)));
    let out = e.respec(U, &me(6));
    assert_eq!(out[0], Effect::TakeLevels(U, 6));
    assert!(out.contains(&Effect::Attribute(U, "minecraft:max_health".into(), "progression:health".into(), None)));
    let d = e.data(U).unwrap();
    assert_eq!((d.points, d.level, d.perks.len()), (had, 10, 0));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn skills_need_the_perk_and_wait_for_their_cooldown() {
    let (mut e, dir) = engine("skills");
    let a = e.action(U, "dash_forward", 0, &me(0));
    assert!(matches!(&a.effects[0], Effect::ActionBar(_, t) if t.contains("не вивчено")));
    level(&mut e, 14);
    e.buy(U, "dash_forward", &me(0));
    e.buy(U, "dash_up", &me(0));
    assert_eq!(e.action(U, "dash_forward", 100, &me(0)).effects, vec![Effect::Dash(U, "Roman".into(), "dash_forward")]);
    assert!(matches!(&e.action(U, "dash_forward", 150, &me(0)).effects[0], Effect::ActionBar(_, t) if t.contains("ще 3 с")));
    assert_eq!(e.action(U, "dash_forward", 200, &me(0)).effects.len(), 1);
    assert_eq!(e.swap_dash(U, true), Some("dash_up"));
    assert_eq!(e.swap_dash(U, false), Some("dash_forward"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_anchor_holds_chunks_around_the_player_until_they_leave() {
    let (mut e, dir) = engine("anchor");
    level(&mut e, 28);
    e.buy(U, "anchor", &me(0));
    let out = e.action(U, "anchor", 0, &me(0)).effects;
    let Effect::Anchor { chunks, hold, dimension, .. } = &out[0] else { panic!("{out:?}") };
    assert!(*hold && chunks.len() == 9 && dimension == "minecraft:overworld");
    assert!(chunks.contains(&[6, -2]));
    // The second tier widens the anchor in place.
    let out = e.buy(U, "anchor", &me(0));
    assert!(out.iter().any(|x| matches!(x, Effect::Anchor { chunks, hold: true, .. } if chunks.len() == 25)));
    // Leaving lets go; joining holds again.
    let out = e.leave(U);
    assert!(out.iter().any(|x| matches!(x, Effect::Anchor { chunks, hold: false, .. } if chunks.len() == 25)));
    let out = e.join(U, "Roman");
    assert!(out.iter().any(|x| matches!(x, Effect::Anchor { hold: true, .. })));
    // The key again takes it away.
    let out = e.action(U, "anchor", 0, &me(0)).effects;
    assert!(matches!(&out[0], Effect::Anchor { hold: false, .. }));
    assert!(e.data(U).unwrap().anchor.is_none());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_spirit_leaves_a_body_and_comes_back() {
    let (mut e, dir) = engine("spirit");
    level(&mut e, 12);
    e.buy(U, "spirit", &me(0));
    e.hurt(U, 100);
    assert!(matches!(&e.action(U, "spirit", 150, &me(0)).effects[0], Effect::ActionBar(_, t) if t.contains("Після шкоди")));
    let out = e.action(U, "spirit", 400, &me(0)).effects;
    assert!(out.iter().any(|x| matches!(x, Effect::Command(c) if c == "gamemode spectator Roman")));
    assert!(e.data(U).unwrap().spirit.is_some());
    // Too far: pulled back.
    let far = Seen { position: [200.0, 64.0, -20.5], game_mode: 3, ..me(0) };
    assert!(e.spirits(&[far]).iter().any(|x| matches!(x, Effect::Command(c) if c.contains(" tp Roman "))));
    // A crash: the join puts them back.
    e.leave(U);
    let out = e.join(U, "Roman");
    assert!(out.iter().any(|x| matches!(x, Effect::Command(c) if c == "gamemode survival Roman")));
    assert!(e.data(U).unwrap().spirit.is_none());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn special_perks_give_their_bonus() {
    let (mut e, dir) = engine("bonus");
    level(&mut e, 15);
    e.buy(U, "hunter", &me(0));
    e.buy(U, "hunter", &me(0));
    assert!((e.bonus(U, "hunter") - 0.20).abs() < 1e-9);
    e.buy(U, "wisdom", &me(0));
    let before = e.data(U).unwrap().total_xp;
    e.advancement(U, "minecraft:story/mine_stone");
    assert_eq!(e.data(U).unwrap().total_xp - before, 132);
    assert_eq!(e.bonus(U, "trophies"), 0.0);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_perk_menu_shows_every_perk_with_its_state() {
    let (mut e, dir) = engine("menu");
    level(&mut e, 1);
    let m = e.menu(U, Page::Perks, true);
    assert_eq!((m.rows, m.cells.len()), (6, 54));
    let shown = m.cells.iter().take(45).flatten().count();
    assert_eq!(shown, 5 + e.config.perks.len());
    // Health: row 0, column 1, can be bought; fall next to it is locked by level.
    assert_eq!(m.cells[1].as_ref().unwrap().frame, Frame::Buy);
    assert_eq!(m.cells[2].as_ref().unwrap().frame, Frame::Locked);
    assert_eq!(m.label, "Рівень 1 · ОП 4");
    let out = e.click(U, Page::Perks, 1, 0, &me(0));
    assert!(out.effects.iter().any(|x| matches!(x, Effect::Tell(_, t, _) if t.contains("Здоров'я I"))));
    let m = e.menu(U, Page::Perks, true);
    assert_eq!(m.cells[1].as_ref().unwrap().frame, Frame::Owned);
    assert_eq!(e.click(U, Page::Perks, SLOT_RESPEC, 0, &me(0)).show, Some(Page::Respec));
    assert!(e.click(U, Page::Perks, SLOT_CLOSE, 0, &me(0)).close);
    // Without the pack: no glyphs, panes in the empty slots.
    let plain = e.menu(U, Page::Perks, false);
    assert!(plain.glyphs.is_empty() && plain.cells.iter().all(Option::is_some));
    assert!(m.cells.iter().any(Option::is_none));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_title_glyphs_end_where_they_started() {
    let (mut e, dir) = engine("glyphs");
    level(&mut e, 30);
    e.add_xp(U, 333, false);
    for page in [Page::Perks, Page::Altar, Page::Respec, Page::Lab] {
        let m = e.menu(U, page, true);
        assert!(!m.glyphs.is_empty());
        assert_eq!(gui::advance(&m.glyphs), 0, "{page:?}");
        assert!(m.glyphs.chars().all(|c| (0xE000..=0xF8FF).contains(&(c as u32))));
    }
    // An empty bar and a full one.
    assert_eq!(gui::advance(&gui::glyphs(Page::Perks, &[(1, Frame::Buy), (53, Frame::Max)], Some(0.0))), 0);
    assert_eq!(gui::advance(&gui::glyphs(Page::Perks, &[], Some(1.0))), 0);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_old_file_turns_its_points_into_experience() {
    let dir = tmp("old");
    std::fs::create_dir_all(dir.join("players")).unwrap();
    std::fs::write(
        dir.join("players").join(format!("{U:032x}.json")),
        r#"{"name":"Roman","points":3,"earned":20,"chains":{"health":2,"strength":1},"quests":["minecraft:story/mine_stone"],"skills":["night_vision"]}"#,
    )
    .unwrap();
    let (mut e, _) = Engine::open(&dir);
    let out = e.join(U, "Roman");
    let d = e.data(U).unwrap();
    assert_eq!(d.total_xp, 2400);
    assert!(d.level >= 5 && d.perks.is_empty() && d.quests.len() == 1);
    assert_eq!(d.points, e.config.points_by(d.level));
    assert!(!out.iter().any(|x| matches!(x, Effect::Command(c) if c.contains("effect give"))));
    // Once only.
    e.leave(U);
    e.join(U, "Roman");
    assert_eq!(e.data(U).unwrap().total_xp, 2400);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn admins_can_give_and_reset() {
    let (mut e, dir) = engine("admin");
    let on = [(U, "Roman".to_owned())];
    e.admin(&["xp", "Roman", "200"], &on);
    assert_eq!(e.data(U).unwrap().level, 1);
    e.admin(&["points", "Roman", "10"], &on);
    assert_eq!(e.data(U).unwrap().points, 14);
    e.admin(&["points", "Roman", "-100"], &on);
    assert_eq!(e.data(U).unwrap().points, 0);
    let (lines, _) = e.admin(&["info", "Nobody"], &on);
    assert!(lines[0].contains("ще не було"));
    e.admin(&["reset", "Roman"], &on);
    assert_eq!(e.data(U).unwrap().level, 0);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn bad_settings_are_refused() {
    let bad = DEFAULT_CONFIG.replace("branch = \"pvp\"", "branch = \"magic\"");
    assert!(Config::parse(&bad).unwrap_err().contains("branch"));
    let bad = DEFAULT_CONFIG.replace("costs = [2, 2, 3, 3, 4]", "costs = [2, 2]");
    assert!(Config::parse(&bad).is_err());
    let bad = DEFAULT_CONFIG.replace("id = \"hunter\"", "id = \"mystery\"");
    assert!(Config::parse(&bad).unwrap_err().contains("attribute"));
}

#[test]
fn sha1_matches_the_known_answers() {
    assert_eq!(sha1_hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
    assert_eq!(sha1_hex(&[b'a'; 1000]), "291e9a6c66994949b57ba5e650361e98fc36b1ba");
}

#[test]
fn a_too_expensive_anvil_still_merges() {
    let e = |id: &str, l: u32| (format!("minecraft:{id}"), l);
    let top = |id: &str| if id == "minecraft:sharpness" { 5 } else { 3 };
    // A book only raises what the item already has.
    let got = anvil_fallback(&[e("sharpness", 5), e("unbreaking", 3)], &[e("sharpness", 7), e("looting", 3)], false, &top);
    assert_eq!(got, Some(vec![e("sharpness", 7), e("unbreaking", 3)]));
    // Equal levels rise by one only within vanilla's top.
    assert_eq!(anvil_fallback(&[e("sharpness", 4)], &[e("sharpness", 4)], false, &top), Some(vec![e("sharpness", 5)]));
    assert_eq!(anvil_fallback(&[e("sharpness", 6)], &[e("sharpness", 6)], false, &top), None);
    // The same kind of item brings new ones, but not clashing ones.
    let got = anvil_fallback(&[e("sharpness", 5)], &[e("smite", 5), e("looting", 3)], true, &top);
    assert_eq!(got, Some(vec![e("sharpness", 5), e("looting", 3)]));
    let got = anvil_fallback(&[e("loyalty", 3)], &[e("channeling", 1), e("riptide", 3)], true, &top);
    assert_eq!(got, Some(vec![e("loyalty", 3), e("channeling", 1)]));
}

#[test]
fn settings_of_an_older_version_are_kept_aside() {
    let dir = tmp("oldconfig");
    std::fs::create_dir_all(&dir).unwrap();
    let old = "[settings]\npvp_damage_multiplier = 2.0\n\n[[chain]]\nid = \"health\"\n";
    std::fs::write(dir.join("progression.toml"), old).unwrap();
    let (e, log) = Engine::open(&dir);
    assert!(log.iter().any(|l| l.contains("older version")), "{log:?}");
    assert_eq!(std::fs::read_to_string(dir.join("progression-v0.toml")).unwrap(), old);
    assert_eq!(std::fs::read_to_string(dir.join("progression.toml")).unwrap(), DEFAULT_CONFIG);
    assert_eq!(e.config.perks.len(), 32);
    // A 2.0 file parses, but its version is older (it still gave experience for kills): it goes too.
    let v1 = DEFAULT_CONFIG.replace("version = 4", "version = 3");
    std::fs::write(dir.join("progression.toml"), &v1).unwrap();
    let (_, log) = Engine::open(&dir);
    assert!(log.iter().any(|l| l.contains("older version")), "{log:?}");
    assert_eq!(std::fs::read_to_string(dir.join("progression-v3.toml")).unwrap(), v1);
    // A broken file of this version is left for its owner to fix.
    let broken = "[settings]\nversion = 4\nmax_level = \"many\"\n";
    std::fs::write(dir.join("progression.toml"), broken).unwrap();
    let (_, log) = Engine::open(&dir);
    assert!(log.iter().any(|l| l.contains("built-in settings")), "{log:?}");
    assert_eq!(std::fs::read_to_string(dir.join("progression.toml")).unwrap(), broken);
}

#[test]
fn perks_of_an_older_layout_are_refunded_once() {
    let dir = tmp("relayout");
    std::fs::create_dir_all(dir.join("players")).unwrap();
    std::fs::write(
        dir.join("players").join(format!("{U:032x}.json")),
        r#"{"name":"Roman","level":10,"xp":5,"total_xp":3000,"points":1,"perks":{"tester":1,"health":2},"books":{"minecraft:sharpness":1}}"#,
    )
    .unwrap();
    let (mut e, _) = Engine::open(&dir);
    let out = e.join(U, "Roman");
    let d = e.data(U).unwrap().clone();
    assert_eq!((d.level, d.points, d.perks.len(), d.books.len()), (10, e.config.points_by(10), 0, 0));
    assert!(out.iter().any(|x| matches!(x, Effect::Tell(_, t, _) if t.contains("повернуто"))));
    // A perk bought now stays after the next join.
    e.buy(U, "health", &me(0));
    e.leave(U);
    e.join(U, "Roman");
    assert_eq!(e.tier(U, "health"), 1);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_alchemist_makes_good_effects_last_longer() {
    assert_eq!(alchemy(0, "minecraft:speed", 0, 3600), None);
    assert_eq!(alchemy(1, "minecraft:speed", 0, 3600), Some((0, 5400)));
    assert_eq!(alchemy(2, "minecraft:regeneration", 1, 900), Some((1, 1800)));
    assert_eq!(alchemy(3, "minecraft:strength", 1, 3600), Some((2, 7200)));
    // Harmful ones aren't touched, and a strength IV stays IV.
    assert_eq!(alchemy(3, "minecraft:poison", 0, 100), None);
    assert_eq!(alchemy(3, "minecraft:strength", 3, 100), Some((3, 200)));
}

#[test]
fn a_cut_fells_the_logs_above_and_beside_but_not_the_stump() {
    // A trunk from y 64 to 70 with a branch, and a log under the cut.
    let mut logs: Vec<[i32; 3]> = (60..=70).map(|y| [0, y, 0]).collect();
    logs.extend([[1, 68, 0], [2, 69, 1], [30, 66, 0]]);
    let got = fell_tree([0, 64, 0], 64, |p| logs.contains(&p));
    assert_eq!(got.len(), 6 + 2);
    assert!(!got.contains(&[0, 63, 0]) && !got.contains(&[30, 66, 0]));
    assert_eq!(fell_tree([0, 64, 0], 3, |p| logs.contains(&p)).len(), 3);
    assert!(is_log_block("minecraft:oak_log") && is_log_block("minecraft:crimson_stem"));
    assert!(!is_log_block("minecraft:stripped_oak_log") && !is_log_block("minecraft:oak_planks"));
}

#[test]
fn rescues_wait_out_a_cooldown_that_tiers_shorten() {
    let (mut e, dir) = engine("rescue");
    level(&mut e, 30);
    assert!(!e.rescue(U, "void_walker", 0));
    e.buy(U, "void_walker", &me(0));
    assert!(e.rescue(U, "void_walker", 100));
    assert!(!e.rescue(U, "void_walker", 100 + 600 * 20 - 1));
    // Tier II: five minutes from the next rescue on.
    e.buy(U, "void_walker", &me(0));
    let at = 100 + 600 * 20;
    assert!(e.rescue(U, "void_walker", at));
    assert!(!e.rescue(U, "void_walker", at + 300 * 20 - 1));
    assert!(e.rescue(U, "void_walker", at + 300 * 20));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn clear_mind_takes_more_with_each_tier() {
    let (mut e, dir) = engine("clear");
    level(&mut e, 32);
    assert!(e.cleared(U).is_empty());
    e.buy(U, "clear_mind", &me(0));
    assert_eq!(e.cleared(U).len(), 3);
    e.buy(U, "clear_mind", &me(0));
    e.buy(U, "clear_mind", &me(0));
    assert!(e.cleared(U).contains(&"minecraft:wither"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_lab_zone_follows_its_menu() {
    let (mut e, dir) = engine("lab");
    level(&mut e, 30);
    assert!(e.action(U, "lab", 0, &me(0)).open.is_none());
    e.buy(U, "lab", &me(0));
    assert_eq!(e.action(U, "lab", 0, &me(0)).open, Some(Page::Lab));
    assert!(e.zones().is_empty());
    e.click(U, Page::Lab, LAB_HERE, 0, &me(0));
    let z = e.zones()[0].clone();
    assert_eq!((z.center, z.radius, z.no_blast, z.freeze), ([100, 64, -21], 8, true, false));
    assert!(z.contains("minecraft:overworld", [108, 56, -13]) && !z.contains("minecraft:overworld", [109, 64, -21]));
    assert!(!z.contains("minecraft:the_nether", [100, 64, -21]));
    e.click(U, Page::Lab, LAB_FREEZE, 0, &me(0));
    e.click(U, Page::Lab, LAB_BLAST, 0, &me(0));
    let z = e.zones()[0].clone();
    assert!(z.freeze && !z.no_blast);
    // The next tier widens it in place; a respec turns it off.
    e.buy(U, "lab", &me(0));
    assert_eq!(e.zones()[0].radius, 16);
    e.respec(U, &me(100));
    assert!(e.zones().is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_map_lists_each_branch_and_its_perks() {
    let (mut e, dir) = engine("map");
    level(&mut e, 1);
    e.buy(U, "health", &me(0));
    let map = e.tree(U, true);
    let lines: Vec<Vec<&str>> = map.lines().map(|l| l.split('\t').collect()).collect();
    assert_eq!(lines[0][..4], ["T", "Прогресія · рівень 1", "2", "1"]);
    let nodes: Vec<&Vec<&str>> = lines.iter().filter(|l| l[0] == "N").collect();
    assert_eq!(nodes.len(), BRANCHES.len() + e.config.perks.len());
    assert!(nodes.iter().all(|n| n.len() == 13));
    let state = |id: &str| nodes.iter().find(|n| n[1] == id).unwrap()[8];
    assert_eq!(state("@survival"), "branch");
    assert_eq!(state("health"), "owned");
    assert_eq!(state("pickaxe3"), "locked");
    // dash_up hangs from dash_forward; the first perk of a branch from the badge.
    assert_eq!(nodes.iter().find(|n| n[1] == "dash_up").unwrap()[2], "dash_forward");
    assert_eq!(nodes.iter().find(|n| n[1] == "health").unwrap()[2], "@survival");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_drawing_has_the_places_of_its_shape() {
    use progression_core_tests::core::engine::draft_shape;
    // A line steps one block at a time along its longest side.
    assert_eq!(draft_shape("line", [0, 0, 0], [3, 1, 0], false, 100), vec![[0, 0, 0], [1, 0, 0], [2, 1, 0], [3, 1, 0]]);
    // A 3×3×3 box: 8 corners and the middle of its 12 edges; filled, all 27.
    assert_eq!(draft_shape("box", [0, 0, 0], [2, 2, 2], false, 100).len(), 20);
    assert_eq!(draft_shape("box", [2, 2, 2], [0, 0, 0], true, 100).len(), 27);
    // A flat box is a rectangle's outline: 4×3 gives 10.
    assert_eq!(draft_shape("box", [0, 5, 0], [3, 5, 2], false, 100).len(), 10);
    // A ring of radius 2 around A, in A's layer, with B anywhere at that distance.
    let ring = draft_shape("circle", [10, 64, 10], [12, 70, 10], false, 100);
    assert!(ring.iter().all(|p| p[1] == 64));
    assert!(ring.contains(&[12, 64, 10]) && ring.contains(&[10, 64, 8]) && !ring.contains(&[10, 64, 10]));
    let disc = draft_shape("circle", [10, 64, 10], [12, 64, 10], true, 100);
    assert!(disc.contains(&[10, 64, 10]) && disc.len() > ring.len());
    // Past the limit it stops one over, so the caller knows.
    assert_eq!(draft_shape("box", [0, 0, 0], [99, 99, 99], true, 256).len(), 257);
}

#[test]
fn the_mason_turns_a_stone_into_the_next_of_its_family() {
    use progression_core_tests::core::engine::mason_next;
    let all = |_: &str| true;
    assert_eq!(mason_next("minecraft:stone_bricks", all).as_deref(), Some("minecraft:cracked_stone_bricks"));
    // The last of a family goes back to the first.
    assert_eq!(mason_next("minecraft:chiseled_stone_bricks", all).as_deref(), Some("minecraft:stone"));
    // Stairs stay stairs, facing the same way; ones the game lacks are skipped.
    let known = ["minecraft:stone_stairs", "minecraft:stone_brick_stairs", "minecraft:mossy_stone_brick_stairs"];
    let has = |id: &str| known.contains(&id);
    assert_eq!(
        mason_next("minecraft:stone_brick_stairs[facing=east,half=top,shape=straight,waterlogged=false]", has).as_deref(),
        Some("minecraft:mossy_stone_brick_stairs[facing=east,half=top,shape=straight,waterlogged=false]")
    );
    assert_eq!(mason_next("minecraft:mossy_stone_brick_stairs[facing=east]", has).as_deref(), Some("minecraft:stone_stairs[facing=east]"));
    // A pillar's axis doesn't carry to bricks; it does to another pillar.
    assert_eq!(mason_next("minecraft:quartz_pillar[axis=x]", all).as_deref(), Some("minecraft:quartz_bricks"));
    assert_eq!(mason_next("minecraft:basalt[axis=z]", all).as_deref(), Some("minecraft:polished_basalt[axis=z]"));
    // Not a stone of any family: nothing.
    assert_eq!(mason_next("minecraft:oak_planks", all), None);
}

#[test]
fn the_trowel_swaps_only_blocks_of_the_same_shape() {
    use progression_core_tests::core::engine::trowel_state;
    assert_eq!(trowel_state("minecraft:stone", "minecraft:andesite").as_deref(), Some("minecraft:andesite"));
    assert_eq!(
        trowel_state("minecraft:oak_stairs[facing=north,half=bottom]", "minecraft:spruce_stairs").as_deref(),
        Some("minecraft:spruce_stairs[facing=north,half=bottom]")
    );
    assert_eq!(trowel_state("minecraft:oak_log[axis=x]", "minecraft:birch_log").as_deref(), Some("minecraft:birch_log[axis=x]"));
    assert_eq!(trowel_state("minecraft:oak_log[axis=x]", "minecraft:stone").as_deref(), Some("minecraft:stone"));
    assert_eq!(trowel_state("minecraft:oak_stairs[facing=north]", "minecraft:stone"), None);
    assert_eq!(trowel_state("minecraft:stone", "minecraft:stone"), None);
}

#[test]
fn the_draftsman_puts_two_points_and_fills_from_the_second_tier() {
    let (mut e, dir) = engine("draft");
    level(&mut e, 40);
    // Not learned: the stick does nothing.
    assert!(e.draft_point(U, "minecraft:overworld", [0, 64, 0]).is_empty());
    e.buy(U, "draft", &me(0));
    e.draft_point(U, "minecraft:overworld", [0, 64, 0]);
    e.draft_point(U, "minecraft:overworld", [4, 64, 0]);
    let shown = e.drafts();
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].2.len(), 5);
    // Tier I draws but doesn't fill.
    let a = e.click(U, Page::Draft, DRAFT_FILL, 0, &me(0));
    assert!(!a.effects.iter().any(|x| matches!(x, Effect::Fill { .. })));
    e.buy(U, "draft", &me(0));
    e.click(U, Page::Draft, DRAFT_BOX, 0, &me(0));
    let a = e.click(U, Page::Draft, DRAFT_FILL, 0, &me(0));
    assert!(a.effects.iter().any(|x| matches!(x, Effect::Fill { blocks, .. } if blocks.len() == 5)));
    // A third point starts a new drawing.
    e.draft_point(U, "minecraft:overworld", [9, 9, 9]);
    assert_eq!(e.data(U).unwrap().draft.as_ref().map(|d| (d.a, d.b)), Some((Some([9, 9, 9]), None)));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_fill_goes_chunk_by_chunk_from_the_top_in_rows() {
    use progression_core_tests::core::engine::{fill_order, runs};
    let mut blocks = vec![[17, 60, 0], [0, 60, 0], [1, 61, 0], [0, 61, 0], [2, 61, 0], [15, 60, 0]];
    fill_order(&mut blocks);
    // The first chunk (x 0..16) top layer first, then its lower one, then the next chunk.
    assert_eq!(blocks, vec![[0, 61, 0], [1, 61, 0], [2, 61, 0], [0, 60, 0], [15, 60, 0], [17, 60, 0]]);
    // A row along x is one run; a gap starts another.
    assert_eq!(runs(&blocks), vec![([0, 61, 0], [2, 61, 0]), ([0, 60, 0], [0, 60, 0]), ([15, 60, 0], [15, 60, 0]), ([17, 60, 0], [17, 60, 0])]);
}
