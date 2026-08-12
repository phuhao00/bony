use crate::auction::{pick_auction_winner, AuctionCandidate};
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
    pub stake: i64,
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
    pub bids: Vec<BidRecord>,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TenderPublishParams {
    pub title: String,
    pub capability: String,
    pub budget: i64,
    pub task_ref: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TenderBidParams {
    pub tender_id: String,
    pub bidder_pubkey: String,
    pub bidder_name: String,
    pub bidder_kind: Option<BidderKind>,
    pub stake: Option<i64>,
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

pub fn publish_tender(
    paths: &EconomyPaths,
    p: TenderPublishParams,
) -> Result<TenderSnapshot, EconomyError> {
    let title = p.title.trim();
    let capability = p.capability.trim();
    let task_ref = p.task_ref.trim();
    if title.is_empty() || capability.is_empty() || task_ref.is_empty() {
        return Err(EconomyError::invalid(
            "title, capability, and task_ref must not be empty",
        ));
    }
    if p.budget <= 0 {
        return Err(EconomyError::invalid("budget must be a positive integer"));
    }
    let tender_id = format!("t-{}-{:x}", Utc::now().timestamp_millis(), fastrand_u32());
    let ts = Utc::now().to_rfc3339();
    let record = TenderRecord {
        ts: ts.clone(),
        tender_id: tender_id.clone(),
        capability: capability.to_string(),
        budget: p.budget,
        task_ref: task_ref.to_string(),
        title: title.to_string(),
        status: "open".into(),
        winner_pubkey: None,
        winner_name: None,
        contract_id: None,
        prev_hash: None,
        hash: None,
    };
    append_chained(&paths.tenders, &record)?;
    Ok(TenderSnapshot {
        tender_id,
        title: title.to_string(),
        capability: capability.to_string(),
        budget: p.budget,
        task_ref: task_ref.to_string(),
        status: "open".into(),
        winner_pubkey: None,
        winner_name: None,
        contract_id: None,
        bids: Vec::new(),
        created_at: ts,
    })
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
        stake: p.stake.unwrap_or(0).max(0),
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

    // Build candidate roster from actual bidders only.
    let mut candidates: Vec<RosterAgent> = Vec::new();
    for bid in &bids {
        match bid.bidder_kind {
            BidderKind::Org => {
                let org = orgs.get(&bid.bidder_pubkey);
                let caps = org
                    .map(|o| org_member_capabilities(o, agents))
                    .unwrap_or_default();
                candidates.push(RosterAgent {
                    name: bid.bidder_name.clone(),
                    pubkey: bid.bidder_pubkey.clone(),
                    capabilities: caps,
                    status: "running".into(),
                });
            }
            BidderKind::Agent => {
                let caps = agents
                    .iter()
                    .find(|a| a.pubkey.eq_ignore_ascii_case(&bid.bidder_pubkey))
                    .map(|a| a.capabilities.clone())
                    .unwrap_or_default();
                candidates.push(RosterAgent {
                    name: bid.bidder_name.clone(),
                    pubkey: bid.bidder_pubkey.clone(),
                    capabilities: caps,
                    status: "running".into(),
                });
            }
        }
    }

    let winner: AuctionCandidate = pick_auction_winner(
        &candidates,
        &tender.capability,
        &balances,
        tender.budget.max(1),
        None,
        false,
    )
    .ok_or_else(|| EconomyError::invalid("could not pick a winner from bids"))?;

    let contract_id = format!("c-{}-{:x}", Utc::now().timestamp_millis(), fastrand_u32());
    let contract = ContractRecord {
        ts: Utc::now().to_rfc3339(),
        contract_id: contract_id.clone(),
        task_ref: tender.task_ref.clone(),
        capability: tender.capability.clone(),
        budget: tender.budget,
        winner_name: winner.name.clone(),
        winner_pubkey: winner.pubkey.clone(),
        effective_score: winner.score,
        mismatch: winner.mismatch,
        parent_contract_id: None,
        cut_bp: None,
        depth: 0,
        status: "awarded".into(),
        bidder_kind: Some(if winner.pubkey.starts_with("org:") {
            BidderKind::Org
        } else {
            BidderKind::Agent
        }),
        prev_hash: None,
        hash: None,
    };
    append_chained(&paths.contracts, &contract)?;

    let resolved = TenderRecord {
        ts: Utc::now().to_rfc3339(),
        tender_id: tender.tender_id.clone(),
        capability: tender.capability.clone(),
        budget: tender.budget,
        task_ref: tender.task_ref.clone(),
        title: tender.title.clone(),
        status: "resolved".into(),
        winner_pubkey: Some(winner.pubkey.clone()),
        winner_name: Some(winner.name.clone()),
        contract_id: Some(contract_id.clone()),
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
        winner_pubkey: Some(winner.pubkey),
        winner_name: Some(winner.name),
        contract_id: Some(contract_id),
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
        }
        let mut snap = TenderSnapshot {
            tender_id: t.tender_id.clone(),
            title: t.title.clone(),
            capability: t.capability.clone(),
            budget: t.budget,
            task_ref: t.task_ref.clone(),
            status: t.status.clone(),
            winner_pubkey: t.winner_pubkey.clone(),
            winner_name: t.winner_name.clone(),
            contract_id: t.contract_id.clone(),
            bids: bids_for(paths, &t.tender_id)?,
            created_at: t.ts.clone(),
        };
        // Prefer earliest open timestamp as created_at — already from fold.
        let _ = &mut snap;
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
