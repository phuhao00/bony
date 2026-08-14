use crate::auction::capability_matches;
use crate::chain::{append_chained, ChainedRow};
use crate::error::EconomyError;
use crate::ledger::fold_ledger;
use crate::org::{fold_orgs, org_member_capabilities};
use crate::paths::EconomyPaths;
use crate::types::{BidderKind, ContractRecord, RosterAgent};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TenderRecord {
    pub ts: String,
    pub tender_id: String,
    pub capability: String,
    pub budget: i64,
    pub task_ref: String,
    pub title: String,
    /// open | resolved | cancelled
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub winner_pubkey: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub winner_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract_id: Option<String>,
    /// Winning agent's task answer / deliverable text (e.g. `2` for `1+1=?`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    /// Soft labels for the tender itself (capability / topic).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reward_gold: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reward_reputation: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reward_tier: Option<String>,
    /// Settlement quality grade: excellent | pass | thin | fail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reward_grade: Option<String>,
    /// Human-readable schedule breakdown (from [`crate::reward::compute_settlement_reward`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reward_note: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reward_tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reward_achievements: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reward_capabilities: Vec<String>,
    /// Final bid board + winner rationale (filled at resolve).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allocation: Option<crate::quote::AllocationDecision>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

impl ChainedRow for TenderRecord {
    fn prev_hash(&self) -> Option<&str> {
        self.prev_hash.as_deref()
    }
    fn hash(&self) -> Option<&str> {
        self.hash.as_deref()
    }
    fn set_prev_hash(&mut self, v: Option<String>) {
        self.prev_hash = v;
    }
    fn set_hash(&mut self, v: Option<String>) {
        self.hash = v;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BidRecord {
    pub ts: String,
    pub tender_id: String,
    pub bidder_pubkey: String,
    pub bidder_name: String,
    pub bidder_kind: BidderKind,
    /// Bond / reserved stake (usually equals quote).
    pub stake: i64,
    /// Asking price for this tender (stats-based).
    #[serde(default)]
    pub quote: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote_basis: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reputation_at_bid: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

impl ChainedRow for BidRecord {
    fn prev_hash(&self) -> Option<&str> {
        self.prev_hash.as_deref()
    }
    fn hash(&self) -> Option<&str> {
        self.hash.as_deref()
    }
    fn set_prev_hash(&mut self, v: Option<String>) {
        self.prev_hash = v;
    }
    fn set_hash(&mut self, v: Option<String>) {
        self.hash = v;
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TenderSnapshot {
    pub tender_id: String,
    pub title: String,
    pub capability: String,
    pub budget: i64,
    pub task_ref: String,
    pub status: String,
    pub winner_pubkey: Option<String>,
    pub winner_name: Option<String>,
    pub contract_id: Option<String>,
    pub outcome: Option<String>,
    pub tags: Vec<String>,
    pub reward_gold: Option<i64>,
    pub reward_reputation: Option<i32>,
    pub reward_tier: Option<String>,
    pub reward_grade: Option<String>,
    pub reward_note: Option<String>,
    pub reward_tags: Vec<String>,
    pub reward_achievements: Vec<String>,
    pub reward_capabilities: Vec<String>,
    pub allocation: Option<crate::quote::AllocationDecision>,
    pub bids: Vec<BidRecord>,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TenderPublishParams {
    pub title: String,
    /// Empty → inferred from title via [`suggest_tender_fields`].
    #[serde(default)]
    pub capability: String,
    /// `<= 0` → inferred from title via [`suggest_tender_fields`].
    #[serde(default)]
    pub budget: i64,
    pub task_ref: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TenderSuggestion {
    pub capability: String,
    pub budget: i64,
    pub reason: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TenderClearMode {
    /// Resolved tenders that never got an outcome (+ optional open leftovers).
    Stuck,
    /// All resolved / cancelled history rows.
    History,
    /// Everything currently listed (open + resolved + cancelled).
    All,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TenderBidParams {
    pub tender_id: String,
    pub bidder_pubkey: String,
    pub bidder_name: String,
    pub bidder_kind: Option<BidderKind>,
    pub stake: Option<i64>,
    pub quote: Option<i64>,
    pub quote_basis: Option<String>,
    pub reputation_at_bid: Option<i32>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TenderResolveParams {
    pub tender_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TenderListParams {
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TenderInviteResult {
    pub tender: TenderSnapshot,
    pub invited: usize,
    pub skipped_existing: usize,
    /// True when the market round finished with a winner after invite.
    pub auto_resolved: bool,
}

pub fn publish_tender(
    paths: &EconomyPaths,
    p: TenderPublishParams,
) -> Result<TenderSnapshot, EconomyError> {
    let title = p.title.trim();
    let task_ref = p.task_ref.trim();
    if title.is_empty() || task_ref.is_empty() {
        return Err(EconomyError::invalid(
            "title and task_ref must not be empty",
        ));
    }
    let suggestion = suggest_tender_fields(title);
    let capability = {
        let trimmed = p.capability.trim();
        if trimmed.is_empty() {
            suggestion.capability
        } else {
            trimmed.to_string()
        }
    };
    let budget = if p.budget <= 0 {
        suggestion.budget
    } else {
        p.budget
    };
    // Empty is normalized to open market.
    let capability = if is_open_capability(&capability) {
        OPEN_CAPABILITY.to_string()
    } else {
        capability
    };
    if budget <= 0 {
        return Err(EconomyError::invalid("budget must be a positive integer"));
    }
    let tender_id = format!("t-{}-{:x}", Utc::now().timestamp_millis(), fastrand_u32());
    let ts = Utc::now().to_rfc3339();
    let tags = tags_for_tender(&capability, title);
    let record = TenderRecord {
        ts: ts.clone(),
        tender_id: tender_id.clone(),
        capability: capability.clone(),
        budget,
        task_ref: task_ref.to_string(),
        title: title.to_string(),
        status: "open".into(),
        winner_pubkey: None,
        winner_name: None,
        contract_id: None,
        outcome: None,
        tags: tags.clone(),
        reward_gold: None,
        reward_reputation: None,
        reward_tier: None,
        reward_grade: None,
        reward_note: None,
        reward_tags: Vec::new(),
        reward_achievements: Vec::new(),
        reward_capabilities: Vec::new(),
        allocation: None,
        prev_hash: None,
        hash: None,
    };
    append_chained(&paths.tenders, &record)?;
    Ok(TenderSnapshot {
        tender_id,
        title: title.to_string(),
        capability,
        budget,
        task_ref: task_ref.to_string(),
        status: "open".into(),
        winner_pubkey: None,
        winner_name: None,
        contract_id: None,
        outcome: None,
        tags,
        reward_gold: None,
        reward_reputation: None,
        reward_tier: None,
        reward_grade: None,
        reward_note: None,
        reward_tags: Vec::new(),
        reward_achievements: Vec::new(),
        reward_capabilities: Vec::new(),
        allocation: None,
        bids: Vec::new(),
        created_at: ts,
    })
}

/// Open-market tender — any running agent may bid; resolve uses reputation/stake.
pub const OPEN_CAPABILITY: &str = "open";

fn is_open_capability(capability: &str) -> bool {
    let c = capability.trim();
    c.is_empty() || c.eq_ignore_ascii_case(OPEN_CAPABILITY) || c == "*"
}

/// Default market fields for a free-text title.
///
/// Does **not** hard-route to named agents or keyword→capability maps.
/// Callers may still pass an explicit capability to narrow the invite set.
pub fn suggest_tender_fields(title: &str) -> TenderSuggestion {
    let raw = title.trim();
    let len_bonus = (raw.chars().count() / 40) as i64 * 5;
    let budget = (40 + len_bonus).clamp(10, 200);
    TenderSuggestion {
        capability: OPEN_CAPABILITY.to_string(),
        budget,
        reason: "open market — quotes + reputation + stake pick the winner".into(),
        tags: tags_for_tender(OPEN_CAPABILITY, raw),
    }
}

/// Soft labels for UI chips. No routing decisions.
pub fn tags_for_tender(capability: &str, title: &str) -> Vec<String> {
    let mut tags = Vec::new();
    if is_open_capability(capability) {
        push_unique(&mut tags, "open");
    } else if !capability.trim().is_empty() {
        push_unique(&mut tags, capability.trim());
    }
    if title.contains('?') || title.contains('？') {
        push_unique(&mut tags, "question");
    }
    tags
}

fn push_unique(tags: &mut Vec<String>, tag: &str) {
    if !tags.iter().any(|t| t.eq_ignore_ascii_case(tag)) {
        tags.push(tag.to_string());
    }
}

/// Performance titles granted only when settlement grade is Pass+.
pub fn performance_tags_for_capability(capability: &str) -> Vec<String> {
    vec![crate::reward::title_for_capability(capability).to_string()]
}

pub fn tender_bid(paths: &EconomyPaths, p: TenderBidParams) -> Result<BidRecord, EconomyError> {
    let tender_id = p.tender_id.trim();
    let bidder = p.bidder_pubkey.trim();
    if tender_id.is_empty() || bidder.is_empty() {
        return Err(EconomyError::invalid(
            "tender_id and bidder_pubkey must not be empty",
        ));
    }
    let snap = latest_tender(paths, tender_id)?
        .ok_or_else(|| EconomyError::invalid(format!("unknown tender {tender_id}")))?;
    if snap.status != "open" {
        return Err(EconomyError::invalid(format!(
            "tender {tender_id} is not open ({})",
            snap.status
        )));
    }
    let quote = p.quote.unwrap_or_else(|| p.stake.unwrap_or(0)).max(0);
    let bid = BidRecord {
        ts: Utc::now().to_rfc3339(),
        tender_id: tender_id.to_string(),
        bidder_pubkey: bidder.to_string(),
        bidder_name: p.bidder_name.trim().to_string(),
        bidder_kind: p.bidder_kind.unwrap_or(if bidder.starts_with("org:") {
            BidderKind::Org
        } else {
            BidderKind::Agent
        }),
        stake: p.stake.unwrap_or(quote).max(0),
        quote,
        quote_basis: p.quote_basis,
        reputation_at_bid: p.reputation_at_bid,
        note: p.note,
        prev_hash: None,
        hash: None,
    };
    // Bids share the tenders file as chained events with a marker field.
    // Store as TenderRecord-compatible envelope via BidRecord on same file
    // using a wrapper: we append BidRecord lines; fold ignores non-tender kinds
    // by detecting missing tender fields via a tagged enum would be cleaner —
    // here we use a side channel: bid lines live in the same JSONL and are
    // distinguished by presence of `bidder_pubkey` without `capability`.
    append_chained(&paths.tenders, &bid)?;
    Ok(bid)
}

/// Invite capability-matching agents (and orgs) to bid on an open tender.
///
/// Skips empty pubkeys, agents that already bid, and agents/orgs whose
/// declared capabilities do not match the tender. Stake defaults to
/// `max(1, budget / 10)`. Idempotent for already-bidding invitees.
pub fn invite_tender_bids_by_capability(
    paths: &EconomyPaths,
    tender_id: &str,
    agents: &[RosterAgent],
) -> Result<TenderInviteResult, EconomyError> {
    let tender_id = tender_id.trim();
    if tender_id.is_empty() {
        return Err(EconomyError::invalid("tender_id must not be empty"));
    }
    let tender = latest_tender(paths, tender_id)?
        .ok_or_else(|| EconomyError::invalid(format!("unknown tender {tender_id}")))?;
    if tender.status != "open" {
        return Err(EconomyError::invalid(format!(
            "tender {tender_id} is not open ({})",
            tender.status
        )));
    }

    let existing = bids_for(paths, tender_id)?;
    let mut already: BTreeMap<String, ()> = BTreeMap::new();
    for bid in &existing {
        already.insert(bid.bidder_pubkey.to_ascii_lowercase(), ());
    }

    let balances = fold_ledger(paths)?;
    let payout_stats = crate::quote::fold_payout_stats(paths)?;
    let open = is_open_capability(&tender.capability);
    let note = format!("auto-invite:{}", tender.capability);
    let mut invited = 0usize;
    let mut skipped_existing = 0usize;

    for agent in agents {
        let pubkey = agent.pubkey.trim();
        if pubkey.is_empty() {
            continue;
        }
        if !open && !capability_matches(&agent.capabilities, &tender.capability) {
            continue;
        }
        let key = pubkey.to_ascii_lowercase();
        if already.contains_key(&key) {
            skipped_existing += 1;
            continue;
        }
        let bal = balances.get(pubkey).cloned().unwrap_or_else(|| {
            crate::ledger::AgentBalance {
                balance: crate::ledger::STARTING_BALANCE,
                reputation: crate::ledger::STARTING_REPUTATION,
                name: agent.name.clone(),
                tags: Vec::new(),
                achievements: Default::default(),
                capability_grants: Default::default(),
            }
        });
        let stats = payout_stats.get(pubkey).cloned().unwrap_or_default();
        let quoted = crate::quote::compute_agent_quote(tender.budget, &bal, &stats);
        tender_bid(
            paths,
            TenderBidParams {
                tender_id: tender_id.to_string(),
                bidder_pubkey: pubkey.to_string(),
                bidder_name: agent.name.clone(),
                bidder_kind: Some(BidderKind::Agent),
                stake: Some(quoted.quote),
                quote: Some(quoted.quote),
                quote_basis: Some(quoted.basis),
                reputation_at_bid: Some(bal.reputation),
                note: Some(note.clone()),
            },
        )?;
        already.insert(key, ());
        invited += 1;
    }

    let orgs = fold_orgs(paths).unwrap_or_default();
    for org in orgs.values() {
        let caps = org_member_capabilities(org, agents);
        if !open && !capability_matches(&caps, &tender.capability) {
            continue;
        }
        let key = org.org_id.to_ascii_lowercase();
        if already.contains_key(&key) {
            skipped_existing += 1;
            continue;
        }
        let bal = balances.get(&org.org_id).cloned().unwrap_or_else(|| {
            crate::ledger::AgentBalance {
                balance: crate::ledger::STARTING_BALANCE,
                reputation: crate::ledger::STARTING_REPUTATION,
                name: org.name.clone(),
                tags: Vec::new(),
                achievements: Default::default(),
                capability_grants: Default::default(),
            }
        });
        let stats = payout_stats.get(&org.org_id).cloned().unwrap_or_default();
        let quoted = crate::quote::compute_agent_quote(tender.budget, &bal, &stats);
        tender_bid(
            paths,
            TenderBidParams {
                tender_id: tender_id.to_string(),
                bidder_pubkey: org.org_id.clone(),
                bidder_name: org.name.clone(),
                bidder_kind: Some(BidderKind::Org),
                stake: Some(quoted.quote),
                quote: Some(quoted.quote),
                quote_basis: Some(quoted.basis),
                reputation_at_bid: Some(bal.reputation),
                note: Some(note.clone()),
            },
        )?;
        already.insert(key, ());
        invited += 1;
    }

    let snap = tender_snapshot(paths, &tender)?;
    Ok(TenderInviteResult {
        tender: snap,
        invited,
        skipped_existing,
        auto_resolved: false,
    })
}

/// Invite matching agents, then immediately resolve if any bids exist.
pub fn complete_open_tender(
    paths: &EconomyPaths,
    tender_id: &str,
    agents: &[RosterAgent],
) -> Result<TenderInviteResult, EconomyError> {
    let mut invited = invite_tender_bids_by_capability(paths, tender_id, agents)?;
    if invited.tender.bids.is_empty() {
        return Ok(invited);
    }
    if invited.tender.status == "resolved" {
        invited.auto_resolved = true;
        return Ok(invited);
    }
    let resolved = resolve_tender(
        paths,
        agents,
        TenderResolveParams {
            tender_id: invited.tender.tender_id.clone(),
        },
    )?;
    Ok(TenderInviteResult {
        tender: resolved,
        invited: invited.invited,
        skipped_existing: invited.skipped_existing,
        auto_resolved: true,
    })
}

/// Publish → auto-invite → auto-resolve (when at least one bidder matched).
pub fn publish_tender_with_invite(
    paths: &EconomyPaths,
    p: TenderPublishParams,
    agents: &[RosterAgent],
) -> Result<TenderInviteResult, EconomyError> {
    let published = publish_tender(paths, p)?;
    complete_open_tender(paths, &published.tender_id, agents)
}

/// Finish every open tender that can collect bids (invite + resolve).
pub fn sweep_open_tenders(
    paths: &EconomyPaths,
    agents: &[RosterAgent],
) -> Result<Vec<TenderSnapshot>, EconomyError> {
    let open = list_tenders(
        paths,
        TenderListParams {
            status: Some("open".into()),
        },
    )?;
    let mut finished = Vec::new();
    for tender in open {
        let result = complete_open_tender(paths, &tender.tender_id, agents)?;
        if result.auto_resolved {
            finished.push(result.tender);
        }
    }
    Ok(finished)
}

pub fn resolve_tender(
    paths: &EconomyPaths,
    agents: &[RosterAgent],
    p: TenderResolveParams,
) -> Result<TenderSnapshot, EconomyError> {
    let tender_id = p.tender_id.trim();
    let tender = latest_tender(paths, tender_id)?
        .ok_or_else(|| EconomyError::invalid(format!("unknown tender {tender_id}")))?;
    if tender.status != "open" {
        return Err(EconomyError::invalid(format!(
            "tender {tender_id} is not open"
        )));
    }
    let bids = bids_for(paths, tender_id)?;
    if bids.is_empty() {
        return Err(EconomyError::invalid(format!(
            "tender {tender_id} has no bids"
        )));
    }

    let balances = fold_ledger(paths)?;
    let orgs = fold_orgs(paths).unwrap_or_default();
    let open = is_open_capability(&tender.capability);

    let mut scored_rows: Vec<crate::quote::AllocationBid> = Vec::new();
    for bid in &bids {
        let caps = match bid.bidder_kind {
            BidderKind::Org => orgs
                .get(&bid.bidder_pubkey)
                .map(|o| org_member_capabilities(o, agents))
                .unwrap_or_default(),
            BidderKind::Agent => agents
                .iter()
                .find(|a| a.pubkey.eq_ignore_ascii_case(&bid.bidder_pubkey))
                .map(|a| a.capabilities.clone())
                .unwrap_or_default(),
        };
        let bal = balances
            .get(&bid.bidder_pubkey)
            .cloned()
            .unwrap_or_else(|| crate::ledger::AgentBalance {
                balance: crate::ledger::STARTING_BALANCE,
                reputation: bid.reputation_at_bid.unwrap_or(0),
                name: bid.bidder_name.clone(),
                tags: Vec::new(),
                achievements: Default::default(),
                capability_grants: Default::default(),
            });
        let quote = if bid.quote > 0 {
            bid.quote
        } else if bid.stake > 0 {
            bid.stake
        } else {
            (tender.budget / 2).max(1)
        };
        let cap_match = if open {
            1.0
        } else {
            crate::auction::capability_match_ratio(&caps, &tender.capability)
        };
        let (score, reason) = crate::quote::score_quote_bid(
            tender.budget,
            quote,
            bal.reputation,
            bal.balance,
            cap_match,
        );
        let reason = if let Some(basis) = bid.quote_basis.as_deref() {
            format!("{reason} · {basis}")
        } else {
            reason
        };
        scored_rows.push(crate::quote::AllocationBid {
            bidder_name: bid.bidder_name.clone(),
            bidder_pubkey: bid.bidder_pubkey.clone(),
            quote,
            reputation: bal.reputation,
            score,
            reason,
            won: false,
        });
    }

    let allocation = crate::quote::decide_allocation(tender.budget, scored_rows)
        .ok_or_else(|| EconomyError::invalid("could not pick a winner from bids"))?;
    let winner_pubkey = allocation.winner_pubkey.clone();
    let winner_name = allocation.winner_name.clone();
    let winner_score = allocation
        .bids
        .iter()
        .find(|b| b.won)
        .map(|b| b.score)
        .unwrap_or(0.0);
    let winner_mismatch = !open
        && agents
            .iter()
            .find(|a| a.pubkey.eq_ignore_ascii_case(&winner_pubkey))
            .map(|a| {
                crate::auction::capability_match_ratio(&a.capabilities, &tender.capability)
                    < crate::auction::MATCH_FULL
            })
            .unwrap_or(true);

    let contract_id = format!("c-{}-{:x}", Utc::now().timestamp_millis(), fastrand_u32());
    let contract = ContractRecord {
        ts: Utc::now().to_rfc3339(),
        contract_id: contract_id.clone(),
        task_ref: tender.task_ref.clone(),
        capability: tender.capability.clone(),
        budget: tender.budget,
        winner_name: winner_name.clone(),
        winner_pubkey: winner_pubkey.clone(),
        effective_score: winner_score,
        mismatch: winner_mismatch,
        parent_contract_id: None,
        cut_bp: None,
        depth: 0,
        status: "awarded".into(),
        bidder_kind: Some(if winner_pubkey.starts_with("org:") {
            BidderKind::Org
        } else {
            BidderKind::Agent
        }),
        prev_hash: None,
        hash: None,
    };
    append_chained(&paths.contracts, &contract)?;

    let tags = if tender.tags.is_empty() {
        tags_for_tender(&tender.capability, &tender.title)
    } else {
        tender.tags.clone()
    };
    let resolved = TenderRecord {
        ts: Utc::now().to_rfc3339(),
        tender_id: tender.tender_id.clone(),
        capability: tender.capability.clone(),
        budget: tender.budget,
        task_ref: tender.task_ref.clone(),
        title: tender.title.clone(),
        status: "resolved".into(),
        winner_pubkey: Some(winner_pubkey.clone()),
        winner_name: Some(winner_name.clone()),
        contract_id: Some(contract_id.clone()),
        outcome: None,
        tags: tags.clone(),
        reward_gold: None,
        reward_reputation: None,
        reward_tier: None,
        reward_grade: None,
        reward_note: None,
        reward_tags: Vec::new(),
        reward_achievements: Vec::new(),
        reward_capabilities: Vec::new(),
        allocation: Some(allocation.clone()),
        prev_hash: None,
        hash: None,
    };
    append_chained(&paths.tenders, &resolved)?;

    Ok(TenderSnapshot {
        tender_id: tender.tender_id,
        title: tender.title,
        capability: tender.capability,
        budget: tender.budget,
        task_ref: tender.task_ref,
        status: "resolved".into(),
        winner_pubkey: Some(winner_pubkey),
        winner_name: Some(winner_name),
        contract_id: Some(contract_id),
        outcome: None,
        tags,
        reward_gold: None,
        reward_reputation: None,
        reward_tier: None,
        reward_grade: None,
        reward_note: None,
        reward_tags: Vec::new(),
        reward_achievements: Vec::new(),
        reward_capabilities: Vec::new(),
        allocation: Some(allocation),
        bids,
        created_at: tender.ts,
    })
}

pub fn list_tenders(
    paths: &EconomyPaths,
    p: TenderListParams,
) -> Result<Vec<TenderSnapshot>, EconomyError> {
    let status_filter = p
        .status
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase());
    let tenders = fold_tenders(paths)?;
    let mut out = Vec::new();
    for t in tenders.values() {
        if let Some(ref st) = status_filter {
            if t.status != *st {
                continue;
            }
        } else if t.status == "cancelled" {
            // Default market board hides cancelled rows.
            continue;
        }
        let bids = bids_for(paths, &t.tender_id)?;
        let allocation = t.allocation.clone().or_else(|| {
            rebuild_allocation_from_bids(t.budget, t.winner_pubkey.as_deref(), &bids)
        });
        let snap = TenderSnapshot {
            tender_id: t.tender_id.clone(),
            title: t.title.clone(),
            capability: t.capability.clone(),
            budget: t.budget,
            task_ref: t.task_ref.clone(),
            status: t.status.clone(),
            winner_pubkey: t.winner_pubkey.clone(),
            winner_name: t.winner_name.clone(),
            contract_id: t.contract_id.clone(),
            outcome: t.outcome.clone(),
            tags: if t.tags.is_empty() {
                tags_for_tender(&t.capability, &t.title)
            } else {
                t.tags.clone()
            },
            reward_gold: t.reward_gold,
            reward_reputation: t.reward_reputation,
            reward_tier: t.reward_tier.clone(),
            reward_grade: t.reward_grade.clone(),
            reward_note: t.reward_note.clone(),
            reward_tags: t.reward_tags.clone(),
            reward_achievements: t.reward_achievements.clone(),
            reward_capabilities: t.reward_capabilities.clone(),
            allocation,
            bids,
            created_at: t.ts.clone(),
        };
        out.push(snap);
    }
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(out)
}

fn fold_tenders(paths: &EconomyPaths) -> Result<BTreeMap<String, TenderRecord>, EconomyError> {
    let file = match fs::File::open(&paths.tenders) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(EconomyError::io("open tenders", &paths.tenders, e)),
    };
    let mut map: BTreeMap<String, TenderRecord> = BTreeMap::new();
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        // Bid lines have bidder_pubkey and no capability — skip for tender fold.
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
            if (v.get("bidder_pubkey").is_some() || v.get("bidderPubkey").is_some())
                && v.get("capability").is_none()
            {
                continue;
            }
        }
        let Ok(rec) = serde_json::from_str::<TenderRecord>(&line) else {
            continue;
        };
        map.insert(rec.tender_id.clone(), rec);
    }
    Ok(map)
}

fn latest_tender(
    paths: &EconomyPaths,
    tender_id: &str,
) -> Result<Option<TenderRecord>, EconomyError> {
    Ok(fold_tenders(paths)?.remove(tender_id))
}

fn tender_snapshot(
    paths: &EconomyPaths,
    tender: &TenderRecord,
) -> Result<TenderSnapshot, EconomyError> {
    let bids = bids_for(paths, &tender.tender_id)?;
    let allocation = tender.allocation.clone().or_else(|| {
        rebuild_allocation_from_bids(
            tender.budget,
            tender.winner_pubkey.as_deref(),
            &bids,
        )
    });
    Ok(TenderSnapshot {
        tender_id: tender.tender_id.clone(),
        title: tender.title.clone(),
        capability: tender.capability.clone(),
        budget: tender.budget,
        task_ref: tender.task_ref.clone(),
        status: tender.status.clone(),
        winner_pubkey: tender.winner_pubkey.clone(),
        winner_name: tender.winner_name.clone(),
        contract_id: tender.contract_id.clone(),
        outcome: tender.outcome.clone(),
        tags: if tender.tags.is_empty() {
            tags_for_tender(&tender.capability, &tender.title)
        } else {
            tender.tags.clone()
        },
        reward_gold: tender.reward_gold,
        reward_reputation: tender.reward_reputation,
        reward_tier: tender.reward_tier.clone(),
        reward_grade: tender.reward_grade.clone(),
        reward_note: tender.reward_note.clone(),
        reward_tags: tender.reward_tags.clone(),
        reward_achievements: tender.reward_achievements.clone(),
        reward_capabilities: tender.reward_capabilities.clone(),
        allocation,
        bids,
        created_at: tender.ts.clone(),
    })
}

/// When older tenders lack a stored allocation, rebuild a board from bid quotes
/// so the market UI can still show quote / score / winner rationale.
fn rebuild_allocation_from_bids(
    budget: i64,
    winner_pubkey: Option<&str>,
    bids: &[BidRecord],
) -> Option<crate::quote::AllocationDecision> {
    if bids.is_empty() {
        return None;
    }
    let mut rows = Vec::with_capacity(bids.len());
    for bid in bids {
        let quote = if bid.quote > 0 {
            bid.quote
        } else if bid.stake > 0 {
            bid.stake
        } else {
            (budget / 2).max(1)
        };
        let reputation = bid.reputation_at_bid.unwrap_or(0);
        let (score, reason) = crate::quote::score_quote_bid(
            budget,
            quote,
            reputation,
            quote.max(1),
            1.0,
        );
        let reason = if let Some(basis) = bid.quote_basis.as_deref() {
            format!("{reason} · {basis}")
        } else {
            reason
        };
        rows.push(crate::quote::AllocationBid {
            bidder_name: bid.bidder_name.clone(),
            bidder_pubkey: bid.bidder_pubkey.clone(),
            quote,
            reputation,
            score,
            reason,
            won: false,
        });
    }
    let mut decision = crate::quote::decide_allocation(budget, rows)?;
    if let Some(wpk) = winner_pubkey.map(str::trim).filter(|s| !s.is_empty()) {
        let mut matched = false;
        for row in &mut decision.bids {
            row.won = row.bidder_pubkey.eq_ignore_ascii_case(wpk);
            if row.won {
                matched = true;
            }
        }
        if matched {
            if let Some(w) = decision.bids.iter().find(|b| b.won) {
                decision.winner_name = w.bidder_name.clone();
                decision.winner_pubkey = w.bidder_pubkey.clone();
                decision.winner_quote = w.quote;
                decision.note = format!(
                    "中标 @{} · 报价¤{} / 预算¤{} · {}",
                    w.bidder_name, w.quote, budget, w.reason
                );
            }
        }
    }
    Some(decision)
}

fn merge_agent_tags(
    paths: &EconomyPaths,
    pubkey: &str,
    name: Option<&str>,
    extra: &[String],
) -> Result<Vec<String>, EconomyError> {
    if extra.is_empty() {
        return Ok(Vec::new());
    }
    let balances = fold_ledger(paths)?;
    let mut tags = balances
        .get(pubkey)
        .map(|b| b.tags.clone())
        .unwrap_or_default();
    let mut granted = Vec::new();
    for tag in extra {
        let trimmed = tag.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !tags.iter().any(|t| t.eq_ignore_ascii_case(trimmed)) {
            tags.push(trimmed.to_string());
            granted.push(trimmed.to_string());
        }
    }
    if granted.is_empty() {
        return Ok(granted);
    }
    crate::admin::set_tags(paths, pubkey, name, tags, Some("tender performance title"))?;
    Ok(granted)
}

/// Soft-delete a tender from the market board (`status = cancelled`).
pub fn cancel_tender(
    paths: &EconomyPaths,
    tender_id: &str,
) -> Result<TenderSnapshot, EconomyError> {
    let tender_id = tender_id.trim();
    if tender_id.is_empty() {
        return Err(EconomyError::invalid("tender_id must not be empty"));
    }
    let tender = latest_tender(paths, tender_id)?
        .ok_or_else(|| EconomyError::invalid(format!("unknown tender {tender_id}")))?;
    if tender.status == "cancelled" {
        return tender_snapshot(paths, &tender);
    }
    let cancelled = TenderRecord {
        ts: Utc::now().to_rfc3339(),
        status: "cancelled".into(),
        prev_hash: None,
        hash: None,
        ..tender.clone()
    };
    append_chained(&paths.tenders, &cancelled)?;
    tender_snapshot(paths, &cancelled)
}

/// Cancel many tenders matching a cleanup mode. Returns how many were newly cancelled.
pub fn clear_tenders(
    paths: &EconomyPaths,
    mode: TenderClearMode,
) -> Result<usize, EconomyError> {
    let all = fold_tenders(paths)?;
    let mut count = 0usize;
    for tender in all.values() {
        if tender.status == "cancelled" {
            continue;
        }
        let has_outcome = tender
            .outcome
            .as_deref()
            .map(str::trim)
            .is_some_and(|s| !s.is_empty());
        let should = match mode {
            TenderClearMode::Stuck => {
                tender.status == "open" || (tender.status == "resolved" && !has_outcome)
            }
            TenderClearMode::History => tender.status == "resolved",
            TenderClearMode::All => tender.status == "open" || tender.status == "resolved",
        };
        if !should {
            continue;
        }
        cancel_tender(paths, &tender.tender_id)?;
        count += 1;
    }
    Ok(count)
}

/// Persist the winning agent's deliverable text on a resolved tender, then
/// settle the awarded contract (payout / penalty) using that text as the note.
pub fn record_tender_outcome(
    paths: &EconomyPaths,
    tender_id: &str,
    outcome: &str,
    success: bool,
) -> Result<TenderSnapshot, EconomyError> {
    let tender_id = tender_id.trim();
    let outcome = outcome.trim();
    if tender_id.is_empty() {
        return Err(EconomyError::invalid("tender_id must not be empty"));
    }
    if outcome.is_empty() {
        return Err(EconomyError::invalid("outcome must not be empty"));
    }
    let tender = latest_tender(paths, tender_id)?
        .ok_or_else(|| EconomyError::invalid(format!("unknown tender {tender_id}")))?;
    if tender.status != "resolved" {
        return Err(EconomyError::invalid(format!(
            "tender {tender_id} is not resolved"
        )));
    }
    let contract_id = tender
        .contract_id
        .clone()
        .ok_or_else(|| EconomyError::invalid(format!("tender {tender_id} has no contract")))?;

    let settled = crate::auction::settle(
        paths,
        crate::auction::SettleParams {
            contract_id,
            status: if success {
                "success".into()
            } else {
                "failed".into()
            },
            quality_note: Some(outcome.to_string()),
        },
    )?;

    let reward_tags = if settled.grant_title {
        let tags = performance_tags_for_capability(&tender.capability);
        merge_agent_tags(
            paths,
            tender.winner_pubkey.as_deref().unwrap_or(""),
            tender.winner_name.as_deref(),
            &tags,
        )?
    } else {
        Vec::new()
    };

    let tags = if tender.tags.is_empty() {
        tags_for_tender(&tender.capability, &tender.title)
    } else {
        tender.tags.clone()
    };
    let updated = TenderRecord {
        ts: Utc::now().to_rfc3339(),
        tender_id: tender.tender_id.clone(),
        capability: tender.capability.clone(),
        budget: tender.budget,
        task_ref: tender.task_ref.clone(),
        title: tender.title.clone(),
        status: tender.status.clone(),
        winner_pubkey: tender.winner_pubkey.clone(),
        winner_name: tender.winner_name.clone(),
        contract_id: tender.contract_id.clone(),
        outcome: Some(outcome.to_string()),
        tags,
        reward_gold: Some(settled.paid_gold),
        reward_reputation: Some(settled.reputation_delta),
        reward_tier: Some(settled.tier),
        reward_grade: Some(settled.quality_grade),
        reward_note: Some(settled.note),
        reward_tags,
        reward_achievements: settled.new_achievements,
        reward_capabilities: settled.new_capabilities,
        allocation: tender.allocation.clone(),
        prev_hash: None,
        hash: None,
    };
    append_chained(&paths.tenders, &updated)?;

    tender_snapshot(paths, &updated)
}

fn bids_for(paths: &EconomyPaths, tender_id: &str) -> Result<Vec<BidRecord>, EconomyError> {
    let file = match fs::File::open(&paths.tenders) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(EconomyError::io("open tenders", &paths.tenders, e)),
    };
    let mut out = Vec::new();
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(bid) = serde_json::from_str::<BidRecord>(&line) else {
            continue;
        };
        // TenderRecord also deserializes partially — require bidder fields present
        // which BidRecord has; TenderRecord won't have bidder_pubkey in schema
        // but serde default would make empty. Filter by tender_id + non-empty bidder.
        if bid.tender_id == tender_id && !bid.bidder_pubkey.is_empty() {
            out.push(bid);
        }
    }
    Ok(out)
}

fn fastrand_u32() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn agent(name: &str, pubkey: &str, caps: &[&str]) -> RosterAgent {
        RosterAgent {
            name: name.into(),
            pubkey: pubkey.into(),
            capabilities: caps.iter().map(|c| (*c).to_string()).collect(),
            status: "running".into(),
        }
    }

    #[test]
    fn publish_invites_matching_agents_then_resolve() {
        let dir = tempdir().unwrap();
        let paths = EconomyPaths::from_root(dir.path());
        let agents = vec![
            agent("ZeroClaw", "pk-zc", &["research.web"]),
            agent("Scout", "pk-scout", &["other.unmatched.cap"]),
            agent("DocSmith", "pk-doc", &["document.create"]),
        ];

        let invited = publish_tender_with_invite(
            &paths,
            TenderPublishParams {
                title: "1=1=?".into(),
                capability: "research.web".into(),
                budget: 40,
                task_ref: "ui-test".into(),
            },
            &agents,
        )
        .unwrap();

        assert!(invited.auto_resolved);
        assert_eq!(invited.invited, 1);
        assert_eq!(invited.tender.status, "resolved");
        assert_eq!(invited.tender.winner_name.as_deref(), Some("ZeroClaw"));
        assert_eq!(invited.tender.bids.len(), 1);
        assert_eq!(invited.tender.bids[0].bidder_name, "ZeroClaw");
        let quote = invited.tender.bids[0].quote;
        assert!(quote >= 1 && quote <= 40);
        assert_eq!(invited.tender.bids[0].stake, quote);
        assert!(invited.tender.contract_id.is_some());
        assert!(invited.tender.allocation.is_some());
        let alloc = invited.tender.allocation.as_ref().unwrap();
        assert_eq!(alloc.winner_name, "ZeroClaw");
        assert_eq!(alloc.winner_quote, quote);
    }

    #[test]
    fn open_market_invites_every_running_agent() {
        let dir = tempdir().unwrap();
        let paths = EconomyPaths::from_root(dir.path());
        let agents = vec![
            agent("ZeroClaw", "pk-zc", &["research.web"]),
            agent("DocSmith", "pk-doc", &["document.create"]),
            agent("Scout", "pk-scout", &["research.web"]),
        ];
        crate::admin::adjust_reputation(&paths, "pk-doc", Some("DocSmith"), 90, Some("rep"))
            .unwrap();
        let invited = publish_tender_with_invite(
            &paths,
            TenderPublishParams {
                title: "今天AI资讯PDF".into(),
                capability: String::new(),
                budget: 0,
                task_ref: "news-pdf".into(),
            },
            &agents,
        )
        .unwrap();
        assert_eq!(invited.tender.capability, OPEN_CAPABILITY);
        assert_eq!(invited.invited, 3);
        assert_eq!(invited.tender.winner_name.as_deref(), Some("DocSmith"));
    }

    #[test]
    fn hire_support_open_picks_by_reputation() {
        let dir = tempdir().unwrap();
        let paths = EconomyPaths::from_root(dir.path());
        let agents = vec![
            agent("DocSmith", "pk-doc", &["document.create"]),
            agent("ZeroClaw", "pk-zc", &["research.web"]),
            agent("ScoutB", "pk-scout-b", &["research.web"]),
        ];
        crate::admin::adjust_balance(&paths, "pk-doc", Some("DocSmith"), 100, Some("seed"))
            .unwrap();
        crate::admin::adjust_reputation(&paths, "pk-zc", Some("ZeroClaw"), 80, Some("rep"))
            .unwrap();
        crate::admin::adjust_reputation(&paths, "pk-scout-b", Some("ScoutB"), 10, Some("rep"))
            .unwrap();

        let hired = crate::auction::hire_support(
            &paths,
            &agents,
            crate::auction::HireSupportParams {
                payer_pubkey: "pk-doc".into(),
                payer_name: Some("DocSmith".into()),
                capability: "open".into(),
                task_ref: "hire-1".into(),
                max_pay: 20,
            },
        )
        .unwrap();
        assert_eq!(hired.hiree_name, "ZeroClaw");
        assert_eq!(hired.paid, 20);
        let bal = crate::ledger::fold_ledger(&paths).unwrap();
        // STARTING_BALANCE(100) + seed(100) - hire(20)
        assert_eq!(bal.get("pk-doc").map(|b| b.balance), Some(180));
        assert!(bal.get("pk-zc").map(|b| b.balance).unwrap_or(0) >= 120);
    }

    #[test]
    fn invite_skips_when_no_capability_match() {
        let dir = tempdir().unwrap();
        let paths = EconomyPaths::from_root(dir.path());
        let published = publish_tender(
            &paths,
            TenderPublishParams {
                title: "scene".into(),
                capability: "other.unmatched.cap".into(),
                budget: 10,
                task_ref: "t1".into(),
            },
        )
        .unwrap();
        let invited = invite_tender_bids_by_capability(
            &paths,
            &published.tender_id,
            &[agent("ZeroClaw", "pk-zc", &["research.web"])],
        )
        .unwrap();
        assert_eq!(invited.invited, 0);
        assert!(invited.tender.bids.is_empty());
    }

    #[test]
    fn snapshot_rebuilds_allocation_when_missing() {
        let dir = tempdir().unwrap();
        let paths = EconomyPaths::from_root(dir.path());
        let agents = vec![
            agent("ZeroClaw", "pk-zc", &["research.web"]),
            agent("DocSmith", "pk-doc", &["document.create"]),
        ];
        crate::admin::adjust_reputation(&paths, "pk-doc", Some("DocSmith"), 80, Some("rep"))
            .unwrap();
        let invited = publish_tender_with_invite(
            &paths,
            TenderPublishParams {
                title: "rebuild-alloc".into(),
                capability: String::new(),
                budget: 40,
                task_ref: "rebuild".into(),
            },
            &agents,
        )
        .unwrap();
        assert!(invited.tender.allocation.is_some());
        let mut stripped = latest_tender(&paths, &invited.tender.tender_id)
            .unwrap()
            .unwrap();
        stripped.allocation = None;
        stripped.prev_hash = None;
        stripped.hash = None;
        append_chained(&paths.tenders, &stripped).unwrap();
        let listed = list_tenders(
            &paths,
            TenderListParams {
                status: None,
            },
        )
        .unwrap();
        let row = listed
            .iter()
            .find(|t| t.tender_id == invited.tender.tender_id)
            .unwrap();
        let alloc = row.allocation.as_ref().expect("rebuilt allocation");
        assert!(!alloc.bids.is_empty());
        assert_eq!(
            alloc.winner_pubkey.to_ascii_lowercase(),
            invited.tender.winner_pubkey.as_deref().unwrap().to_ascii_lowercase()
        );
    }

    #[test]
    fn suggest_tender_fields_is_open_market() {
        let math = suggest_tender_fields("1+1=?");
        assert_eq!(math.capability, OPEN_CAPABILITY);
        assert!(math.budget >= 10);

        let news = suggest_tender_fields("今天AI资讯PDF");
        assert_eq!(news.capability, OPEN_CAPABILITY);
        assert!(news.tags.iter().any(|t| t == "open"));
    }

    #[test]
    fn publish_auto_fills_open_capability_and_budget() {
        let dir = tempdir().unwrap();
        let paths = EconomyPaths::from_root(dir.path());
        let published = publish_tender(
            &paths,
            TenderPublishParams {
                title: "1+1=?".into(),
                capability: String::new(),
                budget: 0,
                task_ref: "ui-auto".into(),
            },
        )
        .unwrap();
        assert_eq!(published.capability, OPEN_CAPABILITY);
        assert!(published.budget > 0);
    }

    #[test]
    fn record_outcome_tool_failure_pays_zero() {
        let dir = tempdir().unwrap();
        let paths = EconomyPaths::from_root(dir.path());
        let agents = vec![agent("DocSmith", "pk-doc", &["document.create"])];
        let invited = publish_tender_with_invite(
            &paths,
            TenderPublishParams {
                title: "今日ai资讯pdf".into(),
                capability: String::new(),
                budget: 50,
                task_ref: "pdf-fail".into(),
            },
            &agents,
        )
        .unwrap();
        let done = record_tender_outcome(
            &paths,
            &invited.tender.tender_id,
            "**处理文档** · 失败 · 'tool' failed",
            true,
        )
        .unwrap();
        assert_eq!(done.reward_grade.as_deref(), Some("fail"));
        assert_eq!(done.reward_gold, Some(0));
        assert!(done.reward_reputation.unwrap_or(0) < 0);
        assert!(done.allocation.as_ref().is_some_and(|a| !a.bids.is_empty()));
    }

    #[test]
    fn record_outcome_settles_contract() {
        let dir = tempdir().unwrap();
        let paths = EconomyPaths::from_root(dir.path());
        let agents = vec![agent("Grok", "pk-grok", &["coordination.route"])];
        let invited = publish_tender_with_invite(
            &paths,
            TenderPublishParams {
                title: "1+1=?".into(),
                capability: String::new(),
                budget: 0,
                task_ref: "math-1".into(),
            },
            &agents,
        )
        .unwrap();
        assert!(invited.auto_resolved);
        let contract_id = invited.tender.contract_id.clone().unwrap();

        let done = record_tender_outcome(&paths, &invited.tender.tender_id, "2", true).unwrap();
        assert_eq!(done.outcome.as_deref(), Some("2"));
        assert_eq!(done.contract_id.as_deref(), Some(contract_id.as_str()));

        let leaf = crate::auction::latest_contract(&paths, &contract_id)
            .unwrap()
            .unwrap();
        assert_eq!(leaf.status, "settled_success");
        assert_eq!(done.reward_gold, Some(done.budget));
        assert_eq!(done.reward_reputation, Some(12));
        assert_eq!(done.reward_grade.as_deref(), Some("excellent"));
        assert!(done.reward_note.as_deref().unwrap_or("").contains("优秀"));
        assert!(done.reward_tags.iter().any(|t| t == "finisher"));
        assert!(!done.tags.is_empty());
    }

    #[test]
    fn thin_outcome_pays_partial_without_title() {
        let dir = tempdir().unwrap();
        let paths = EconomyPaths::from_root(dir.path());
        let agents = vec![agent("Grok", "pk-grok", &["coordination.route"])];
        let invited = publish_tender_with_invite(
            &paths,
            TenderPublishParams {
                title: "say hi briefly".into(),
                capability: "coordination.route".into(),
                budget: 20,
                task_ref: "thin-1".into(),
            },
            &agents,
        )
        .unwrap();
        let done = record_tender_outcome(&paths, &invited.tender.tender_id, "嗯", true).unwrap();
        assert_eq!(done.reward_grade.as_deref(), Some("thin"));
        assert_eq!(done.reward_gold, Some(14)); // 70% of 20
        assert_eq!(done.reward_reputation, Some(3));
        assert!(done.reward_tags.is_empty());
    }

    #[test]
    fn cancel_and_clear_stuck_tenders() {
        let dir = tempdir().unwrap();
        let paths = EconomyPaths::from_root(dir.path());
        let agents = vec![agent("Grok", "pk-grok", &["coordination.route"])];
        let invited = publish_tender_with_invite(
            &paths,
            TenderPublishParams {
                title: "hello".into(),
                capability: String::new(),
                budget: 0,
                task_ref: "clear-1".into(),
            },
            &agents,
        )
        .unwrap();
        assert_eq!(invited.tender.status, "resolved");
        assert!(invited.tender.outcome.is_none());

        let cleared = clear_tenders(&paths, TenderClearMode::Stuck).unwrap();
        assert_eq!(cleared, 1);
        let listed = list_tenders(
            &paths,
            TenderListParams {
                status: None,
            },
        )
        .unwrap();
        assert!(listed.is_empty());
    }
}
