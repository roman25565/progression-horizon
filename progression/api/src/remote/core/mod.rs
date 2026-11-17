//! The mod's rules, pure Rust: `core-tests/` runs them without the game.
pub mod config;
pub mod coords;
pub mod menu;
pub mod store;

/// A block position.
pub type Pos = [i32; 3];
/// A level: 0 the Overworld, 1 the Nether, 2 the End.
pub type Dim = u8;
/// A transmitter: its level and position.
pub type TxKey = (Dim, Pos);

pub const DIMENSIONS: [&str; 3] = ["minecraft:overworld", "minecraft:the_nether", "minecraft:the_end"];

pub fn dim_code(name: &str) -> Option<Dim> {
    DIMENSIONS.iter().position(|d| *d == name).map(|i| i as Dim)
}

pub fn dim_name(dim: Dim) -> &'static str {
    DIMENSIONS.get(usize::from(dim)).copied().unwrap_or(DIMENSIONS[0])
}

/// The block a face points to: `pos` one step along `normal`.
pub fn step(pos: Pos, normal: Pos) -> Pos {
    [pos[0] + normal[0], pos[1] + normal[1], pos[2] + normal[2]]
}
