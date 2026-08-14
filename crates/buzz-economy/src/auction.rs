use crate::achievements::{evaluate_achievements, load_catalog};
use crate::chain::append_chained;
use crate::error::EconomyError;
use crate::evolve::evolve_capabilities;
use crate::ledger::{fold_ledger, tier_for, AgentBalance, STARTING_BALANCE, STARTING_REPUTATION};
use crate::paths::EconomyPaths;
use crate::reward::{
    compute_settlement_reward, SettlementRewardInput, FAIL_MISMATCH_PENALTY_BPS,
};
use crate::types::{BidderKind, ContractRecord, LedgerEntry, LedgerKind, RosterAgent};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

const MAX_SUBCONTRACT_DEPTH: u32 = 2;
const MAX_SCAN_LINES: usize = 5000;

const W_CAPABILITY: f64 = 0.5;
const W_REPUTATION: f64 = 0.3;
const W_STAKE: f64 = 0.2;
pub const MATCH_FULL: f64 = 1.0;
pub const MATCH_PARTIAL_PREFIX: f64 = 0.7;
pub const MATCH_MISMATCH: f64 = 0.3;

#[derive(Debug, Clone, Deserialize)]
pub struct AuctionParams {
    pub capability: String,
    pub budget: i64,
    pub task_ref: String,
    pub max_stake: Option<i64>,
    pub bidder_kind: Option<BidderKind>,
    /// Optional org id when bidding as organization (`org:<slug>` pubkey).
    pub org_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuctionResult {
    pub contract_id: String,
    pub winner_name: String,
    pub winner_pubkey: String,
    pub score: f64,
    pub mismatch: bool,
    pub budget: i64,
    pub reason: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HireSupportParams {
    /// Prime contractor paying for help.
    pub payer_pubkey: String,
    pub payer_name: Option<String>,
    /// Capability the hiree must cover (e.g. `research.web`).
    pub capability: String,
    pub task_ref: String,
    /// Max gold the prime is willing to spend on this hire.
    pub max_pay: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct HireSupportResult {
    pub hiree_name: String,
    pub hiree_pubkey: String,
    pub paid: i64,
    pub score: f64,
    pub mismatch: bool,
    pub reason: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SubcontractParams {
    pub contract_id: String,
    pub to_capability_or_name: String,
    pub cut_bp: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubcontractResult {
    pub parent_contract_id: String,
    pub child_contract_id: String,
    pub winner_name: String,
    pub remaining_budget: i64,
    pub brokerage: i64,
    pub cut_bp: u32,
    pub mismatch: bool,
    pub depth: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SettleParams {
    pub contract_id: String,
    pub status: String,
    pub quality_note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SettleResult {
    pub contract_id: String,
    pub settled_status: String,
    pub winner_name: String,
    pub balance: i64,
    pub reputation: i32,
    pub tier: String,
    pub note: String,
    /// Net gold credited (positive payout) or clawed back (negative on fail+mismatch).
    pub paid_gold: i64,
    pub reputation_delta: i32,
    pub quality_grade: String,
    pub grant_title: bool,
    pub new_achievements: Vec<String>,
    pub new_capabilities: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AuctionCandidate {
    pub name: String,
    pub pubkey: String,
    pub capabilities: Vec<String>,
    pub score: f64,
    pub mismatch: bool,
    pub reason: String,
}

pub fn auction(
    paths: &EconomyPaths,
    agents: &[RosterAgent],
    p: AuctionParams,
) -> Result<AuctionResult, EconomyError> {
    let capability = p.capability.trim();
    let task_ref = p.task_ref.trim();
    if capability.is_empty() {
        return Err(EconomyError::invalid("capability must not be empty"));
    }
    if task_ref.is_empty() {
        return Err(EconomyError::invalid("task_ref must not be empty"));
    }
    if p.budget <= 0 {
        return Err(EconomyError::invalid("budget must be a positive integer"));
    }

    let balances = fold_ledger(paths)?;
    let max_stake = p.max_stake.unwrap_or(p.budget).max(1);

    // If org_id is set, treat that org as the sole bidder (org wallet pubkey).
    let winner = if let Some(org_id) = p.org_id.as_deref().map(str::trim).filter(|s| !s.is_empty())
    {
        let org_pubkey = if org_id.starts_with("org:") {
            org_id.to_string()
        } else {
            format!("org:{org_id}")
        };
        let org_agent = agents
            .iter()
            .find(|a| a.pubkey.eq_ignore_ascii_case(&org_pubkey))
            .cloned()
            .unwrap_or(RosterAgent {
                name: org_pubkey.clone(),
                pubkey: org_pubkey.clone(),
                capabilities: Vec::new(),
                status: "running".into(),
            });
        let ratio = capability_match_ratio(&org_agent.capabilities, capability);
        let bal = balances
            .get(&org_pubkey)
            .cloned()
            .unwrap_or_else(|| empty_balance(&org_agent.name));
        let norm_rep = normalize_reputation(bal.reputation);
        let stake = bal.balance.clamp(0, max_stake) as f64 / max_stake.max(1) as f64;
        let score = W_CAPABILITY * ratio + W_REPUTATION * norm_rep + W_STAKE * stake;
        AuctionCandidate {
            name: org_agent.name,
            pubkey: org_pubkey,
            capabilities: org_agent.capabilities,
            score,
            mismatch: ratio < MATCH_FULL,
            reason: format!("org bid cap={ratio:.2} rep_n={norm_rep:.2} stake_n={stake:.2}"),
        }
    } else {
        pick_auction_winner(agents, capability, &balances, max_stake, None, true).ok_or_else(
            || {
                EconomyError::invalid(format!(
                    "no running agents available to auction for \"{capability}\""
                ))
            },
        )?
    };

    let contract_id = new_contract_id();
    let record = ContractRecord {
        ts: Utc::now().to_rfc3339(),
        contract_id: contract_id.clone(),
        task_ref: task_ref.to_string(),
        capability: capability.to_string(),
        budget: p.budget,
        winner_name: winner.name.clone(),
        winner_pubkey: winner.pubkey.clone(),
        effective_score: winner.score,
        mismatch: winner.mismatch,
        parent_contract_id: None,
        cut_bp: None,
        depth: 0,
        status: "awarded".into(),
        bidder_kind: p.bidder_kind.or(Some(if winner.pubkey.starts_with("org:") {
            BidderKind::Org
        } else {
            BidderKind::Agent
        })),
        prev_hash: None,
        hash: None,
    };
    append_chained(&paths.contracts, &record)?;

    Ok(AuctionResult {
        contract_id,
        winner_name: winner.name,
        winner_pubkey: winner.pubkey,
        score: winner.score,
        mismatch: winner.mismatch,
        budget: p.budget,
        reason: winner.reason,
        capabilities: winner.capabilities,
    })
}

pub fn subcontract(
    paths: &EconomyPaths,
    agents: &[RosterAgent],
    p: SubcontractParams,
) -> Result<SubcontractResult, EconomyError> {
    let contract_id = p.contract_id.trim();
    let target = p.to_capability_or_name.trim();
    if contract_id.is_empty() {
        return Err(EconomyError::invalid("contract_id must not be empty"));
    }
    if target.is_empty() {
        return Err(EconomyError::invalid(
            "to_capability_or_name must not be empty",
        ));
    }
    let cut_bp = p.cut_bp.unwrap_or(1000).min(5000);
    let parent = latest_contract(paths, contract_id)?
        .ok_or_else(|| EconomyError::invalid(format!("unknown contract_id \"{contract_id}\"")))?;
    if parent.status != "awarded" && parent.status != "subcontracted" {
        return Err(EconomyError::invalid(format!(
            "contract {contract_id} status is \"{}\" — only awarded/subcontracted contracts can be subcontracted",
            parent.status
        )));
    }
    if parent.depth >= MAX_SUBCONTRACT_DEPTH {
        return Err(EconomyError::invalid(format!(
            "reached subcontract depth limit ({MAX_SUBCONTRACT_DEPTH}) for {contract_id}"
        )));
    }

    let remaining = parent.budget.saturating_mul(10_000 - cut_bp as i64) / 10_000;
    let brokerage = parent.budget.saturating_sub(remaining);
    let balances = fold_ledger(paths)?;

    let name_pin = agents
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case(target) && a.status.eq_ignore_ascii_case("running"));
    let winner = if let Some(agent) = name_pin {
        if agent.pubkey.eq_ignore_ascii_case(&parent.winner_pubkey) {
            return Err(EconomyError::invalid(
                "cannot subcontract to the same agent",
            ));
        }
        let caps = &agent.capabilities;
        let ratio = capability_match_ratio(caps, &parent.capability);
        AuctionCandidate {
            name: agent.name.clone(),
            pubkey: agent.pubkey.clone(),
            capabilities: caps.clone(),
            score: ratio,
            mismatch: ratio < MATCH_FULL,
            reason: "explicit name pin".into(),
        }
    } else {
        pick_auction_winner(
            agents,
            target,
            &balances,
            remaining.max(1),
            Some(parent.winner_pubkey.as_str()),
            true,
        )
        .ok_or_else(|| {
            EconomyError::invalid(format!(
                "no running agents available to subcontract for \"{target}\""
            ))
        })?
    };

    if brokerage > 0 {
        append_chained(
            &paths.ledger,
            &LedgerEntry {
                ts: Utc::now().to_rfc3339(),
                pubkey: parent.winner_pubkey.clone(),
                kind: LedgerKind::Brokerage,
                amount: brokerage,
                reputation_delta: 0,
                task_ref: Some(parent.task_ref.clone()),
                note: Some(format!(
                    "broker cut {cut_bp}bp on subcontract of {contract_id}"
                )),
                name: Some(parent.winner_name.clone()),
                tags: Vec::new(),
                achievements: Vec::new(),
                capability_grants: Vec::new(),
                prev_hash: None,
                hash: None,
            },
        )?;
    }

    let parent_update = ContractRecord {
        ts: Utc::now().to_rfc3339(),
        status: "subcontracted".into(),
        prev_hash: None,
        hash: None,
        ..parent.clone()
    };
    append_chained(&paths.contracts, &parent_update)?;

    let child_id = new_contract_id();
    let child = ContractRecord {
        ts: Utc::now().to_rfc3339(),
        contract_id: child_id.clone(),
        task_ref: parent.task_ref.clone(),
        capability: if name_pin.is_some() {
            parent.capability.clone()
        } else {
            target.to_string()
        },
        budget: remaining,
        winner_name: winner.name.clone(),
        winner_pubkey: winner.pubkey.clone(),
        effective_score: winner.score,
        mismatch: winner.mismatch,
        parent_contract_id: Some(contract_id.to_string()),
        cut_bp: Some(cut_bp),
        depth: parent.depth + 1,
        status: "awarded".into(),
        bidder_kind: Some(BidderKind::Agent),
        prev_hash: None,
        hash: None,
    };
    append_chained(&paths.contracts, &child)?;

    Ok(SubcontractResult {
        parent_contract_id: contract_id.to_string(),
        child_contract_id: child_id,
        winner_name: winner.name,
        remaining_budget: remaining,
        brokerage,
        cut_bp,
        mismatch: winner.mismatch,
        depth: child.depth,
    })
}

/// Prime contractor hires help on the open market (or for an explicit capability).
///
/// Selection uses reputation / stake (+ capability diversity vs the payer when
/// capability is `open`). Payment moves gold from the payer wallet to the hiree;
/// the awarded contract leaf is unchanged (prime still settles the main job).
pub fn hire_support(
    paths: &EconomyPaths,
    agents: &[RosterAgent],
    p: HireSupportParams,
) -> Result<HireSupportResult, EconomyError> {
    let capability = p.capability.trim();
    let payer = p.payer_pubkey.trim();
    if payer.is_empty() {
        return Err(EconomyError::invalid("payer_pubkey must not be empty"));
    }
    if p.max_pay <= 0 {
        return Err(EconomyError::invalid("max_pay must be a positive integer"));
    }

    let balances = fold_ledger(paths)?;
    let payer_bal = balances
        .get(payer)
        .cloned()
        .unwrap_or_else(|| empty_balance(p.payer_name.as_deref().unwrap_or(payer)));
    let affordable = payer_bal.balance.max(0);
    if affordable <= 0 {
        return Err(EconomyError::invalid(format!(
            "payer {payer} has no balance to hire support"
        )));
    }

    let open = capability.is_empty()
        || capability.eq_ignore_ascii_case("open")
        || capability == "*";
    let payer_caps = agents
        .iter()
        .find(|a| a.pubkey.eq_ignore_ascii_case(payer))
        .map(|a| a.capabilities.as_slice())
        .unwrap_or(&[]);

    let hiree = if open {
        pick_open_market_winner(
            agents,
            &balances,
            p.max_pay.max(1),
            Some(payer),
            true,
            Some(payer_caps),
        )
    } else {
        pick_auction_winner(
            agents,
            capability,
            &balances,
            p.max_pay.max(1),
            Some(payer),
            true,
        )
    }
    .ok_or_else(|| {
        EconomyError::invalid(format!(
            "no running agents available to hire for \"{}\"",
            if open { "open" } else { capability }
        ))
    })?;

    if hiree.pubkey.eq_ignore_ascii_case(payer) {
        return Err(EconomyError::invalid("cannot hire yourself"));
    }

    let paid = p.max_pay.min(affordable).max(1);
    let payer_name = p
        .payer_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(payer);
    let task_ref = p.task_ref.trim();
    let cap_label = if open { "open" } else { capability };

    append_chained(
        &paths.ledger,
        &LedgerEntry {
            ts: Utc::now().to_rfc3339(),
            pubkey: payer.to_string(),
            kind: LedgerKind::Brokerage,
            amount: -paid,
            reputation_delta: 0,
            task_ref: (!task_ref.is_empty()).then(|| task_ref.to_string()),
            note: Some(format!("hired @{} for {cap_label} (support)", hiree.name)),
            name: Some(payer_name.to_string()),
            tags: Vec::new(),
            achievements: Vec::new(),
            capability_grants: Vec::new(),
            prev_hash: None,
            hash: None,
        },
    )?;
    append_chained(
        &paths.ledger,
        &LedgerEntry {
            ts: Utc::now().to_rfc3339(),
            pubkey: hiree.pubkey.clone(),
            kind: LedgerKind::Payout,
            amount: paid,
            reputation_delta: 1,
            task_ref: (!task_ref.is_empty()).then(|| task_ref.to_string()),
            note: Some(format!(
                "support hire from @{payer_name} for {cap_label}"
            )),
            name: Some(hiree.name.clone()),
            tags: Vec::new(),
            achievements: Vec::new(),
            capability_grants: Vec::new(),
            prev_hash: None,
            hash: None,
        },
    )?;

    Ok(HireSupportResult {
        hiree_name: hiree.name,
        hiree_pubkey: hiree.pubkey,
        paid,
        score: hiree.score,
        mismatch: hiree.mismatch,
        reason: hiree.reason,
        capabilities: hiree.capabilities,
    })
}

pub fn settle(paths: &EconomyPaths, p: SettleParams) -> Result<SettleResult, EconomyError> {
    let contract_id = p.contract_id.trim();
    if contract_id.is_empty() {
        return Err(EconomyError::invalid("contract_id must not be empty"));
    }
    let status_raw = p.status.trim().to_ascii_lowercase();
    let success = match status_raw.as_str() {
        "success" | "done" | "ok" => true,
        "failed" | "fail" | "blocked" => false,
        _ => {
            return Err(EconomyError::invalid(
                "status must be \"success\" or \"failed\"",
            ))
        }
    };

    let leaf = latest_contract(paths, contract_id)?
        .ok_or_else(|| EconomyError::invalid(format!("unknown contract_id \"{contract_id}\"")))?;
    if leaf.status.starts_with("settled_") {
        return Err(EconomyError::invalid(format!(
            "contract {contract_id} already settled ({})",
            leaf.status
        )));
    }

    let executor = resolve_executor_contract(paths, &leaf)?;
    let balances_before = fold_ledger(paths)?;
    let current = balances_before
        .get(&executor.winner_pubkey)
        .cloned()
        .unwrap_or_else(|| empty_balance(&executor.winner_name));

    let note = p
        .quality_note
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let reward = compute_settlement_reward(SettlementRewardInput {
        budget: executor.budget,
        capability: &executor.capability,
        mismatch: executor.mismatch,
        declared_success: success,
        outcome: note.as_deref().unwrap_or(""),
    });

    // Effective ledger success follows quality grade (Thin still pays partial).
    let ledger_success = !matches!(reward.grade, crate::reward::QualityGrade::Fail);

    let paid_gold;
    if ledger_success {
        paid_gold = reward.gold;
        append_chained(
            &paths.ledger,
            &LedgerEntry {
                ts: Utc::now().to_rfc3339(),
                pubkey: executor.winner_pubkey.clone(),
                kind: LedgerKind::Payout,
                amount: reward.gold,
                reputation_delta: reward.reputation,
                task_ref: Some(executor.task_ref.clone()),
                note: Some(reward.note.clone()),
                name: Some(executor.winner_name.clone()),
                tags: Vec::new(),
                achievements: Vec::new(),
                capability_grants: Vec::new(),
                prev_hash: None,
                hash: None,
            },
        )?;
    } else {
        let mut penalty_amount = 0i64;
        if executor.mismatch {
            penalty_amount = (executor
                .budget
                .saturating_mul(FAIL_MISMATCH_PENALTY_BPS as i64)
                / 10_000)
                .max(1);
            let affordable = current.balance.max(0);
            penalty_amount = penalty_amount.min(affordable);
        }
        paid_gold = -penalty_amount;
        append_chained(
            &paths.ledger,
            &LedgerEntry {
                ts: Utc::now().to_rfc3339(),
                pubkey: executor.winner_pubkey.clone(),
                kind: LedgerKind::Penalty,
                amount: -penalty_amount,
                reputation_delta: reward.reputation,
                task_ref: Some(executor.task_ref.clone()),
                note: Some(reward.note.clone()),
                name: Some(executor.winner_name.clone()),
                tags: Vec::new(),
                achievements: Vec::new(),
                capability_grants: Vec::new(),
                prev_hash: None,
                hash: None,
            },
        )?;

        if let Some(parent_id) = &executor.parent_contract_id {
            if let Some(parent) = latest_contract(paths, parent_id)? {
                const FAIL_REP_BROKER: i32 = -8;
                append_chained(
                    &paths.ledger,
                    &LedgerEntry {
                        ts: Utc::now().to_rfc3339(),
                        pubkey: parent.winner_pubkey.clone(),
                        kind: LedgerKind::Penalty,
                        amount: 0,
                        reputation_delta: FAIL_REP_BROKER,
                        task_ref: Some(parent.task_ref.clone()),
                        note: Some(format!(
                            "broker liability: descendant {} failed",
                            executor.contract_id
                        )),
                        name: Some(parent.winner_name.clone()),
                        tags: Vec::new(),
                        achievements: Vec::new(),
                        capability_grants: Vec::new(),
                        prev_hash: None,
                        hash: None,
                    },
                )?;
            }
        }
    }

    let settled_status = if ledger_success {
        "settled_success"
    } else {
        "settled_failed"
    };
    let update = ContractRecord {
        ts: Utc::now().to_rfc3339(),
        status: settled_status.into(),
        prev_hash: None,
        hash: None,
        ..executor.clone()
    };
    append_chained(&paths.contracts, &update)?;

    if leaf.contract_id != executor.contract_id {
        let parent_update = ContractRecord {
            ts: Utc::now().to_rfc3339(),
            status: settled_status.into(),
            prev_hash: None,
            hash: None,
            ..leaf
        };
        append_chained(&paths.contracts, &parent_update)?;
    }

    let after = fold_ledger(paths)?;
    let snap = after
        .get(&executor.winner_pubkey)
        .cloned()
        .unwrap_or(current.clone());

    let catalog = load_catalog(paths);
    let new_achievements = evaluate_achievements(
        paths,
        &executor.winner_pubkey,
        Some(&executor.winner_name),
        &current,
        &snap,
        &catalog,
    )?;

    let new_capabilities = if ledger_success {
        evolve_capabilities(paths, &executor.winner_pubkey, Some(&executor.winner_name))?.granted
    } else {
        Vec::new()
    };

    Ok(SettleResult {
        contract_id: executor.contract_id,
        settled_status: settled_status.into(),
        winner_name: executor.winner_name,
        balance: snap.balance,
        reputation: snap.reputation,
        tier: tier_for(snap.reputation),
        note: reward.note.clone(),
        paid_gold,
        reputation_delta: reward.reputation,
        quality_grade: reward.grade.as_str().into(),
        grant_title: reward.grant_title,
        new_achievements,
        new_capabilities,
    })
}

pub fn pick_auction_winner(
    agents: &[RosterAgent],
    capability: &str,
    balances: &BTreeMap<String, AgentBalance>,
    max_stake: i64,
    exclude_pubkey: Option<&str>,
    require_running: bool,
) -> Option<AuctionCandidate> {
    let cap = capability.trim();
    if cap.is_empty() || cap.eq_ignore_ascii_case("open") || cap == "*" {
        return pick_open_market_winner(
            agents,
            balances,
            max_stake,
            exclude_pubkey,
            require_running,
            None,
        );
    }
    pick_auction_winner_best_of(
        agents,
        &[capability],
        balances,
        max_stake,
        exclude_pubkey,
        require_running,
    )
}

/// Open market: score by reputation + stake (+ optional capability diversity).
///
/// No title-keyword routing — any running agent can win / be hired.
pub fn pick_open_market_winner(
    agents: &[RosterAgent],
    balances: &BTreeMap<String, AgentBalance>,
    max_stake: i64,
    exclude_pubkey: Option<&str>,
    require_running: bool,
    prefer_unlike_caps: Option<&[String]>,
) -> Option<AuctionCandidate> {
    const W_REP: f64 = 0.55;
    const W_STAKE: f64 = 0.30;
    const W_DIVERSITY: f64 = 0.15;

    let mut scored: Vec<AuctionCandidate> = agents
        .iter()
        .filter(|a| !require_running || a.status.eq_ignore_ascii_case("running"))
        .filter(|a| {
            exclude_pubkey
                .map(|ex| !a.pubkey.eq_ignore_ascii_case(ex))
                .unwrap_or(true)
        })
        .filter(|a| !a.pubkey.trim().is_empty())
        .map(|a| {
            let bal = balances
                .get(&a.pubkey)
                .cloned()
                .unwrap_or_else(|| empty_balance(&a.name));
            let norm_rep = normalize_reputation(bal.reputation);
            let stake = bal.balance.clamp(0, max_stake) as f64 / max_stake.max(1) as f64;
            let diversity = match prefer_unlike_caps {
                Some(other) if !other.is_empty() => {
                    let overlap = a.capabilities.iter().any(|c| {
                        other.iter().any(|o| o.eq_ignore_ascii_case(c))
                    });
                    if overlap {
                        0.35
                    } else {
                        1.0
                    }
                }
                _ => 1.0,
            };
            let score = W_REP * norm_rep + W_STAKE * stake + W_DIVERSITY * diversity;
            AuctionCandidate {
                name: a.name.clone(),
                pubkey: a.pubkey.clone(),
                capabilities: a.capabilities.clone(),
                score,
                mismatch: false,
                reason: format!(
                    "open rep_n={norm_rep:.2} stake_n={stake:.2} div={diversity:.2} bal={}",
                    bal.balance
                ),
            }
        })
        .collect();
    if scored.is_empty() {
        return None;
    }
    scored.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                left.pubkey
                    .to_ascii_lowercase()
                    .cmp(&right.pubkey.to_ascii_lowercase())
            })
    });
    scored.into_iter().next()
}

/// Like [`pick_auction_winner`], but scores each agent by their **best** match
/// across several required capabilities (reputation / stake still apply).
pub fn pick_auction_winner_best_of(
    agents: &[RosterAgent],
    required: &[&str],
    balances: &BTreeMap<String, AgentBalance>,
    max_stake: i64,
    exclude_pubkey: Option<&str>,
    require_running: bool,
) -> Option<AuctionCandidate> {
    let required: Vec<&str> = required
        .iter()
        .map(|c| c.trim())
        .filter(|c| !c.is_empty())
        .collect();
    if required.is_empty() {
        return None;
    }
    let mut scored: Vec<AuctionCandidate> = agents
        .iter()
        .filter(|a| !require_running || a.status.eq_ignore_ascii_case("running"))
        .filter(|a| {
            exclude_pubkey
                .map(|ex| !a.pubkey.eq_ignore_ascii_case(ex))
                .unwrap_or(true)
        })
        .filter(|a| !a.pubkey.is_empty() || !a.name.is_empty())
        .map(|a| {
            let bal = balances
                .get(&a.pubkey)
                .cloned()
                .unwrap_or_else(|| empty_balance(&a.name));
            let match_ratio = best_capability_match_ratio(&a.capabilities, &required);
            let mismatch = match_ratio < MATCH_FULL;
            let norm_rep = normalize_reputation(bal.reputation);
            let stake = bal.balance.clamp(0, max_stake) as f64 / max_stake.max(1) as f64;
            let score = W_CAPABILITY * match_ratio + W_REPUTATION * norm_rep + W_STAKE * stake;
            AuctionCandidate {
                name: a.name.clone(),
                pubkey: a.pubkey.clone(),
                capabilities: a.capabilities.clone(),
                score,
                mismatch,
                reason: format!(
                    "cap={match_ratio:.2} rep_n={norm_rep:.2} stake_n={stake:.2} bal={}",
                    bal.balance
                ),
            }
        })
        .collect();
    if scored.is_empty() {
        return None;
    }
    scored.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                left.pubkey
                    .to_ascii_lowercase()
                    .cmp(&right.pubkey.to_ascii_lowercase())
            })
            .then_with(|| {
                left.name
                    .to_ascii_lowercase()
                    .cmp(&right.name.to_ascii_lowercase())
            })
    });
    scored.into_iter().next()
}

