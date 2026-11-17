# Progression for Horizon 2.4

A server-side mod. Players earn **progression XP**, XP gives **levels** (not lost on
death), levels give **perk points (PP)**, and PP unlock perks. Each perk becomes available from a
certain level. There are no beacon or potion effects among the perks: every perk gives something of its own that the vanilla
game can't do.

## Where XP comes from

Only from **advancements**, each gives XP once: task 120, goal 360, challenge 600 XP
(dragon and wither more). Vanilla advancements count, and so do advancements from data packs and mods, for example for
discovering a new structure for the first time. Kills, ores, building and anything else that can be repeated forever
give no progression XP, so there is nothing to farm.

50 levels, level n costs 200 + 40·n XP, 340 PP at the maximum, the whole tree costs 339 PP.
Vanilla advancements together give about level 37; the rest comes from structure and mod advancements.

For the weights file to have the right frames (goal, challenge) for data pack and mod advancements, pass them to
the generator: `python tools/gen_datapack.py --jar ... --config ... --extra path\to\datapack_or_mod`
(can be repeated). Without this every such advancement counts as a task.

## Branches

- **Survival**: health; cat landing (safe height and less fall damage); child of the
  water (speed and air underwater); wisdom (more progression XP); clear mind (removes
  bad effects, three levels remove more and more); void walk (falling into the void
  teleports you to the last solid ground with slow falling, with a cooldown); alchemist (good effects last
  1.5 or 2 times longer, and at level three are also stronger); hard head (anvils, stalactites,
  falling sand and gravel don't hurt).
- **PvP**: toughening (armour toughness); steadfast (knockback and explosions); lunge (attack
  reach); dash forward; dash up; last stand (once every 10 min a lethal hit leaves you with
  absorption and regeneration, like a totem); weapon altar.
- **PvE**: hunter (damage to mobs); trophies (chance of an extra drop); scholar (+30%
  vanilla XP per level); lava vision (Horizon client); armour altar.
- **Building**: long arms; light step (step up without jumping, fast sneaking, mud, sand and
  cobwebs slow you less); 3×3 shovel and 3×3 pickaxe (Tool-Kit); lumberjack (a broken log
  fells the whole tree, Shift disables it); trowel (a block placed with Shift becomes a random block
  of the same kind from the hotbar); stonemason (Shift + right-click with an empty hand turns stone into the next
  stonecutter variant, stairs and slabs too); tool altar.
- **Redstone**: laboratory (a zone of your own around the player where explosions don't break blocks and
  observers don't fire while you build); spirit (a free camera near your body); chunk anchor;
  draftsman (Shift + right-click with a stick sets points A and B, right-click with a stick opens a menu: a line,
  rectangle or circle of particles only the owner sees; from level II, filling the outline with blocks
  from the off hand, 256 blocks at a time, 2048 at III); remote signal (transmitters, see below).

The trowel, stonemason, draftsman and hard head are based on mechanics from the Copper Overthrow mod (MIT).

- **Minecart** (the fastest): fast tracks (a minecart makes 2–5 normal rail steps per tick,
  up to 40 m/s, and doesn't fly off on curves); acceleration (W speeds up on any rails, also from a standstill);
  conductor (the minecart returns to your inventory when you get out).
- **Horse**: courser (up to +100% speed); sure descent (the horse falls faster down slopes so it doesn't
  slow down, jumps as before, no fall damage); sure-footed (steps over 2 blocks, faster in
  water and on soul sand); call (the J key calls your horse, even from unloaded chunks of the same
  world); immortal horse; saddlebags (the N key near the horse, 27 or 54 slots, items
  stay even after a respec). "Your horse" is the one you rode last.
- **Elytra** (no speed bonus, vanilla rockets already give ~33 m/s): light wings (elytra
  often don't wear), rocketeer (a firework sometimes comes back), soft landing (hitting a wall doesn't
  hurt), glider (less gravity in flight).

The chest menu has two pages: the main branches and the movement branches (the saddle button). On the
Horizon client the perk map draws each branch as a constellation.

## Remote signal

The former RemoteRedstone mod is now part of progression. A transmitter (a lamp with an Ender pearl and redstone,
crafted in a crafting table) passes its redstone signal to every block in its coordinate list. Only a player who has learned
the perk can craft and place a transmitter and change its targets; operators can do it without the perk.
The distance to targets is unlimited (in the same world); perk levels set the number of targets per
transmitter: I 8, II 32, III 128. The limit comes from the transmitter owner's perk. Transmitters already placed keep working
after a perk respec, they just can't be edited.

Right-clicking a transmitter opens a menu (enter coordinates, pick blocks by clicking, remove targets).
Commands: `/rr add|remove|list|clear|link|done|menu|help`, `/rr give [player]` for operators.
Settings and transmitters stay where they were: `config/remoteredstone/remoteredstone.toml` and
`transmitters.json`. Don't install the separate `remoteredstone` mod together with progression, because
transmitters would be processed twice.

## Enchanting altar

Three altar perks: tools, armour, weapons. Each altar level adds +1 to the maximum level of
books in its group (up to a cap specific to each book). An enchanting table on an amethyst block; right-click
opens the menu. A book costs vanilla levels and can be bought any number of times. The anvil doesn't cut
levels above vanilla and never asks for more than 39 levels.

## Laboratory

The L key (or a button on the map) opens a mini-menu: "Center here" places the zone around the player,
toggles turn on explosion protection and freezing observers, and there is a "Remove zone" button. The zone's
edge grows with the perk level: 17, 33 or 65 blocks. The zone is active while its owner is on
the server.

## Perk map and keys

Horizon client with the `serverkeys` 0.6 mod: K opens the perk map (a starry sky, eight branch constellations, clicking
a perk learns it, buttons for altar, laboratory and respec at the bottom). R dash forward, V dash up,
B spirit, L laboratory, H anchor, J call horse, N saddlebags. All are rebindable in Options > Mods > Server keys. A vanilla
client sees a chest menu from the resource pack; G (Quick Actions) opens the same actions; F and Shift+F are the
dashes. Commands: `/progress`, `/progress respec`, `/progress dash forward|up`,
`/progress spirit|lab|anchor|travel|horse|bags`.

## Menus from the resource pack

The menu is a server-side container, but the resource pack draws it: the container title starts with glyphs
of the `progression:gui` font. The mod serves the pack itself: a built-in zip and a small HTTP server on `pack_port`.
In `config/progression/progression.toml` set `pack_host` to the server's public address and open
the port, or upload the zip elsewhere and set `pack_url`.

Redrawing the textures: `python tools/make_pack.py` (`--preview` renders menu images), then
rebuild the mod.

## Upgrading from 2.2 and 2.3

As before when the settings version changes: the old file is moved aside as `progression-v<N>.toml`,
and players get all their PP back for free.

## Upgrading from 1.x

The old `progression.toml` is moved aside as `progression-v<N>.toml` and a new one is written in its place.
On first join players get all their PP back for free (perks and books are reset); levels
and XP are kept.

## Admin

`/progressadmin xp|level|points|reset|info <player> [number]`, `/progressadmin reload`.

## Building

The mod is in `progression/`, core tests in `core-tests/` (`cargo test`). Build it together with your other
Horizon mods. Keys and the map: the [`serverkeys`](https://github.com/roman25565/serverkeys-horizon) client mod 0.6.

## Credits

The trowel, stonemason, draftsman and hard head perks are based on mechanics from the Copper Overthrow mod (MIT).
The 3×3 tools come from [Kirca / Tool-Kit](https://github.com/roman25565/Tool-Kit).

## License

MIT, see [LICENSE](LICENSE). A mod for [Horizon](https://github.com/horizon-rust/horizon) (MIT, Ludovic Marie and the Horizon contributors).

Unofficial project, not affiliated with Mojang Studios or Microsoft. Minecraft is a trademark of Mojang Synergies AB.
