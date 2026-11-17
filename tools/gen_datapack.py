#!/usr/bin/env python3
"""Makes the advancement rewards for the Progression mod from the vanilla 26.3 jar.

    python gen_datapack.py --jar PATH\\TO\\26.3.jar --config ...\\config\\progression\\progression.toml --out OUTDIR

Writes, in OUTDIR:
  progression-rewards/   a data pack for world/datapacks/: every vanilla advancement that gives
                         points, with "+N досвіду прогресії" in green under its description (the description keeps
                         its translation key, so everyone sees their own language), plus the mod's
                         own advancements (killing the Wither, drying lava with the toolkit).
  advancements.toml      advancement id -> points, for config/progression/.

Points: task 1, goal 3, challenge 5; [advancement_points] in progression.toml wins, as in the mod.
Recipe unlocks and the tabs' roots never give points. Needs Python 3.11 or newer.
"""
import argparse, json, shutil, sys, tomllib, zipfile

# Progression experience for one point of weight ([settings] advancement_xp_per_point).
XP_PER_POINT = 40
from pathlib import Path

FRAME_POINTS = {"task": 1, "goal": 3, "challenge": 5}
PACK_FORMAT = 121  # 26.3

OWN = {
    "progression:boss/kill_wither": {
        "parent": "minecraft:nether/summon_wither",
        "display": {
            "icon": {"id": "minecraft:wither_skeleton_skull"},
            "title": "Кінець Візера",
            "description": "Вбий Візера",
            "frame": "challenge",
        },
        "criteria": {
            "killed": {
                "trigger": "minecraft:player_killed_entity",
                "conditions": {"entity": [{"condition": "minecraft:entity_properties", "entity": "this",
                                           "predicate": {"type": "minecraft:wither"}}]},
            }
        },
    },
    "progression:toolkit/dry_lava": {
        "parent": "minecraft:nether/root",
        "display": {
            "icon": {"id": "minecraft:wet_sponge"},
            "title": "Губка проти лави",
            "description": "Осуши лаву мокрою губкою",
            "frame": "goal",
        },
        # The toolkit (kirca) fires this trigger when a wet sponge dries lava.
        "criteria": {"dried": {"trigger": "kirca:dry_lava"}},
    },
}


def rewardable(adv_id: str) -> bool:
    path = adv_id.split(":", 1)[-1]
    return not (path.startswith("recipes/") or path.endswith("/root") or not path)


