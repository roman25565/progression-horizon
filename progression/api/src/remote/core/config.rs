//! The server's settings, `config/remoteredstone/remoteredstone.toml`.

use serde::Deserialize;
use std::path::Path;

/// The settings file the mod writes on the first start.
pub const DEFAULT_CONFIG: &str = include_str!("../../../data/remoteredstone.toml");

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// The block a transmitter is (`minecraft:redstone_lamp`).
    pub block: String,
    pub max_targets: usize,
    /// Per axis, in blocks; 0 for no limit.
    pub max_distance: i32,
    pub recipe: bool,
    pub owner_only: bool,
    pub link_seconds: u32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            block: "minecraft:redstone_lamp".into(),
            max_targets: 64,
            max_distance: 0,
            recipe: true,
            owner_only: true,
            link_seconds: 120,
        }
    }
}

impl Config {
    /// Reads the settings and puts out-of-range values back in range.
    pub fn parse(text: &str) -> Result<Config, String> {
        let mut c: Config = toml::from_str(text).map_err(|e| e.to_string())?;
        let block = c.block.trim().to_ascii_lowercase();
        c.block = if block.is_empty() {
            Config::default().block
        } else if block.contains(':') {
            block
        } else {
            format!("minecraft:{block}")
        };
        c.max_targets = c.max_targets.clamp(1, 1024);
        c.max_distance = c.max_distance.max(0);
        c.link_seconds = c.link_seconds.clamp(5, 3600);
        Ok(c)
    }

    /// The settings in `dir` (written with the defaults when missing). A broken file gives the
    /// defaults and the reason, for the log; the file is left for the owner to fix.
    pub fn load(dir: &Path) -> (Config, Option<String>) {
        let path = dir.join("remoteredstone.toml");
        match std::fs::read_to_string(&path) {
            Ok(text) => match Config::parse(&text) {
                Ok(c) => (c, None),
                Err(e) => (Config::default(), Some(format!("{}: {e}; using the defaults", path.display()))),
            },
            Err(_) => {
                let _ = std::fs::create_dir_all(dir);
                let warning = std::fs::write(&path, DEFAULT_CONFIG)
                    .err()
                    .map(|e| format!("{}: {e}", path.display()));
                (Config::parse(DEFAULT_CONFIG).unwrap_or_default(), warning)
            }
        }
    }

    /// Whether `target` is close enough to a transmitter at `origin`.
    pub fn in_reach(&self, origin: [i32; 3], target: [i32; 3]) -> bool {
        self.max_distance == 0
            || (0..3).all(|i| (i64::from(origin[i]) - i64::from(target[i])).abs() <= i64::from(self.max_distance))
    }
}
