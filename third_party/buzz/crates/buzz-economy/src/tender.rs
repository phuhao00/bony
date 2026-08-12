use crate::auction::{capability_matches, pick_auction_winner, AuctionCandidate};
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
    pub outcome: Option<String>,
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
    if capability.is_empty() {
        return Err(EconomyError::invalid("capability must not be empty"));
    }
    if budget <= 0 {
        return Err(EconomyError::invalid("budget must be a positive integer"));
    }
    let tender_id = format!("t-{}-{:x}", Utc::now().timestamp_millis(), fastrand_u32());
    let ts = Utc::now().to_rfc3339();
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
        bids: Vec::new(),
        created_at: ts,
    })
}

/// Infer capability + budget from a free-text tender title.
///
/// Keyword map aligns with room specialist capabilities in
/// `docs/buzz-room-agent-orchestration-plan.md`. Explicit overrides still win
/// when callers pass non-empty capability / positive budget.
pub fn suggest_tender_fields(title: &str) -> TenderSuggestion {
    let raw = title.trim();
    let lower = raw.to_ascii_lowercase();
    let (capability, base_budget, reason) = if contains_any(
        &lower,
        raw,
        &[
            "video",
            "montage",
            "trailer",
            "reel",
            "视频",
            "剪辑",
            "蒙太奇",
            "openmontage",
        ],
    ) {
        ("media.video.render", 100, "title matches media/video")
    } else if contains_any(
        &lower,
        raw,
        &["unity", "scene", "3d", "场景", "关卡", "prefab"],
    ) {
        ("unity.scene.edit", 80, "title matches Unity/scene")
    } else if contains_any(
        &lower,
        raw,
        &[
            "pdf",
            "docx",
            "pptx",
            "xlsx",
            "word",
            "excel",
            "ppt",
            "文档",
            "报告",
            "slides",
            "docsmith",
        ],
    ) {
        ("document.create", 50, "title matches document deliverable")
    } else if contains_any(
        &lower,
        raw,
        &[
            "weather",
            "news",
            "search",
            "research",
            "lookup",
            "资讯",
            "新闻",
            "天气",
            "检索",
            "搜索",
            "查一下",
            "zeroclaw",
        ],
    ) {
        ("research.web", 40, "title matches research/web")
    } else if contains_any(
        &lower,
        raw,
        &[
            "rust",
            "code",
            "bug",
            "fix",
            "compile",
            "refactor",
            "编码",
            "编译",
            "修复",
            "代码",
        ],
    ) {
        ("code.repo.read", 60, "title matches coding work")
    } else {
        (
            "coordination.route",
            30,
            "default to coordinator for general asks",
        )
    };

    // Longer briefs cost a bit more; keep within a small band.
    let len_bonus = (raw.chars().count() / 40) as i64 * 5;
    let budget = (base_budget + len_bonus).clamp(10, 200);

    TenderSuggestion {
        capability: capability.to_string(),
        budget,
        reason: reason.to_string(),
    }
}

fn contains_any(lower_ascii: &str, original: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| {
        if needle.is_ascii() {
            lower_ascii.contains(needle)
        } else {
            original.contains(needle)
        }
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

    let stake = (tender.budget / 10).max(1);
    let note = format!("auto-invite:{}", tender.capability);
    let mut invited = 0usize;
    let mut skipped_existing = 0usize;

    for agent in agents {
        let pubkey = agent.pubkey.trim();
        if pubkey.is_empty() {
            continue;
        }
        if !capability_matches(&agent.capabilities, &tender.capability) {
            continue;
        }
        let key = pubkey.to_ascii_lowercase();
        if already.contains_key(&key) {
            skipped_existing += 1;
            continue;
        }
        tender_bid(
            paths,
            TenderBidParams {
                tender_id: tender_id.to_string(),
                bidder_pubkey: pubkey.to_string(),
                bidder_name: agent.name.clone(),
                bidder_kind: Some(BidderKind::Agent),
                stake: Some(stake),
                note: Some(note.clone()),
            },
        )?;
        already.insert(key, ());
        invited += 1;
    }

    let orgs = fold_orgs(paths).unwrap_or_default();
    for org in orgs.values() {
        let caps = org_member_capabilities(org, agents);
        if !capability_matches(&caps, &tender.capability) {
            continue;
        }
        let key = org.org_id.to_ascii_lowercase();
        if already.contains_key(&key) {
            skipped_existing += 1;
            continue;
        }
        tender_bid(
            paths,
            TenderBidParams {
                tender_id: tender_id.to_string(),
                bidder_pubkey: org.org_id.clone(),
                bidder_name: org.name.clone(),
                bidder_kind: Some(BidderKind::Org),
                stake: Some(stake),
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
        outcome: None,
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
        outcome: None,
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
            outcome: t.outcome.clone(),
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

fn tender_snapshot(
    paths: &EconomyPaths,
    tender: &TenderRecord,
) -> Result<TenderSnapshot, EconomyError> {
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
        bids: bids_for(paths, &tender.tender_id)?,
        created_at: tender.ts.clone(),
    })
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
        prev_hash: None,
        hash: None,
    };
    append_chained(&paths.tenders, &updated)?;

    let _ = crate::auction::settle(
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
            agent("Unity", "pk-unity", &["unity.scene.edit"]),
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
        assert_eq!(invited.tender.bids[0].stake, 4);
        assert!(invited.tender.contract_id.is_some());
    }

    #[test]
    fn invite_skips_when_no_capability_match() {
        let dir = tempdir().unwrap();
        let paths = EconomyPaths::from_root(dir.path());
        let published = publish_tender(
            &paths,
            TenderPublishParams {
                title: "scene".into(),
                capability: "unity.scene.edit".into(),
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
    fn suggest_tender_fields_from_title_keywords() {
        let math = suggest_tender_fields("1+1=?");
        assert_eq!(math.capability, "coordination.route");
        assert!(math.budget >= 10);

        let news = suggest_tender_fields("今天AI资讯PDF");
        assert_eq!(news.capability, "document.create");

        let research = suggest_tender_fields("查一下今天天气");
        assert_eq!(research.capability, "research.web");

        let video = suggest_tender_fields("剪一个产品宣传视频");
        assert_eq!(video.capability, "media.video.render");
    }

    #[test]
    fn publish_auto_fills_capability_and_budget() {
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
        assert_eq!(published.capability, "coordination.route");
        assert!(published.budget > 0);
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
    }
}
