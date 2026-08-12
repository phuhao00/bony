use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct EconomyPaths {
    pub root: PathBuf,
    pub ledger: PathBuf,
    pub contracts: PathBuf,
    pub orgs: PathBuf,
    pub tenders: PathBuf,
    pub achievements_catalog: PathBuf,
}

impl EconomyPaths {
    pub fn from_root(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            ledger: root.join("economy-ledger.jsonl"),
            contracts: root.join("economy-contracts.jsonl"),
            orgs: root.join("organizations.jsonl"),
            tenders: root.join("tenders.jsonl"),
            achievements_catalog: root.join("achievements-catalog.json"),
            root,
        }
    }

    /// Resolve using the same env overrides as buzz-dev-mcp / Desktop.
    pub fn resolve() -> Self {
        let root = room_memory_dir();
        let mut paths = Self::from_root(root);
        if let Ok(p) = std::env::var("BONY_ROOM_ECONOMY_LEDGER_PATH") {
            if !p.trim().is_empty() {
                paths.ledger = PathBuf::from(p);
            }
        }
        if let Ok(p) = std::env::var("BONY_ROOM_ECONOMY_CONTRACTS_PATH") {
            if !p.trim().is_empty() {
                paths.contracts = PathBuf::from(p);
            }
        }
        if let Ok(p) = std::env::var("BONY_ROOM_ECONOMY_ORGS_PATH") {
            if !p.trim().is_empty() {
                paths.orgs = PathBuf::from(p);
            }
        }
        if let Ok(p) = std::env::var("BONY_ROOM_ECONOMY_TENDERS_PATH") {
            if !p.trim().is_empty() {
                paths.tenders = PathBuf::from(p);
            }
        }
        paths
    }
}

pub fn room_memory_dir() -> PathBuf {
    if let Ok(p) = std::env::var("BONY_ROOM_MEMORY_DIR") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        return PathBuf::from(home).join(".bony-build").join("room-memory");
    }
    PathBuf::from(".bony-build").join("room-memory")
}

pub fn ledger_path() -> PathBuf {
    EconomyPaths::resolve().ledger
}

pub fn contracts_path() -> PathBuf {
    EconomyPaths::resolve().contracts
}

pub fn orgs_path() -> PathBuf {
    EconomyPaths::resolve().orgs
}

pub fn tenders_path() -> PathBuf {
    EconomyPaths::resolve().tenders
}

pub fn achievements_catalog_path() -> PathBuf {
    EconomyPaths::resolve().achievements_catalog
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathOverride {
    pub root: Option<String>,
}

pub fn ensure_parent(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}
