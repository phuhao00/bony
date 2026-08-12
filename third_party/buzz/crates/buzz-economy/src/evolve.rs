//! Capability self-evolution — routing/bidding labels only.
//!
//! HARD BOUNDARY: `CapabilityGrant` entries expand route eligibility and
//! auction scoring. They MUST NOT write or influence `BUZZ_ACP_DENY_TOOLS`,
//! `session/request_permission`, or any ACP tool permission decision.
//! Effective permission remains:
//!   user auth ∩ ACP allow/deny ∩ runtime capability ∩ room policy.

use crate::auction::load_contracts;
use crate::chain::append_chained;
use crate::error::EconomyError;
use crate::ledger::fold_ledger;
use crate::paths::EconomyPaths;
use crate::types::{LedgerEntry, LedgerKind};
use chrono::Utc;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// Consecutive successful settlements on a capability required to grant it.
const SUCCESS_STREAK_THRESHOLD: usize = 3;

#[derive(Debug, Clone, Serialize)]
pub struct EvolveResult {
    pub pubkey: String,
    pub granted: Vec<String>,
    pub evidence: BTreeMap<String, Vec<String>>,
}

pub fn evolve_capabilities(
    paths: &EconomyPaths,
    pubkey: &str,
    name: Option<&str>,
) -> Result<EvolveResult, EconomyError> {
    let balances = fold_ledger(paths)?;
    let already: BTreeSet<String> = balances
        .get(pubkey)
        .map(|b| b.capability_grants.clone())
        .unwrap_or_default();

    let contracts = load_contracts(paths)?;
    // Group settled_success by capability for this pubkey (newest first).
    let mut by_cap: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for c in contracts
        .into_iter()
        .filter(|c| c.winner_pubkey.eq_ignore_ascii_case(pubkey))
        .filter(|c| c.status == "settled_success")
    {
        by_cap
            .entry(c.capability.clone())
            .or_default()
            .push(c.contract_id);
    }

    let mut granted = Vec::new();
    let mut evidence = BTreeMap::new();
    for (cap, ids) in by_cap {
        if already.contains(&cap) {
            continue;
        }
        if ids.len() < SUCCESS_STREAK_THRESHOLD {
            continue;
        }
        let proof: Vec<String> = ids.into_iter().take(SUCCESS_STREAK_THRESHOLD).collect();
        append_chained(
            &paths.ledger,
            &LedgerEntry {
                ts: Utc::now().to_rfc3339(),
                pubkey: pubkey.to_string(),
                kind: LedgerKind::CapabilityGrant,
                amount: 0,
                reputation_delta: 0,
                task_ref: None,
                note: Some(format!(
                    "self-evolve capability grant (routing only): {cap}; evidence={}",
                    proof.join(",")
                )),
                name: name.map(str::to_string),
                tags: Vec::new(),
                achievements: Vec::new(),
                capability_grants: vec![cap.clone()],
                prev_hash: None,
                hash: None,
            },
        )?;
        evidence.insert(cap.clone(), proof);
        granted.push(cap);
    }

    Ok(EvolveResult {
        pubkey: pubkey.to_string(),
        granted,
        evidence,
    })
}

/// Dynamic capability grants folded from the ledger (routing overlay).
pub fn granted_capabilities(
    paths: &EconomyPaths,
    pubkey: &str,
) -> Result<Vec<String>, EconomyError> {
    let balances = fold_ledger(paths)?;
    Ok(balances
        .get(pubkey)
        .map(|b| b.capability_grants.iter().cloned().collect())
        .unwrap_or_default())
}