pub fn best_capability_match_ratio(capabilities: &[String], required: &[&str]) -> f64 {
    required
        .iter()
        .map(|c| capability_match_ratio(capabilities, c))
        .fold(MATCH_MISMATCH, f64::max)
}

pub fn capability_match_ratio(capabilities: &[String], query: &str) -> f64 {
    let query = query.trim();
    if query.is_empty() {
        return MATCH_MISMATCH;
    }
    if capability_matches(capabilities, query) {
        if capabilities.iter().any(|c| c == query) {
            MATCH_FULL
        } else {
            MATCH_PARTIAL_PREFIX
        }
    } else {
        MATCH_MISMATCH
    }
}

pub fn capability_matches(capabilities: &[String], query: &str) -> bool {
    let q = query.trim();
    if q.is_empty() {
        return false;
    }
    capabilities.iter().any(|c| {
        c == q || (q.ends_with('.') && c.starts_with(q)) || (c.starts_with(&format!("{q}.")))
    })
}

fn normalize_reputation(rep: i32) -> f64 {
    let r = rep.max(0) as f64;
    r / (r + 500.0)
}

fn empty_balance(name: &str) -> AgentBalance {
    AgentBalance {
        balance: STARTING_BALANCE,
        reputation: STARTING_REPUTATION,
        name: name.to_string(),
        tags: Vec::new(),
        achievements: Default::default(),
        capability_grants: Default::default(),
    }
}

