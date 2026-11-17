#!/usr/bin/env python3
"""Draws the Progression resource pack: the menus' pictures as glyphs of the font
`progression:gui`, which the mod puts in each menu's title (api/src/core/gui.rs has the same code
points and positions).

  python tools/make_pack.py            # writes resourcepack/ and progression/api/data/progression.zip
  python tools/make_pack.py --preview  # also preview-*.png: the menus as a client draws them

Needs Pillow. Replace any PNG in resourcepack/assets/progression/textures/font/ with your own
art of the same size and run again with --zip-only to rebuild the zip from the folder.
"""
import argparse, json, zipfile
from pathlib import Path
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
PACK = ROOT / "resourcepack"
TEX = PACK / "assets/progression/textures/font"
ZIP = ROOT / "progression/api/data/progression.zip"
FORMAT = 97  # Minecraft 26.3

# Shared with gui.rs.
NEG, POS, BACKGROUND, FRAME, BAR = 0xF800, 0xF810, 0xE000, 0xE100, 0xE200
BAR_X, BAR_Y, BAR_W, BAR_H = 108, 8, 60, 5
TITLE_BASELINE = 13  # a glyph with this ascent has its top at the window's top

INK, PANEL, PANEL2, GOLD, GOLD_D = (12, 14, 24), (27, 31, 46), (36, 41, 61), (201, 164, 74), (122, 92, 34)
WELL, WELL_L, PAPER = (15, 17, 27), (58, 64, 92), (198, 186, 160)
BRANCH = [(64, 150, 92), (176, 58, 58), (122, 84, 176), (196, 136, 52), (190, 44, 72)]
STATES = {  # frame colours: locked, poor, buy, owned, max
    0: ((70, 74, 90), (40, 43, 56)),
    1: ((214, 170, 60), (110, 84, 24)),
    2: ((92, 220, 110), (30, 110, 48)),
    3: ((86, 170, 232), (30, 78, 130)),
    4: ((255, 214, 92), (176, 120, 20)),
}


def mix(a, b, t):
    return tuple(round(x + (y - x) * t) for x, y in zip(a, b))


def well(d, x, y, tint=None):
    """An 18x18 slot: dark inside, a lighter lower right edge."""
    d.rectangle([x, y, x + 17, y + 17], fill=mix(WELL, tint, 0.18) if tint else WELL)
    d.line([x, y, x + 17, y], fill=INK)
    d.line([x, y, x, y + 17], fill=INK)
    edge = mix(WELL_L, tint, 0.35) if tint else WELL_L
    d.line([x + 17, y + 1, x + 17, y + 17], fill=edge)
    d.line([x + 1, y + 17, x + 17, y + 17], fill=edge)


