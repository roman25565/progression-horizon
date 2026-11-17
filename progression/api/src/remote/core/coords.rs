//! Coordinates as players type them, and as the transmitter item keeps them.

use super::Pos;

/// At most this many positions in one command or dialog.
pub const MAX_PER_LINE: usize = 64;

/// Positions from text: numbers in threes, separated by spaces, commas or `;`
/// (`10 64 -3; 12 64 -3`). `~` and `~N` are relative to `origin` (the transmitter), as in
/// vanilla commands; decimals are floored (`10.5` is block 10), so F3's numbers work too.
pub fn parse(text: &str, origin: Pos) -> Result<Vec<Pos>, String> {
    let mut numbers = Vec::new();
    for token in text.split(|c: char| c.is_whitespace() || c == ',' || c == ';').filter(|t| !t.is_empty()) {
        let axis = numbers.len() % 3;
        numbers.push(number(token, origin[axis]).ok_or_else(|| format!("«{token}» не схоже на координату"))?);
    }
    if numbers.is_empty() {
        return Err("Вкажи координати: x y z (можна ~ відносно передавача)".into());
    }
    if numbers.len() % 3 != 0 {
        return Err(format!("Координат має бути по три (x y z), а їх {}", numbers.len()));
    }
    if numbers.len() / 3 > MAX_PER_LINE {
        return Err(format!("Не більше {MAX_PER_LINE} блоків за раз"));
    }
    Ok(numbers.chunks(3).map(|c| [c[0], c[1], c[2]]).collect())
}

fn number(token: &str, origin: i32) -> Option<i32> {
    let (relative, rest) = match token.strip_prefix('~') {
        Some(rest) => (true, rest),
        None => (false, token),
    };
    let value = if rest.is_empty() {
        if !relative {
            return None;
        }
        0.0
    } else {
        let v: f64 = rest.parse().ok()?;
        if !v.is_finite() || v.abs() > 30_000_000.0 {
            return None;
        }
        v
    };
    let base = if relative { f64::from(origin) } else { 0.0 };
    let n = (base + value).floor();
    (n.abs() <= 30_000_000.0).then_some(n as i32)
}

/// `x y z;x y z`: how the item keeps its targets.
pub fn encode(targets: &[Pos]) -> String {
    targets.iter().map(|p| format!("{} {} {}", p[0], p[1], p[2])).collect::<Vec<_>>().join(";")
}

/// The item's targets; whatever doesn't read is left out.
pub fn decode(text: &str) -> Vec<Pos> {
    text.split(';')
        .filter_map(|part| {
            let n: Vec<i32> = part.split_whitespace().filter_map(|t| t.parse().ok()).collect();
            (n.len() == 3).then(|| [n[0], n[1], n[2]])
        })
        .collect()
}

/// `10 64 -3`, as shown to players.
pub fn show(p: Pos) -> String {
    format!("{} {} {}", p[0], p[1], p[2])
}
