use serde::{Deserialize, Serialize};

const MIB: u64 = 1024 * 1024;

/// Size and count ceilings applied to every manifest, on both sides. They
/// protect the player from a server (or an impostor) that announces a
/// multi-terabyte pack, and the admin from packaging a whole game by mistake.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    pub max_file_bytes: u64,
    pub max_pack_bytes: u64,
    pub max_files: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_file_bytes: 200 * MIB,
            max_pack_bytes: 2048 * MIB,
            max_files: 5000,
        }
    }
}

impl Limits {
    pub const fn from_mib(max_file_mib: u64, max_pack_mib: u64, max_files: usize) -> Self {
        Self {
            max_file_bytes: max_file_mib * MIB,
            max_pack_bytes: max_pack_mib * MIB,
            max_files,
        }
    }
}

/// Human-readable byte count, for summaries and error messages.
#[allow(clippy::cast_precision_loss)] // display only
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_bytes() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(1023), "1023 B");
        assert_eq!(human_bytes(1024), "1.0 KiB");
        assert_eq!(human_bytes(131_072), "128.0 KiB");
        assert_eq!(human_bytes(3 * 1024 * 1024 * 1024), "3.0 GiB");
    }
}
