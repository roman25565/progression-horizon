//! What the menus look like. With the mod's resource pack a menu is a chest whose title starts
//! with glyphs of the font `progression:gui`: the whole window's picture, a frame around each
//! perk's slot in the colour of its state, and the experience bar. The pack's font file
//! (`tools/make_pack.py`) and this file share the code points and positions below.

use super::engine::Page;

/// The font the glyphs are in.
pub const FONT: &str = "progression:gui";

/// The frame around a slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frame {
    None,
    /// The player's level is too low.
    Locked,
    /// Not enough points.
    Poor,
    /// Can be bought now.
    Buy,
    /// Some tiers bought, the next one not affordable yet.
    Owned,
    /// Every tier bought.
    Max,
}

impl Frame {
    fn index(self) -> Option<u32> {
        match self {
            Frame::None => None,
            Frame::Locked => Some(0),
            Frame::Poor => Some(1),
            Frame::Buy => Some(2),
            Frame::Owned => Some(3),
            Frame::Max => Some(4),
        }
    }
}

/// One slot of a menu.
#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub icon: String,
    pub count: u32,
    pub name: String,
    pub color: &'static str,
    pub lore: Vec<(String, &'static str)>,
    pub glint: bool,
    pub frame: Frame,
}

impl Cell {
    pub fn plain(icon: &str, name: &str, color: &'static str, lore: Vec<(String, &'static str)>) -> Cell {
        Cell { icon: icon.to_owned(), count: 1, name: name.to_owned(), color, lore, glint: false, frame: Frame::None }
    }
    /// A pane for the slots nothing is in, for clients without the pack.
    pub fn filler() -> Cell {
        Cell::plain("minecraft:gray_stained_glass_pane", " ", "gray", Vec::new())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MenuView {
    pub rows: u8,
    /// The title's first part, in [`FONT`] (empty without the pack).
    pub glyphs: String,
    /// The title's readable part.
    pub label: String,
    pub cells: Vec<Option<Cell>>,
}

/// A branch's name, badge and what it is for.
pub fn branch(id: &str) -> (&'static str, &'static str, &'static str) {
    match id {
        "survival" => ("Виживання", "minecraft:totem_of_undying", "здоров'я, падіння, вода, порожнеча, зілля, міцна голова"),
        "pvp" => ("PvP", "minecraft:diamond_sword", "броня, стійкість, ривки, остання стійка, книги зброї"),
        "pve" => ("PvE", "minecraft:skeleton_skull", "мисливець, трофеї, досвід, бачення в лаві, книги броні"),
        "build" => ("Будівництво", "minecraft:bricks", "руки, легкий крок, 3×3, лісоруб, кельма, каменяр, книги інструментів"),
        "cart" => ("Вагонетка", "minecraft:minecart", "найшвидша дорога: швидкі колії, розгін, кондуктор"),
        "horse" => ("Кінь", "minecraft:saddle", "скакун, спуск, прохідливість, поклик, безсмертя, сумки"),
        "elytra" => ("Елітри", "minecraft:elytra", "легкі крила, ракетник, м'яка посадка, планер"),
        _ => ("Редстоун", "minecraft:redstone", "лабораторія, дух, якір чанків, кресляр, дистанційний сигнал"),
    }
}

/// `minecraft:amethyst_block` -> `amethyst_block`.
pub fn short(id: &str) -> &str {
    id.split_once(':').map_or(id, |(_, p)| p)
}

// Code points, shared with tools/make_pack.py.
const NEG: u32 = 0xF800; // NEG + n: back 2^(n-1) pixels, n = 1..=8
const POS: u32 = 0xF810; // POS + n: forward 2^(n-1) pixels
const BACKGROUND: u32 = 0xE000; // + page
const FRAME: u32 = 0xE100; // + state * 8 + row
const BAR: u32 = 0xE200; // + n: a piece of the bar 2^n pixels wide, n = 0..=6

/// Where the client starts a container's title, and where the experience bar is.
const TITLE_X: i32 = 8;
pub const BAR_X: i32 = 108;
pub const BAR_WIDTH: i32 = 60;
const WINDOW: i32 = 176;

fn ch(code: u32) -> char {
    char::from_u32(code).unwrap_or(' ')
}

/// Moves the pen by `dx` pixels with the font's space glyphs.
fn shift(out: &mut String, dx: i32) {
    let base = if dx < 0 { NEG } else { POS };
    let mut left = dx.unsigned_abs();
    for n in (1..=8u32).rev() {
        let step = 1 << (n - 1);
        while left >= step {
            out.push(ch(base + n));
            left -= step;
        }
    }
}

/// The glyph part of a menu's title: the page's picture, the frames of `frames` (slot, state)
/// and the experience bar filled to `bar` (0 to 1). The pen ends where the title started, so
/// the readable part follows as usual.
pub fn glyphs(page: Page, frames: &[(usize, Frame)], bar: Option<f32>) -> String {
    let mut out = String::new();
    // A bitmap glyph moves the pen by its width and one more pixel.
    let mut pen = TITLE_X;
    let mut draw = |out: &mut String, x: i32, code: u32, width: i32| {
        shift(out, x - pen);
        out.push(ch(code));
        pen = x + width + 1;
    };
    let index = match page {
        Page::Perks | Page::Travel => 0,
        Page::Altar => 1,
        // The lab is a three-row window like the respec one.
        Page::Respec | Page::Lab | Page::Draft => 2,
    };
    draw(&mut out, 0, BACKGROUND + index, WINDOW);
    for (slot, frame) in frames {
        let (row, col) = ((slot / 9) as u32, (slot % 9) as i32);
        if let Some(state) = frame.index()
            && row < 6
        {
            draw(&mut out, 7 + col * 18, FRAME + state * 8 + row, 18);
        }
    }
    if let Some(bar) = bar {
        let mut fill = (bar.clamp(0.0, 1.0) * BAR_WIDTH as f32).round() as i32;
        let mut x = BAR_X;
        for n in (0..=6u32).rev() {
            let piece = 1 << n;
            while fill >= piece {
                draw(&mut out, x, BAR + n, piece);
                x += piece;
                fill -= piece;
            }
        }
    }
    shift(&mut out, TITLE_X - pen);
    out
}

/// The pixels a glyph string moves the pen by, as the pack's font defines them (for tests).
pub fn advance(glyphs: &str) -> i32 {
    glyphs
        .chars()
        .map(|c| {
            let c = c as u32;
            match c {
                c if (NEG + 1..=NEG + 8).contains(&c) => -(1 << (c - NEG - 1)),
                c if (POS + 1..=POS + 8).contains(&c) => 1 << (c - POS - 1),
                c if (BACKGROUND..BACKGROUND + 8).contains(&c) => WINDOW + 1,
                c if (FRAME..FRAME + 64).contains(&c) => 19,
                c if (BAR..=BAR + 6).contains(&c) => (1 << (c - BAR)) + 1,
                _ => 0,
            }
        })
        .sum()
}
