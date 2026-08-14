//! Room agent economy — hash-chained virtual credits, reputation, orgs, tenders.
//!
//! Single authoritative implementation shared by `buzz-dev-mcp` (Grok MCP)
//! and `buzz-desktop` (UI / admin writes). Append paths use exclusive file
//! locks so both writers can coexist safely.

mod achievements;
mod auction;
mod chain;
mod error;
mod evolve;
mod ledger;
mod org;
mod paths;
mod quote;
mod reward;
mod tender;
mod types;

pub use achievements::{
    default_catalog, evaluate_achievements, load_catalog, save_catalog, AchievementCatalog,
    AchievementDef,
};
pub use auction::{
    auction, best_capability_match_ratio, capability_match_ratio, hire_support,
    pick_auction_winner, pick_auction_winner_best_of, pick_open_market_winner, settle, subcontract,
    AuctionParams, AuctionResult, HireSupportParams, HireSupportResult, SettleParams, SettleResult,
    SubcontractParams, SubcontractResult,
};
pub use chain::{append_chained, last_hash, verify_chain, ChainReport, GENESIS_HASH_HEX};
pub use error::EconomyError;
pub use evolve::{evolve_capabilities, granted_capabilities, EvolveResult};
pub use ledger::{
    fold_ledger, get_leaderboard, get_wallet, tier_for, AgentBalance, AgentEconomySnapshot,
    EconomyWalletView, LedgerHistoryEntry, WalletParams,
};
pub use org::{
    create_org, join_org, leave_org, list_orgs, org_member_capabilities, OrgCreateParams,
    OrgJoinParams, OrgLeaveParams, OrgSnapshot,
};
pub use paths::{
    achievements_catalog_path, contracts_path, ledger_path, orgs_path, room_memory_dir,
    tenders_path, EconomyPaths,
};
pub use quote::{
    compute_agent_quote, decide_allocation, fold_payout_stats, quote_for_agent, score_quote_bid,
    AgentQuote, AllocationBid, AllocationDecision, PayoutStats, W_CAPABILITY as QUOTE_W_CAPABILITY,
    W_LIQUIDITY, W_QUOTE, W_REPUTATION as QUOTE_W_REPUTATION,
};
pub use reward::{
    base_budget_for_capability, compute_settlement_reward, grade_outcome, title_for_capability,
    QualityGrade, SettlementReward, SettlementRewardInput, FAIL_MISMATCH_PENALTY_BPS,
};
pub use tender::{
    cancel_tender, clear_tenders, complete_open_tender, invite_tender_bids_by_capability,
    list_tenders, performance_tags_for_capability, publish_tender, publish_tender_with_invite,
    record_tender_outcome, resolve_tender, suggest_tender_fields, sweep_open_tenders,
    tags_for_tender, tender_bid, BidRecord, TenderBidParams, TenderClearMode, TenderInviteResult,
    TenderListParams, TenderPublishParams, TenderRecord, TenderResolveParams, TenderSnapshot,
    TenderSuggestion, OPEN_CAPABILITY,
};
pub use types::{
    BidderKind, ContractRecord, LedgerEntry, LedgerKind, OrgEvent, OrgEventKind, RosterAgent,
};

/// Admin / Desktop manual adjustment helpers.
pub mod admin {
    use crate::chain::append_chained;
    use crate::error::EconomyError;
    use crate::paths::EconomyPaths;
    use crate::types::{LedgerEntry, LedgerKind};
    use chrono::Utc;

    pub fn adjust_balance(
        paths: &EconomyPaths,
        pubkey: &str,
        name: Option<&str>,
        delta: i64,
        note: Option<&str>,
    ) -> Result<LedgerEntry, EconomyError> {
        let entry = LedgerEntry {
            ts: Utc::now().to_rfc3339(),
            pubkey: pubkey.to_string(),
            kind: LedgerKind::ManualAdjustment,
            amount: delta,
            reputation_delta: 0,
            task_ref: None,
            note: Some(
                note.unwrap_or("manual balance adjustment from Desktop")
                    .to_string(),
            ),
            name: name.map(str::to_string),
            tags: Vec::new(),
            achievements: Vec::new(),
            capability_grants: Vec::new(),
            prev_hash: None,
            hash: None,
        };
        append_chained(&paths.ledger, &entry)
    }

    pub fn adjust_reputation(
        paths: &EconomyPaths,
        pubkey: &str,
        name: Option<&str>,
        delta: i32,
        note: Option<&str>,
    ) -> Result<LedgerEntry, EconomyError> {
        let entry = LedgerEntry {
            ts: Utc::now().to_rfc3339(),
            pubkey: pubkey.to_string(),
            kind: LedgerKind::ManualAdjustment,
            amount: 0,
            reputation_delta: delta,
            task_ref: None,
            note: Some(
                note.unwrap_or("manual reputation adjustment from Desktop")
                    .to_string(),
            ),
            name: name.map(str::to_string),
            tags: Vec::new(),
            achievements: Vec::new(),
            capability_grants: Vec::new(),
            prev_hash: None,
            hash: None,
        };
        append_chained(&paths.ledger, &entry)
    }

    pub fn set_tags(
        paths: &EconomyPaths,
        pubkey: &str,
        name: Option<&str>,
        tags: Vec<String>,
        note: Option<&str>,
    ) -> Result<LedgerEntry, EconomyError> {
        let entry = LedgerEntry {
            ts: Utc::now().to_rfc3339(),
            pubkey: pubkey.to_string(),
            kind: LedgerKind::TagAssign,
            amount: 0,
            reputation_delta: 0,
            task_ref: None,
            note: Some(note.unwrap_or("tag assign from Desktop").to_string()),
            name: name.map(str::to_string),
            tags,
            achievements: Vec::new(),
            capability_grants: Vec::new(),
            prev_hash: None,
            hash: None,
        };
        append_chained(&paths.ledger, &entry)
    }

    pub fn grant_achievement(
        paths: &EconomyPaths,
        pubkey: &str,
        name: Option<&str>,
        achievement_id: &str,
        gold: i64,
        reputation: i32,
        note: Option<&str>,
    ) -> Result<LedgerEntry, EconomyError> {
        let entry = LedgerEntry {
            ts: Utc::now().to_rfc3339(),
            pubkey: pubkey.to_string(),
            kind: LedgerKind::Achievement,
            amount: gold,
            reputation_delta: reputation,
            task_ref: None,
            note: Some(note.unwrap_or("achievement grant from Desktop").to_string()),
            name: name.map(str::to_string),
            tags: Vec::new(),
            achievements: vec![achievement_id.to_string()],
            capability_grants: Vec::new(),
            prev_hash: None,
            hash: None,
        };
        append_chained(&paths.ledger, &entry)
    }
}
