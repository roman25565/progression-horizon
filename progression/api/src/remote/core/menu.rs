//! The transmitter's menu: a six-row chest, its targets above and the buttons in the last row.

use super::Pos;
use super::coords::show;
use super::store::Transmitter;

pub const ROWS: u8 = 6;
pub const SIZE: usize = 54;
/// Targets on one page: the first five rows.
pub const PER_PAGE: usize = 45;

pub const PREV: usize = 45;
pub const COORDS: usize = 47;
pub const INFO: usize = 48;
pub const LINK: usize = 49;
pub const CLEAR: usize = 50;
pub const NEXT: usize = 53;

pub const TITLE: &str = "Дистанційний редстоун";

#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub icon: String,
    pub name: String,
    /// A text colour name (`red`) for the name.
    pub color: &'static str,
    /// Lines under the name, each with its colour.
    pub lore: Vec<(String, &'static str)>,
    pub count: u32,
    pub glint: bool,
}

impl Cell {
    fn new(icon: &str, name: impl Into<String>, color: &'static str) -> Cell {
        Cell { icon: icon.into(), name: name.into(), color, lore: Vec::new(), count: 1, glint: false }
    }
    fn line(mut self, text: impl Into<String>, color: &'static str) -> Cell {
        self.lore.push((text.into(), color));
        self
    }
}

/// What a click in the menu asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Remove(Pos),
    Page(usize),
    /// The dialog to type coordinates in.
    Coords,
    /// Pick targets by right-clicking them.
    Link,
    Clear,
    Nothing,
}

pub fn pages(t: &Transmitter) -> usize {
    t.targets.len().div_ceil(PER_PAGE).max(1)
}

/// The menu's slots for `page` of transmitter `t` at `pos`; `max` is the settings' limit.
pub fn cells(t: &Transmitter, pos: Pos, page: usize, max: usize) -> Vec<Option<Cell>> {
    let page = page.min(pages(t) - 1);
    let mut out = vec![None; SIZE];
    for (slot, target) in t.targets.iter().skip(page * PER_PAGE).take(PER_PAGE).enumerate() {
        let n = page * PER_PAGE + slot + 1;
        out[slot] = Some(
            Cell::new("minecraft:redstone", show(*target), "red")
                .line(format!("Ціль №{n}"), "gray")
                .line("ПКМ або Shift+клік: прибрати", "dark_gray"),
        );
    }
    if page > 0 {
        out[PREV] = Some(Cell::new("minecraft:arrow", "◀ Попередня сторінка", "yellow"));
    }
    if page + 1 < pages(t) {
        out[NEXT] = Some(Cell::new("minecraft:arrow", "Наступна сторінка ▶", "yellow"));
    }
    out[COORDS] = Some(
        Cell::new("minecraft:writable_book", "Ввести координати", "green")
            .line("x y z, кілька через ;", "gray")
            .line("~ рахує від передавача: ~ ~1 ~", "gray"),
    );
    let mut info = Cell::new(
        if t.signal > 0 { "minecraft:redstone_torch" } else { "minecraft:lever" },
        format!("Сигнал: {}", t.signal),
        if t.signal > 0 { "red" } else { "gray" },
    )
    .line(format!("Передавач: {}", show(pos)), "gray")
    .line(format!("Цілей: {} з {max}", t.targets.len()), "gray")
    .line(format!("Сторінка {} з {}", page + 1, pages(t)), "dark_gray");
    info.count = u32::from(t.signal.max(1));
    info.glint = t.signal > 0;
    out[INFO] = Some(info);
    out[LINK] = Some(
        Cell::new("minecraft:spyglass", "Вибрати блоки кліком", "aqua")
            .line("ПКМ по блоку додає або прибирає його", "gray")
            .line("ПКМ по передавачу: готово", "gray"),
    );
    if !t.targets.is_empty() {
        out[CLEAR] = Some(Cell::new("minecraft:barrier", "Прибрати всі цілі", "red").line("Shift+клік", "gray"));
    }
    out
}

/// What a click asks: `button` 0 left, 1 right; `mode` 0 a plain click, 1 with Shift.
pub fn click(t: &Transmitter, page: usize, slot: i32, button: i32, mode: i32) -> Action {
    let Ok(slot) = usize::try_from(slot) else {
        return Action::Nothing;
    };
    let page = page.min(pages(t) - 1);
    let plain = mode == 0;
    match slot {
        s if s < PER_PAGE => match t.targets.get(page * PER_PAGE + s) {
            Some(p) if (plain && button == 1) || mode == 1 => Action::Remove(*p),
            _ => Action::Nothing,
        },
        PREV if page > 0 && plain => Action::Page(page - 1),
        NEXT if page + 1 < pages(t) && plain => Action::Page(page + 1),
        COORDS if plain => Action::Coords,
        LINK if plain => Action::Link,
        CLEAR if mode == 1 && !t.targets.is_empty() => Action::Clear,
        _ => Action::Nothing,
    }
}