def window(rows):
    """The window every menu shares: border, title strip, the player's inventory."""
    h = 114 + rows * 18
    im = Image.new("RGBA", (176, h), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    d.rounded_rectangle([0, 0, 175, h - 1], radius=4, fill=PANEL, outline=INK)
    d.rounded_rectangle([1, 1, 174, h - 2], radius=3, outline=GOLD)
    d.rounded_rectangle([2, 2, 173, h - 3], radius=2, outline=GOLD_D)
    # The title strip, a shade lighter, with a gold line under it.
    d.rectangle([3, 3, 172, 15], fill=PANEL2)
    d.line([3, 16, 172, 16], fill=GOLD_D)
    # The vanilla "Inventory" label is dark grey: a paper strip under it.
    top = 17 + rows * 18
    d.line([3, top + 1, 172, top + 1], fill=GOLD_D)
    d.rounded_rectangle([6, top + 3, 62, top + 12], radius=2, fill=PAPER, outline=GOLD_D)
    for r in range(3):
        for c in range(9):
            well(d, 7 + c * 18, top + 13 + r * 18)
    for c in range(9):
        well(d, 7 + c * 18, top + 71)
    # The last column and row must not be transparent: the client cuts a glyph at its last
    # coloured column, and the mod counts 176 pixels.
    im.putpixel((175, h // 2), INK + (255,))
    return im, d


def perks():
    im, d = window(6)
    for row, tint in enumerate(BRANCH):
        y = 17 + row * 18
        d.rectangle([3, y, 172, y + 17], fill=mix(PANEL, tint, 0.22))
        # The branch's badge: a brighter slot and a bar beside it.
        d.rectangle([7, y, 24, y + 17], fill=mix(PANEL, tint, 0.55))
        d.rectangle([7, y, 24, y + 17], outline=mix(tint, (255, 255, 255), 0.25))
        d.line([25, y + 2, 25, y + 15], fill=mix(tint, (255, 255, 255), 0.25))
        for c in range(1, 9):
            well(d, 7 + c * 18, y, tint)
        if row:
            d.line([3, y, 172, y], fill=INK)
    y = 17 + 5 * 18
    d.line([3, y, 172, y], fill=GOLD_D)
    for c in (0, 1, 4, 7, 8):
        well(d, 7 + c * 18, y)
        d.rectangle([7 + c * 18, y, 24 + c * 18, y + 17], outline=GOLD_D)
    # The experience bar's trough.
    d.rectangle([BAR_X - 1, BAR_Y - 1, BAR_X + BAR_W, BAR_Y + BAR_H], fill=INK, outline=GOLD_D)
    return im


def altar():
    im, d = window(6)
    purple = (122, 84, 176)
    d.rectangle([3, 17, 172, 17 + 108 - 1], fill=mix(PANEL, purple, 0.16))
    well(d, 7 + 4 * 18, 17, purple)
    d.rectangle([7 + 4 * 18, 17, 24 + 4 * 18, 34], outline=GOLD)
    for i in range(9, 45):
        well(d, 7 + (i % 9) * 18, 17 + (i // 9) * 18, purple)
    # Little stars in the top row, as an enchanting table's letters.
    for x, y in [(14, 22), (30, 29), (48, 21), (62, 30), (110, 23), (126, 30), (142, 21), (158, 28)]:
        d.point((x, y), fill=(214, 190, 255))
        d.point((x + 1, y + 1), fill=(150, 120, 210))
    well(d, 7 + 8 * 18, 17 + 5 * 18)
    d.rectangle([7 + 8 * 18, 107, 24 + 8 * 18, 124], outline=GOLD_D)
    return im


def respec():
    im, d = window(3)
    for c, tint in ((2, (64, 170, 92)), (6, (190, 60, 60))):
        x, y = 7 + c * 18, 17 + 18
        d.rounded_rectangle([x - 12, y - 6, x + 29, y + 23], radius=3, fill=mix(PANEL, tint, 0.35), outline=tint)
        well(d, x, y, tint)
    return im


def frame(state):
    light, dark = STATES[state]
    im = Image.new("RGBA", (18, 18), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    d.rectangle([0, 0, 17, 17], outline=dark)
    d.rectangle([1, 1, 16, 16], outline=light)
    if state == 0:  # locked: a dim veil with a keyhole's dot
        d.rectangle([2, 2, 15, 15], fill=(8, 9, 14, 150))
    if state == 4:  # learned whole: bright corners
        for x, y in ((0, 0), (17, 0), (0, 17), (17, 17)):
            im.putpixel((x, y), (255, 244, 190, 255))
    if state == 2:  # can be bought: a soft glow inside
        d.rectangle([2, 2, 15, 15], outline=light + (90,))
    return im


def bar(width):
    im = Image.new("RGBA", (width, BAR_H), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    d.rectangle([0, 0, width - 1, BAR_H - 1], fill=(92, 214, 110))
    d.line([0, 0, width - 1, 0], fill=(170, 246, 170))
    d.line([0, BAR_H - 1, width - 1, BAR_H - 1], fill=(40, 130, 60))
    return im


def build():
    TEX.mkdir(parents=True, exist_ok=True)
    providers = [{"type": "space", "advances": {
        **{chr(NEG + n): -(1 << (n - 1)) for n in range(1, 9)},
        **{chr(POS + n): 1 << (n - 1) for n in range(1, 9)}}}]

    def glyph(name, image, code, ascent):
        image.save(TEX / f"{name}.png")
        providers.append({"type": "bitmap", "file": f"progression:font/{name}.png",
                          "ascent": ascent, "height": image.height, "chars": [chr(code)]})

    for i, (name, image) in enumerate((("perks", perks()), ("altar", altar()), ("respec", respec()))):
        glyph(name, image, BACKGROUND + i, TITLE_BASELINE)
    for state in STATES:
        image = frame(state)
        image.save(TEX / f"frame_{state}.png")
        for row in range(6):
            providers.append({"type": "bitmap", "file": f"progression:font/frame_{state}.png",
                              "ascent": TITLE_BASELINE - (17 + row * 18), "height": 18,
                              "chars": [chr(FRAME + state * 8 + row)]})
    for n in range(7):
        glyph(f"bar_{1 << n}", bar(1 << n), BAR + n, TITLE_BASELINE - BAR_Y)
    font = PACK / "assets/progression/font"
    font.mkdir(parents=True, exist_ok=True)
    (font / "gui.json").write_text(json.dumps({"providers": providers}, indent=1), encoding="utf-8")
    (PACK / "pack.mcmeta").write_text(json.dumps({"pack": {
        "description": "Progression: menus",
        "pack_format": FORMAT, "min_format": [FORMAT, 0], "max_format": [FORMAT, 99],
    }}, indent=2), encoding="utf-8")
    perks().resize((64, 80)).crop((0, 8, 64, 72)).save(PACK / "pack.png")


def pack_zip():
    ZIP.parent.mkdir(parents=True, exist_ok=True)
    files = sorted(p for p in PACK.rglob("*") if p.is_file())
    with zipfile.ZipFile(ZIP, "w", zipfile.ZIP_DEFLATED) as z:
        for p in files:
            # A fixed date, so the same pack is the same zip (and the same hash for clients).
            info = zipfile.ZipInfo(p.relative_to(PACK).as_posix(), (2026, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            z.writestr(info, p.read_bytes())
    print(f"{len(files)} files, {ZIP.stat().st_size} bytes: {ZIP}")


def preview():
    """The menus as a client draws them, three times the size, with sample frames."""
    def show(name, rows, frames, fill):
        im = Image.open(TEX / f"{name}.png").convert("RGBA")
        for slot, state in frames:
            f = Image.open(TEX / f"frame_{state}.png")
            im.alpha_composite(f, (7 + (slot % 9) * 18, 17 + (slot // 9) * 18))
        if fill is not None:
            im.alpha_composite(bar(max(1, round(BAR_W * fill))), (BAR_X, BAR_Y))
        out = Image.new("RGBA", (176 + 24, im.height + 24), (58, 62, 74, 255))
        out.alpha_composite(im, (12, 12))
        out.resize((out.width * 3, out.height * 3), Image.NEAREST).save(ROOT / f"preview-{name}.png")
    show("perks", 6, [(1, 4), (2, 3), (3, 2), (4, 1), (5, 0), (10, 3), (11, 2), (12, 0), (13, 0), (14, 0), (19, 4), (20, 2),
                      (21, 0), (22, 0), (28, 3), (29, 4), (30, 2), (31, 1), (32, 0), (33, 0), (37, 2), (38, 0), (39, 0)], 0.62)
    show("altar", 6, [(9, 3), (10, 2), (11, 0), (12, 4), (13, 1)] + [(i, 0) for i in range(14, 24)], None)
    show("respec", 3, [], None)


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--preview", action="store_true")
    ap.add_argument("--zip-only", action="store_true", help="zip resourcepack/ as it is (your own art)")
    a = ap.parse_args()
    if not a.zip_only:
        build()
    pack_zip()
    if a.preview:
        preview()
