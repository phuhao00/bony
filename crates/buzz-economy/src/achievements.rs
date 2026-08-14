use crate::chain::append_chained;
use crate::error::EconomyError;
use crate::ledger::AgentBalance;
use crate::paths::EconomyPaths;
use crate::types::{LedgerEntry, LedgerKind};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AchievementDef {
    pub id: String,
    pub title: String,
    pub description: String,
    pub gold_reward: i64,
    pub reputation_reward: i32,
    /// "first_contract" | "gold_threshold" | "rep_threshold" | "tier_reached"
    pub kind: String,
    #[serde(default)]
    pub threshold: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AchievementCatalog {
    pub achievements: Vec<AchievementDef>,
}

pub fn default_catalog() -> AchievementCatalog {
    AchievementCatalog {
        achievements: vec![
            AchievementDef {
                id: "first_contract_won".into(),
                title: "First Contract".into(),
                description: "Complete the first successful settlement".into(),
                gold_reward: 20,
                reputation_reward: 5,
                kind: "first_contract".into(),
                threshold: 1,
            },
            AchievementDef {
                id: "gold_1000".into(),
                title: "Thousand Credits".into(),
                description: "Reach a balance of 1000 credits".into(),
                gold_reward: 0,
                reputation_reward: 15,
                kind: "gold_threshold".into(),
                threshold: 1000,
            },
            AchievementDef {
                id: "tier_reached_adept".into(),
                title: "Reach Adept".into(),
                description: "Reach the Adept reputation tier".into(),
                gold_reward: 30,
                reputation_reward: 0,
                kind: "rep_threshold".into(),
                threshold: 100,
            },
            AchievementDef {
                id: "tier_reached_expert".into(),
                title: "Reach Expert".into(),
                description: "Reach the Expert reputation tier".into(),
                gold_reward: 80,
                reputation_reward: 0,
                kind: "rep_threshold".into(),
                threshold: 500,
            },
        ],
    }
}

pub fn load_catalog(paths: &EconomyPaths) -> AchievementCatalog {
    match fs::read_to_string(&paths.achievements_catalog) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|_| default_catalog()),
        Err(_) => default_catalog(),
    }
}

pub fn save_catalog(
    paths: &EconomyPaths,
    catalog: &AchievementCatalog,
) -> Result<(), EconomyError> {
    if let Some(parent) = paths.achievements_catalog.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| EconomyError::io("create catalog dir", &paths.achievements_catalog, e))?;
    }
    let raw = serde_json::to_string_pretty(catalog)
        .map_err(|e| EconomyError::Serialize(format!("catalog: {e}")))?;
    fs::write(&paths.achievements_catalog, raw)
        .map_err(|e| EconomyError::io("write catalog", &paths.achievements_catalog, e))?;
    Ok(())
}

/// Compare before/after balances and unlock any newly met achievements.
/// Returns newly unlocked achievement ids.
pub fn evaluate_achievements(
    paths: &EconomyPaths,
    pubkey: &str,
    name: Option<&str>,
    before: &AgentBalance,
    after: &AgentBalance,
    catalog: &AchievementCatalog,
) -> Result<Vec<String>, EconomyError> {
    let mut unlocked = Vec::new();
    for def in &catalog.achievements {
        if after.achievements.contains(&def.id) || before.achievements.contains(&def.id) {
            continue;
        }
        let met = match def.kind.as_str() {
            "first_contract" => {
                after.reputation > before.reputation || after.balance > before.balance
            }
            "gold_threshold" => after.balance >= def.threshold && before.balance < def.threshold,
            "rep_threshold" => {
                after.reputation as i64 >= def.threshold
                    && (before.reputation as i64) < def.threshold
            }
            _ => false,
        };
        if !met {
            continue;
        }
        append_chained(
            &paths.ledger,
            &LedgerEntry {
                ts: Utc::now().to_rfc3339(),
                pubkey: pubkey.to_string(),
                kind: LedgerKind::Achievement,
                amount: def.gold_reward,
                reputation_delta: def.reputation_reward,
                task_ref: None,
                note: Some(format!("achievement unlocked: {}", def.title)),
                name: name.map(str::to_string),
                tags: Vec::new(),
                achievements: vec![def.id.clone()],
                capability_grants: Vec::new(),
                prev_hash: None,
                hash: None,
            },
        )?;
        unlocked.push(def.id.clone());
    }
    Ok(unlocked)
}
