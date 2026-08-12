//! Thin MCP wrapper over `buzz-economy` (single authoritative implementation).

use crate::route::{self, RosterAgent};
use crate::shell::SharedState;
use buzz_economy::{self as eco, EconomyPaths, RosterAgent as EcoAgent};
use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::Deserialize;

fn paths_from_state(state: &SharedState) -> EconomyPaths {
    // Prefer env overrides; fall back to the same room-memory dir as route/memory.
    let mut paths = EconomyPaths::resolve();
    // When BONY_ROOM_MEMORY_DIR is unset, align root with route::room_memory_dir
    // so MCP cwd-relative fallbacks stay consistent.
    if std::env::var("BONY_ROOM_MEMORY_DIR")
        .ok()
        .as_deref()
        .unwrap_or("")
        .is_empty()
        && std::env::var("BONY_ROOM_ECONOMY_LEDGER_PATH")
            .ok()
            .as_deref()
            .unwrap_or("")
            .is_empty()
    {
        paths = EconomyPaths::from_root(route::room_memory_dir(state));
    }
    paths
}

fn to_eco_agents(agents: &[RosterAgent]) -> Vec<EcoAgent> {
    agents
        .iter()
        .map(|a| EcoAgent {
            name: a.name.clone(),
            pubkey: a.pubkey.clone(),
            capabilities: a.capabilities.clone(),
            status: a.status.clone(),
        })
        .collect()
}

