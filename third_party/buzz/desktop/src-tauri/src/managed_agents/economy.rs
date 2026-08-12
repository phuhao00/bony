//! Desktop economy surface — thin wrapper over `buzz-economy`.
//!
//! Grok (`buzz-dev-mcp`) and Desktop both write via the shared crate's
//! locked hash-chain append. Desktop also exposes admin adjustments.

use buzz_economy::{self as eco, EconomyPaths};
use serde::Serialize;

pub use eco::{
    AgentEconomySnapshot as EconomyAgentSnapshot, EconomyWalletView, OrgSnapshot, TenderSnapshot,
};

fn paths() -> EconomyPaths {
    EconomyPaths::resolve()
}

pub fn tier_for(reputation: i32) -> String {
    eco::tier_for(reputation)
}

pub fn get_leaderboard(
    known: &[(String, String)],
    limit: Option<usize>,
) -> Result<Vec<EconomyAgentSnapshot>, String> {
    let paths = paths();
    let mut known = known.to_vec();
    if let Ok(orgs) = eco::list_orgs(&paths) {
        for org in orgs {
            known.push((org.org_id, org.name));
        }
    }
    eco::get_leaderboard(&paths, &known, limit).map_err(|e| e.to_string())
}

pub fn get_wallet(pubkey_or_name: &str) -> Result<Option<EconomyWalletView>, String> {
    eco::get_wallet(
        &paths(),
        &eco::WalletParams {
            pubkey_or_name: pubkey_or_name.to_string(),
            history_limit: Some(20),
        },
    )
    .map_err(|e| e.to_string())
}

pub fn list_orgs() -> Result<Vec<OrgSnapshot>, String> {
    eco::list_orgs(&paths()).map_err(|e| e.to_string())
}

pub fn create_org(
    name: &str,
    founder_pubkey: &str,
    founder_name: Option<&str>,
    tags: Option<Vec<String>>,
) -> Result<OrgSnapshot, String> {
    eco::create_org(
        &paths(),
        eco::OrgCreateParams {
            name: name.to_string(),
            founder_pubkey: founder_pubkey.to_string(),
            founder_name: founder_name.map(str::to_string),
            tags,
        },
    )
    .map_err(|e| e.to_string())
}

pub fn join_org(
    org_id: &str,
    member_pubkey: &str,
    member_name: Option<&str>,
) -> Result<OrgSnapshot, String> {
    eco::join_org(
        &paths(),
        eco::OrgJoinParams {
            org_id: org_id.to_string(),
            member_pubkey: member_pubkey.to_string(),
            member_name: member_name.map(str::to_string),
        },
    )
    .map_err(|e| e.to_string())
}

pub fn leave_org(org_id: &str, member_pubkey: &str) -> Result<OrgSnapshot, String> {
    eco::leave_org(
        &paths(),
        eco::OrgLeaveParams {
            org_id: org_id.to_string(),
            member_pubkey: member_pubkey.to_string(),
        },
    )
    .map_err(|e| e.to_string())
}

pub fn list_tenders(status: Option<&str>) -> Result<Vec<TenderSnapshot>, String> {
    eco::list_tenders(
        &paths(),
        eco::TenderListParams {
            status: status.map(str::to_string),
        },
    )
    .map_err(|e| e.to_string())
}

pub fn publish_tender(
    title: &str,
    capability: &str,
    budget: i64,
    task_ref: &str,
) -> Result<TenderSnapshot, String> {
    eco::publish_tender(
        &paths(),
        eco::TenderPublishParams {
            title: title.to_string(),
            capability: capability.to_string(),
            budget,
            task_ref: task_ref.to_string(),
        },
    )
    .map_err(|e| e.to_string())
}

pub fn admin_adjust_balance(
    pubkey: &str,
    name: Option<&str>,
    delta: i64,
    note: Option<&str>,
) -> Result<(), String> {
    eco::admin::adjust_balance(&paths(), pubkey, name, delta, note)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub fn admin_adjust_reputation(
    pubkey: &str,
    name: Option<&str>,
    delta: i32,
    note: Option<&str>,
) -> Result<(), String> {
    eco::admin::adjust_reputation(&paths(), pubkey, name, delta, note)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub fn admin_set_tags(
    pubkey: &str,
    name: Option<&str>,
    tags: Vec<String>,
    note: Option<&str>,
) -> Result<(), String> {
    eco::admin::set_tags(&paths(), pubkey, name, tags, note)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub fn admin_grant_achievement(
    pubkey: &str,
    name: Option<&str>,
    achievement_id: &str,
    gold: i64,
    reputation: i32,
    note: Option<&str>,
) -> Result<(), String> {
    eco::admin::grant_achievement(
        &paths(),
        pubkey,
        name,
        achievement_id,
        gold,
        reputation,
        note,
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainHealth {
    pub ok: bool,
    pub entries: usize,
    pub tip_hash: String,
    pub broken_at: Option<usize>,
    pub reason: Option<String>,
}

pub fn verify_ledger_chain() -> Result<ChainHealth, String> {
    let report =
        eco::verify_chain::<eco::LedgerEntry>(&paths().ledger).map_err(|e| e.to_string())?;
    Ok(ChainHealth {
        ok: report.ok,
        entries: report.entries,
        tip_hash: report.tip_hash,
        broken_at: report.broken_at,
        reason: report.reason,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_thresholds() {
        assert_eq!(tier_for(0), "Novice");
        assert_eq!(tier_for(100), "Adept");
        assert_eq!(tier_for(500), "Expert");
    }
}
