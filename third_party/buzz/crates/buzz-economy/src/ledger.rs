use crate::error::EconomyError;
use crate::paths::EconomyPaths;
use crate::types::{LedgerEntry, LedgerKind, RosterAgent};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

pub const STARTING_BALANCE: i64 = 100;
pub const STARTING_REPUTATION: i32 = 0;
const MAX_SCAN_LINES: usize = 5000;
const DEFAULT_LEADERBOARD_LIMIT: usize = 20;
const MAX_LEADERBOARD_LIMIT: usize = 50;
const DEFAULT_WALLET_HISTORY: usize = 12;

#[derive(Debug, Clone)]
pub struct AgentBalance {
    pub balance: i64,
    pub reputation: i32,
    pub name: String,
    pub tags: Vec<String>,
    pub achievements: BTreeSet<String>,
    pub capability_grants: BTreeSet<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentEconomySnapshot {
    pub name: String,
    pub pubkey: String,
    pub balance: i64,
    pub reputation: i32,
    pub tier: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub achievements: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LedgerHistoryEntry {
    pub ts: String,
    pub kind: String,
    pub amount: i64,
    pub reputation_delta: i32,
    pub task_ref: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EconomyWalletView {
    pub name: String,
    pub pubkey: String,
    pub balance: i64,
    pub reputation: i32,
    pub tier: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub achievements: Vec<String>,
    #[serde(default)]
    pub capability_grants: Vec<String>,
    #[serde(default)]
    pub history: Vec<LedgerHistoryEntry>,
}

#[derive(Debug, Clone)]
pub struct WalletParams {
    pub pubkey_or_name: String,
    pub history_limit: Option<usize>,
}

pub fn tier_for(reputation: i32) -> String {
    let r = reputation.max(0);
    if r < 100 {
        "Novice".into()
    } else if r < 500 {
        "Adept".into()
    } else if r < 2000 {
        "Expert".into()
    } else if r < 5000 {
        "Master".into()
    } else {
        "Legend".into()
    }
}

pub fn fold_ledger(paths: &EconomyPaths) -> Result<BTreeMap<String, AgentBalance>, EconomyError> {
    fold_ledger_at(&paths.ledger)
}

pub fn fold_ledger_at(path: &Path) -> Result<BTreeMap<String, AgentBalance>, EconomyError> {
    let mut map: BTreeMap<String, AgentBalance> = BTreeMap::new();
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(map),
        Err(e) => return Err(EconomyError::io("open economy ledger", path, e)),
    };
    let lines: Vec<String> = BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter(|l| !l.trim().is_empty())
        .collect();
    let start = lines.len().saturating_sub(MAX_SCAN_LINES);
    for line in &lines[start..] {
        let Ok(entry) = serde_json::from_str::<LedgerEntry>(line) else {
            continue;
        };
        if entry.pubkey.trim().is_empty() {
            continue;
        }
        let slot = map
            .entry(entry.pubkey.clone())
            .or_insert_with(|| AgentBalance {
                balance: STARTING_BALANCE,
                reputation: STARTING_REPUTATION,
                name: entry.name.clone().unwrap_or_else(|| entry.pubkey.clone()),
                tags: Vec::new(),
                achievements: BTreeSet::new(),
                capability_grants: BTreeSet::new(),
            });
        if let Some(name) = &entry.name {
            if !name.trim().is_empty() {
                slot.name = name.clone();
            }
        }
        slot.balance = slot.balance.saturating_add(entry.amount);
        if slot.balance < 0 {
            slot.balance = 0;
        }
        slot.reputation = slot.reputation.saturating_add(entry.reputation_delta);
        match entry.kind {
            LedgerKind::TagAssign => {
                slot.tags = entry.tags;
            }
            LedgerKind::Achievement => {
                for a in entry.achievements {
                    slot.achievements.insert(a);
                }
            }
            LedgerKind::CapabilityGrant => {
                for c in entry.capability_grants {
                    slot.capability_grants.insert(c);
                }
            }
            _ => {}
        }
    }
    Ok(map)
}

pub fn get_leaderboard(
    paths: &EconomyPaths,
    known: &[(String, String)],
    limit: Option<usize>,
) -> Result<Vec<AgentEconomySnapshot>, EconomyError> {
    let balances = fold_ledger(paths)?;
    let limit = limit
        .unwrap_or(DEFAULT_LEADERBOARD_LIMIT)
        .clamp(1, MAX_LEADERBOARD_LIMIT);
    let mut rows: Vec<AgentEconomySnapshot> = Vec::new();
    let mut seen = BTreeSet::new();

    for (pubkey, name) in known {
        if pubkey.trim().is_empty() {
            continue;
        }
        seen.insert(pubkey.to_ascii_lowercase());
        let bal = balances.get(pubkey).cloned().unwrap_or(AgentBalance {
            balance: STARTING_BALANCE,
            reputation: STARTING_REPUTATION,
            name: name.clone(),
            tags: Vec::new(),
            achievements: BTreeSet::new(),
            capability_grants: BTreeSet::new(),
        });
        rows.push(snapshot(pubkey, name, &bal));
    }
    for (pubkey, bal) in &balances {
        if seen.contains(&pubkey.to_ascii_lowercase()) {
            continue;
        }
        rows.push(snapshot(pubkey, &bal.name, bal));
    }
    rows.sort_by(|left, right| {
        right
            .reputation
            .cmp(&left.reputation)
            .then_with(|| right.balance.cmp(&left.balance))
            .then_with(|| {
                left.pubkey
                    .to_ascii_lowercase()
                    .cmp(&right.pubkey.to_ascii_lowercase())
            })
    });
    rows.truncate(limit);
    Ok(rows)
}

pub fn get_leaderboard_from_roster(
    paths: &EconomyPaths,
    roster: &[RosterAgent],
    limit: Option<usize>,
) -> Result<Vec<AgentEconomySnapshot>, EconomyError> {
    let known: Vec<(String, String)> = roster
        .iter()
        .filter(|a| !a.pubkey.is_empty())
        .map(|a| (a.pubkey.clone(), a.name.clone()))
        .collect();
    get_leaderboard(paths, &known, limit)
}

pub fn get_wallet(
    paths: &EconomyPaths,
    p: &WalletParams,
) -> Result<Option<EconomyWalletView>, EconomyError> {
    let key = p.pubkey_or_name.trim();
    if key.is_empty() {
        return Err(EconomyError::invalid("pubkey_or_name must not be empty"));
    }
    let balances = fold_ledger(paths)?;
    let Some((pubkey, bal)) = balances
        .iter()
        .find(|(pk, bal)| pk.eq_ignore_ascii_case(key) || bal.name.eq_ignore_ascii_case(key))
        .map(|(pk, bal)| (pk.clone(), bal.clone()))
    else {
        return Ok(None);
    };
    let limit = p.history_limit.unwrap_or(DEFAULT_WALLET_HISTORY);
    let history = recent_ledger_for(&paths.ledger, &pubkey, limit)?;
    Ok(Some(EconomyWalletView {
        name: bal.name.clone(),
        pubkey,
        balance: bal.balance,
        reputation: bal.reputation,
        tier: tier_for(bal.reputation),
        tags: bal.tags,
        achievements: bal.achievements.into_iter().collect(),
        capability_grants: bal.capability_grants.into_iter().collect(),
        history,
    }))
}

fn snapshot(pubkey: &str, name: &str, bal: &AgentBalance) -> AgentEconomySnapshot {
    AgentEconomySnapshot {
        name: name.to_string(),
        pubkey: pubkey.to_string(),
        balance: bal.balance,
        reputation: bal.reputation,
        tier: tier_for(bal.reputation),
        tags: bal.tags.clone(),
        achievements: bal.achievements.iter().cloned().collect(),
    }
}

fn recent_ledger_for(
    path: &Path,
    pubkey: &str,
    limit: usize,
) -> Result<Vec<LedgerHistoryEntry>, EconomyError> {
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(EconomyError::io("open economy ledger", path, e)),
    };
    let lines: Vec<String> = BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter(|l| !l.trim().is_empty())
        .collect();
    let start = lines.len().saturating_sub(MAX_SCAN_LINES);
    let mut matches = Vec::new();
    for line in lines[start..].iter().rev() {
        let Ok(entry) = serde_json::from_str::<LedgerEntry>(line) else {
            continue;
        };
        if entry.pubkey.eq_ignore_ascii_case(pubkey) {
            matches.push(LedgerHistoryEntry {
                ts: entry.ts,
                kind: format!("{:?}", entry.kind).to_ascii_lowercase(),
                amount: entry.amount,
                reputation_delta: entry.reputation_delta,
                task_ref: entry.task_ref,
                note: entry.note,
            });
            if matches.len() >= limit {
                break;
            }
        }
    }
    Ok(matches)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::append_chained;
    use tempfile::tempdir;

    #[test]
    fn fold_applies_starting_balance_and_tags() {
        let dir = tempdir().unwrap();
        let paths = EconomyPaths::from_root(dir.path());
        let entry = LedgerEntry {
            ts: "t".into(),
            pubkey: "pk1".into(),
            kind: LedgerKind::Payout,
            amount: 40,
            reputation_delta: 10,
            task_ref: None,
            note: None,
            name: Some("ZeroClaw".into()),
            tags: Vec::new(),
            achievements: Vec::new(),
            capability_grants: Vec::new(),
            prev_hash: None,
            hash: None,
        };
        append_chained(&paths.ledger, &entry).unwrap();
        append_chained(
            &paths.ledger,
            &LedgerEntry {
                kind: LedgerKind::TagAssign,
                amount: 0,
                reputation_delta: 0,
                tags: vec!["swift".into()],
                ..entry.clone()
            },
        )
        .unwrap();
        let map = fold_ledger(&paths).unwrap();
        let bal = map.get("pk1").unwrap();
        assert_eq!(bal.balance, 140);
        assert_eq!(bal.reputation, 10);
        assert_eq!(bal.tags, vec!["swift".to_string()]);
        assert_eq!(tier_for(10), "Novice");
    }
}