fn map_err(e: eco::EconomyError) -> ErrorData {
    match e {
        eco::EconomyError::InvalidParams(m) => ErrorData::invalid_params(m, None),
        other => ErrorData::internal_error(other.to_string(), None),
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AuctionParams {
    pub capability: String,
    pub budget: i64,
    pub task_ref: String,
    #[serde(default)]
    pub max_stake: Option<i64>,
    #[serde(default)]
    pub bidder_kind: Option<String>,
    #[serde(default)]
    pub org_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SubcontractParams {
    pub contract_id: String,
    pub to_capability_or_name: String,
    #[serde(default)]
    pub cut_bp: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SettleParams {
    pub contract_id: String,
    pub status: String,
    #[serde(default)]
    pub quality_note: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LeaderboardParams {
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WalletParams {
    pub pubkey_or_name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OrgCreateParams {
    pub name: String,
    pub founder_pubkey: String,
    #[serde(default)]
    pub founder_name: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OrgJoinParams {
    pub org_id: String,
    pub member_pubkey: String,
    #[serde(default)]
    pub member_name: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OrgLeaveParams {
    pub org_id: String,
    pub member_pubkey: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OrgListParams {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TenderPublishParams {
    pub title: String,
    pub capability: String,
    pub budget: i64,
    pub task_ref: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TenderBidParams {
    pub tender_id: String,
    pub bidder_pubkey: String,
    pub bidder_name: String,
    #[serde(default)]
    pub bidder_kind: Option<String>,
    #[serde(default)]
    pub stake: Option<i64>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TenderResolveParams {
    pub tender_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TenderListParams {
    #[serde(default)]
    pub status: Option<String>,
}

fn parse_bidder_kind(raw: Option<&str>) -> Option<eco::BidderKind> {
    match raw.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("org") | Some("organization") => Some(eco::BidderKind::Org),
        Some("agent") => Some(eco::BidderKind::Agent),
        _ => None,
    }
}

pub fn auction(state: &SharedState, p: AuctionParams) -> Result<String, ErrorData> {
    let paths = paths_from_state(state);
    let roster = route::load_roster(state).map_err(map_route_err)?;
    let agents = to_eco_agents(&roster.agents);
    // Overlay dynamic capability grants onto roster for scoring.
    let agents = overlay_grants(&paths, agents);
    let result = eco::auction(
        &paths,
        &agents,
        eco::AuctionParams {
            capability: p.capability,
            budget: p.budget,
            task_ref: p.task_ref,
            max_stake: p.max_stake,
            bidder_kind: parse_bidder_kind(p.bidder_kind.as_deref()),
            org_id: p.org_id,
        },
    )
    .map_err(map_err)?;
    Ok(format!(
        "auction awarded contract {} → @{} | score={:.3} | mismatch={} | budget={} | caps: {} | reason: {}",
        result.contract_id,
        result.winner_name,
        result.score,
        result.mismatch,
        result.budget,
        result.capabilities.join(","),
        result.reason
    ))
}

pub fn subcontract(state: &SharedState, p: SubcontractParams) -> Result<String, ErrorData> {
    let paths = paths_from_state(state);
    let roster = route::load_roster(state).map_err(map_route_err)?;
    let agents = overlay_grants(&paths, to_eco_agents(&roster.agents));
    let result = eco::subcontract(
        &paths,
        &agents,
        eco::SubcontractParams {
            contract_id: p.contract_id,
            to_capability_or_name: p.to_capability_or_name,
            cut_bp: p.cut_bp,
        },
    )
    .map_err(map_err)?;
    Ok(format!(
        "subcontract {} → child {} @{} | remaining_budget={} | broker_cut={} ({}bp) | mismatch={} | depth={}",
        result.parent_contract_id,
        result.child_contract_id,
        result.winner_name,
        result.remaining_budget,
        result.brokerage,
        result.cut_bp,
        result.mismatch,
        result.depth
    ))
}

pub fn settle(state: &SharedState, p: SettleParams) -> Result<String, ErrorData> {
    let paths = paths_from_state(state);
    let result = eco::settle(
        &paths,
        eco::SettleParams {
            contract_id: p.contract_id,
            status: p.status,
            quality_note: p.quality_note,
        },
    )
    .map_err(map_err)?;
    let mut out = format!(
        "settled {} as {} → @{} | balance={} | reputation={} ({}) | note={}",
        result.contract_id,
        result.settled_status,
        result.winner_name,
        result.balance,
        result.reputation,
        result.tier,
        result.note
    );
    if !result.new_achievements.is_empty() {
        out.push_str(&format!(
            " | achievements={}",
            result.new_achievements.join(",")
        ));
    }
    if !result.new_capabilities.is_empty() {
        out.push_str(&format!(
            " | capability_grants={}",
            result.new_capabilities.join(",")
        ));
    }
    Ok(out)
}

pub fn leaderboard(state: &SharedState, p: LeaderboardParams) -> Result<String, ErrorData> {
    let paths = paths_from_state(state);
    let roster = route::load_roster(state).unwrap_or_else(|_| route::LiveRoster {
        updated_at: String::new(),
        agents: Vec::new(),
    });
    let known: Vec<(String, String)> = roster
        .agents
        .iter()
        .filter(|a| !a.pubkey.is_empty())
        .map(|a| (a.pubkey.clone(), a.name.clone()))
        .collect();
    // Include orgs on the board.
    let mut known = known;
    if let Ok(orgs) = eco::list_orgs(&paths) {
        for org in orgs {
            known.push((org.org_id, org.name));
        }
    }
    let rows =
        eco::get_leaderboard(&paths, &known, p.limit.map(|n| n as usize)).map_err(map_err)?;
    if rows.is_empty() {
        return Ok(
            "economy leaderboard empty — no agents on roster and no ledger entries yet".into(),
        );
    }
    let mut out = format!("{} agent(s) by reputation then balance:\n", rows.len());
    for (i, row) in rows.iter().enumerate() {
        out.push_str(&format!(
            "{}. {} | {} | bal={} | rep={} | tags={} | pubkey={}\n",
            i + 1,
            row.name,
            row.tier,
            row.balance,
            row.reputation,
            row.tags.join(","),
            if row.pubkey.is_empty() {
                "-"
            } else {
                &row.pubkey
            }
        ));
    }
    Ok(out)
}

pub fn wallet(state: &SharedState, p: WalletParams) -> Result<String, ErrorData> {
    let paths = paths_from_state(state);
    let view = eco::get_wallet(
        &paths,
        &eco::WalletParams {
            pubkey_or_name: p.pubkey_or_name.clone(),
            history_limit: None,
        },
    )
    .map_err(map_err)?;
    let Some(snap) = view else {
        return Ok(format!("no wallet found for \"{}\"", p.pubkey_or_name));
    };
    let mut out = format!(
        "wallet @{} | {} | bal={} | rep={} | tags={} | achievements={} | grants={} | pubkey={}\n",
        snap.name,
        snap.tier,
        snap.balance,
        snap.reputation,
        snap.tags.join(","),
        snap.achievements.join(","),
        snap.capability_grants.join(","),
        if snap.pubkey.is_empty() {
            "-"
        } else {
            &snap.pubkey
        }
    );
    if snap.history.is_empty() {
        out.push_str("recent ledger: (none — starting balance only)\n");
    } else {
        out.push_str("recent ledger (newest first):\n");
        for entry in &snap.history {
            out.push_str(&format!(
                "- [{}] {} amount={} rep_delta={} task={} note={}\n",
                entry.ts,
                entry.kind,
                entry.amount,
                entry.reputation_delta,
                entry.task_ref.as_deref().unwrap_or("-"),
                entry.note.as_deref().unwrap_or("-")
            ));
        }
    }
    Ok(out)
}

pub fn org_create(state: &SharedState, p: OrgCreateParams) -> Result<String, ErrorData> {
    let paths = paths_from_state(state);
    let org = eco::create_org(
        &paths,
        eco::OrgCreateParams {
            name: p.name,
            founder_pubkey: p.founder_pubkey,
            founder_name: p.founder_name,
            tags: p.tags,
        },
    )
    .map_err(map_err)?;
    Ok(format!(
        "org created {} ({}) founder={} members={} tags={}",
        org.name,
        org.org_id,
        org.member_names.first().cloned().unwrap_or_default(),
        org.member_pubkeys.len(),
        org.tags.join(",")
    ))
}

pub fn org_join(state: &SharedState, p: OrgJoinParams) -> Result<String, ErrorData> {
    let paths = paths_from_state(state);
    let org = eco::join_org(
        &paths,
        eco::OrgJoinParams {
            org_id: p.org_id,
            member_pubkey: p.member_pubkey,
            member_name: p.member_name,
        },
    )
    .map_err(map_err)?;
    Ok(format!(
        "joined {} ({}) members={}",
        org.name,
        org.org_id,
        org.member_pubkeys.len()
    ))
}

pub fn org_leave(state: &SharedState, p: OrgLeaveParams) -> Result<String, ErrorData> {
    let paths = paths_from_state(state);
    let org = eco::leave_org(
        &paths,
        eco::OrgLeaveParams {
            org_id: p.org_id,
            member_pubkey: p.member_pubkey,
        },
    )
    .map_err(map_err)?;
    Ok(format!(
        "left {} ({}) remaining_members={}",
        org.name,
        org.org_id,
        org.member_pubkeys.len()
    ))
}

pub fn org_list(state: &SharedState, _p: OrgListParams) -> Result<String, ErrorData> {
    let paths = paths_from_state(state);
    let orgs = eco::list_orgs(&paths).map_err(map_err)?;
    if orgs.is_empty() {
        return Ok("no organizations yet".into());
    }
    let mut out = format!("{} organization(s):\n", orgs.len());
    for org in orgs {
        out.push_str(&format!(
            "- {} ({}) members={} tags={}\n",
            org.name,
            org.org_id,
            org.member_pubkeys.len(),
            org.tags.join(",")
        ));
    }
    Ok(out)
}

pub fn tender_publish(state: &SharedState, p: TenderPublishParams) -> Result<String, ErrorData> {
    let paths = paths_from_state(state);
    let t = eco::publish_tender(
        &paths,
        eco::TenderPublishParams {
            title: p.title,
            capability: p.capability,
            budget: p.budget,
            task_ref: p.task_ref,
        },
    )
    .map_err(map_err)?;
    Ok(format!(
        "tender published {} | {} | cap={} | budget={} | task={}",
        t.tender_id, t.title, t.capability, t.budget, t.task_ref
    ))
}

pub fn tender_bid(state: &SharedState, p: TenderBidParams) -> Result<String, ErrorData> {
    let paths = paths_from_state(state);
    let bid = eco::tender_bid(
        &paths,
        eco::TenderBidParams {
            tender_id: p.tender_id,
            bidder_pubkey: p.bidder_pubkey,
            bidder_name: p.bidder_name,
            bidder_kind: parse_bidder_kind(p.bidder_kind.as_deref()),
            stake: p.stake,
            note: p.note,
        },
    )
    .map_err(map_err)?;
    Ok(format!(
        "bid placed on {} by @{} ({:?}) stake={}",
        bid.tender_id, bid.bidder_name, bid.bidder_kind, bid.stake
    ))
}

pub fn tender_resolve(state: &SharedState, p: TenderResolveParams) -> Result<String, ErrorData> {
    let paths = paths_from_state(state);
    let roster = route::load_roster(state).map_err(map_route_err)?;
    let agents = overlay_grants(&paths, to_eco_agents(&roster.agents));
    // Enrich org candidates via org fold inside resolve_tender.
    let t = eco::resolve_tender(
        &paths,
        &agents,
        eco::TenderResolveParams {
            tender_id: p.tender_id,
        },
    )
    .map_err(map_err)?;
    Ok(format!(
        "tender {} resolved → @{} | contract={} | bids={}",
        t.tender_id,
        t.winner_name.as_deref().unwrap_or("-"),
        t.contract_id.as_deref().unwrap_or("-"),
        t.bids.len()
    ))
}

pub fn tender_list(state: &SharedState, p: TenderListParams) -> Result<String, ErrorData> {
    let paths = paths_from_state(state);
    let rows =
        eco::list_tenders(&paths, eco::TenderListParams { status: p.status }).map_err(map_err)?;
    if rows.is_empty() {
        return Ok("no tenders".into());
    }
    let mut out = format!("{} tender(s):\n", rows.len());
    for t in rows {
        out.push_str(&format!(
            "- {} | {} | {} | cap={} | budget={} | bids={} | winner={}\n",
            t.tender_id,
            t.status,
            t.title,
            t.capability,
            t.budget,
            t.bids.len(),
            t.winner_name.as_deref().unwrap_or("-")
        ));
    }
    Ok(out)
}

fn overlay_grants(paths: &EconomyPaths, mut agents: Vec<EcoAgent>) -> Vec<EcoAgent> {
    for agent in &mut agents {
        if let Ok(grants) = eco::granted_capabilities(paths, &agent.pubkey) {
            for g in grants {
                if !agent.capabilities.iter().any(|c| c == &g) {
                    agent.capabilities.push(g);
                }
            }
        }
    }
    agents
}

fn map_route_err(e: ErrorData) -> ErrorData {
    e
}