def with_points(description, points: int):
    line = {"text": f"\n+{points * XP_PER_POINT} досвіду прогресії", "color": "green"}
    if isinstance(description, str):
        description = {"text": description}
    return ["", description, line]


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--jar", required=True, help="the vanilla 26.3 client or server jar")
    ap.add_argument("--config", help="progression.toml (for [advancement_points] and the default)")
    ap.add_argument("--out", default="out")
    ap.add_argument("--extra", action="append", default=[],
                    help="a datapack or mod (folder, .zip or .jar) whose advancements also count; "
                         "their frame sets the weight (repeatable)")
    a = ap.parse_args()

    overrides, default = {}, 1
    if a.config:
        cfg = tomllib.loads(Path(a.config).read_text(encoding="utf-8"))
        overrides = cfg.get("advancement_points", {})
        default = cfg.get("settings", {}).get("default_advancement_points", 1)
        global XP_PER_POINT
        XP_PER_POINT = cfg.get("settings", {}).get("advancement_xp_per_point", 40)

    out = Path(a.out)
    pack = out / "progression-rewards"
    if pack.exists():
        shutil.rmtree(pack)
    points = {}
    written = 0

    def write(adv_id: str, data: dict):
        nonlocal written
        ns, path = adv_id.split(":", 1)
        f = pack / "data" / ns / "advancement" / f"{path}.json"
        f.parent.mkdir(parents=True, exist_ok=True)
        f.write_text(json.dumps(data, ensure_ascii=False, indent=2), encoding="utf-8")
        written += 1

    jar = zipfile.ZipFile(a.jar)
    # The server jar keeps the real one inside META-INF/versions/.
    inner = [n for n in jar.namelist() if n.startswith("META-INF/versions/") and n.endswith(".jar")]
    if not any(n.startswith("data/minecraft/advancement/") for n in jar.namelist()) and inner:
        import io
        jar = zipfile.ZipFile(io.BytesIO(jar.read(inner[0])))
    prefix = "data/minecraft/advancement/"
    names = sorted(n for n in jar.namelist() if n.startswith(prefix) and n.endswith(".json"))
    if not names:
        print("no advancements in that jar: is it the 26.3 client or server jar?", file=sys.stderr)
        return 1
    for n in names:
        adv_id = "minecraft:" + n[len(prefix):-len(".json")]
        if not rewardable(adv_id):
            continue
        data = json.loads(jar.read(n))
        display = data.get("display")
        if display is None:
            continue
        frame = display.get("frame", "task")
        p = overrides.get(adv_id, FRAME_POINTS.get(frame, default))
        points[adv_id] = p
        if p > 0:
            display["description"] = with_points(display.get("description", ""), p)
            write(adv_id, data)

    # Other datapacks and mods (new structures and the like): weights only, their files stay theirs.
    import re
    pattern = re.compile(r"^(?:.*/)?data/([^/]+)/advancements?/(.+)\.json$")
    def extra_files(src: Path):
        if src.is_dir():
            for f in src.rglob("*"):
                if f.suffix.lower() == ".json":
                    yield f.relative_to(src).as_posix(), f.read_bytes
                elif f.suffix.lower() in (".zip", ".jar") and f.is_file():
                    yield from extra_files(f)
        elif src.suffix.lower() in (".zip", ".jar"):
            try:
                z = zipfile.ZipFile(src)
            except (zipfile.BadZipFile, OSError):
                return
            for n in z.namelist():
                if n.endswith(".json"):
                    yield n, (lambda n=n: z.read(n))
    for src in a.extra:
        for name, read in extra_files(Path(src)):
            m = pattern.match(name)
            if not m:
                continue
            adv_id = f"{m.group(1)}:{m.group(2)}"
            if adv_id in points or not rewardable(adv_id):
                continue
            try:
                display = json.loads(read()).get("display")
            except (ValueError, UnicodeDecodeError):
                continue
            if not display:
                continue
            points[adv_id] = overrides.get(adv_id, FRAME_POINTS.get(display.get("frame", "task"), default))

    for adv_id, data in OWN.items():
        data = json.loads(json.dumps(data))
        frame = data["display"]["frame"]
        p = overrides.get(adv_id, FRAME_POINTS[frame])
        points[adv_id] = p
        if p > 0:
            data["display"]["description"] = with_points(data["display"]["description"], p)
        write(adv_id, data)

    (pack / "pack.mcmeta").write_text(json.dumps({"pack": {
        "description": "Progression: progression experience in advancement descriptions",
        "pack_format": PACK_FORMAT,
        "min_format": [PACK_FORMAT, 0],
        "max_format": [PACK_FORMAT, 0],
    }}, ensure_ascii=False, indent=2), encoding="utf-8")

    lines = ["# Made by tools/gen_datapack.py from the vanilla jar: advancement id -> perk points.",
             "# [advancement_points] in progression.toml wins over this file.", "[points]"]
    lines += [f'"{k}" = {v}' for k, v in sorted(points.items())]
    (out / "advancements.toml").write_text("\n".join(lines) + "\n", encoding="utf-8")
    total = sum(points.values())
    if a.extra:
        print(f"{sum(1 for k in points if not k.startswith(('minecraft:', 'progression:')))} of them from --extra")
    print(f"{len(points)} advancements, {total} ОП in all; {written} files in {pack}")
    print(f"copy {pack.name} into world/datapacks/ and advancements.toml into config/progression/")
    return 0


if __name__ == "__main__":
    sys.exit(main())