pub fn latest_contract(
    paths: &EconomyPaths,
    contract_id: &str,
) -> Result<Option<ContractRecord>, EconomyError> {
    let all = load_contracts(paths)?;
    let mut matches: Vec<ContractRecord> = all
        .into_iter()
        .filter(|c| c.contract_id == contract_id)
        .collect();
    matches.sort_by(|a, b| b.ts.cmp(&a.ts));
    Ok(matches.into_iter().next())
}

pub fn load_contracts(paths: &EconomyPaths) -> Result<Vec<ContractRecord>, EconomyError> {
    load_contracts_at(&paths.contracts)
}

pub fn load_contracts_at(path: &Path) -> Result<Vec<ContractRecord>, EconomyError> {
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(EconomyError::io("open economy contracts", path, e)),
    };
    let lines: Vec<String> = BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter(|l| !l.trim().is_empty())
        .collect();
    let start = lines.len().saturating_sub(MAX_SCAN_LINES);
    let mut out = Vec::new();
    for line in &lines[start..] {
        if let Ok(record) = serde_json::from_str::<ContractRecord>(line) {
            out.push(record);
        }
    }
    Ok(out)
}

fn resolve_executor_contract(
    paths: &EconomyPaths,
    leaf: &ContractRecord,
) -> Result<ContractRecord, EconomyError> {
    if leaf.status != "subcontracted" {
        return Ok(leaf.clone());
    }
    let children = load_contracts(paths)?;
    let mut descendants: Vec<ContractRecord> = children
        .into_iter()
        .filter(|c| {
            c.parent_contract_id
                .as_deref()
                .is_some_and(|p| p == leaf.contract_id)
                && (c.status == "awarded" || c.status == "subcontracted")
        })
        .collect();
    descendants.sort_by(|a, b| b.ts.cmp(&a.ts));
    if let Some(child) = descendants.first() {
        return resolve_executor_contract(paths, child);
    }
    Ok(leaf.clone())
}

fn new_contract_id() -> String {
    format!(
        "c-{}-{:x}",
        Utc::now().timestamp_millis(),
        (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0))
    )
}
