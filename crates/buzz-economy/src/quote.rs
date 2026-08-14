//! Agent quote schedule — statistical asking prices for tender bids.
//!
//! Quotes are derived from ledger payout history + reputation + budget band.
//! Allocation scoring prefers **reasonable low quotes** without starving
//! high-reputation agents. Single authority for invite + resolve.

use crate::error::EconomyError;
use crate::ledger::{fold_ledger, AgentBalance};
use crate::paths::EconomyPaths;
use crate::types::{LedgerEntry, LedgerKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

const MAX_SCAN_LINES: usize = 5000;

/// Weight: lower quote is better (value).
pub const W_QUOTE: f64 = 0.35;
/// Weight: reputation quality.
pub const W_REPUTATION: f64 = 0.35;
/// Weight: capability match (1.0 on open market).
pub const W_CAPABILITY: f64 = 0.20;
/// Weight: wallet liquidity / ability to bond.
pub const W_LIQUIDITY: f64 = 0.10;

#[derive(Debug, Clone, Default)]
pub struct PayoutStats {
    pub payout_count: u32,
    pub total_payout: i64,
}

impl PayoutStats {
    pub fn avg_payout(&self) -> i64 {
        if self.payout_count == 0 {
            0
        } else {
            self.total_payout / i64::from(self.payout_count)
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentQuote {
    pub quote: i64,
    pub floor: i64,
    pub ceiling: i64,
    /// Short basis for UI (“历史均价 / 预算中位 / 声望溢价”).
    pub basis: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AllocationBid {
    pub bidder_name: String,
    pub bidder_pubkey: String,
    pub quote: i64,
    pub reputation: i32,
    pub score: f64,
    pub reason: String,
    pub won: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AllocationDecision {
    pub winner_name: String,
    pub winner_pubkey: String,
    pub winner_quote: i64,
    pub budget: i64,
    pub note: String,
    pub bids: Vec<AllocationBid>,
}

/// Fold positive payout amounts per pubkey (settlement / support hire income).
pub fn fold_payout_stats(paths: &EconomyPaths) -> Result<BTreeMap<String, PayoutStats>, EconomyError> {
    fold_payout_stats_at(&paths.ledger)
}

pub fn fold_payout_stats_at(path: &Path) -> Result<BTreeMap<String, PayoutStats>, EconomyError> {
    let mut map: BTreeMap<String, PayoutStats> = BTreeMap::new();
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
        if !matches!(entry.kind, LedgerKind::Payout) || entry.amount <= 0 {
            continue;
        }
        let slot = map.entry(entry.pubkey).or_default();
        slot.payout_count = slot.payout_count.saturating_add(1);
        slot.total_payout = slot.total_payout.saturating_add(entry.amount);
    }
    Ok(map)
}

/// Compute a reasonable asking quote for `budget`.
///
/// Band: `[25% · budget, budget]`. Anchor blends historical average payout
/// (when ≥2 samples) with mid-budget; reputation adds a small premium;
/// high wallet balance allows a slight competitive discount.
pub fn compute_agent_quote(
    budget: i64,
    balance: &AgentBalance,
    stats: &PayoutStats,
) -> AgentQuote {
    let budget = budget.max(1);
    let floor = ((budget * 25) / 100).max(1);
    let ceiling = budget;
    let mid = (budget * 50) / 100;

    let (anchor, hist_note) = if stats.payout_count >= 2 {
        let avg = stats.avg_payout().clamp(floor, ceiling);
        (avg, format!("历史均价¤{avg}×{}", stats.payout_count))
    } else {
        (mid, "预算中位".into())
    };

    let norm_rep = normalize_reputation(balance.reputation);
    let premium = ((budget as f64) * 0.12 * norm_rep).round() as i64;
    let eager_discount = {
        let flush = (balance.balance.max(0) as f64) / (budget as f64);
        if flush >= 2.0 {
            ((budget as f64) * 0.05).round() as i64
        } else {
            0
        }
    };

    let raw = anchor.saturating_add(premium).saturating_sub(eager_discount);
    let quote = raw.clamp(floor, ceiling);
    let basis = format!(
        "{hist_note} · 声望溢价¤{premium} · 竞争折扣¤{eager_discount} → ¤{quote}"
    );
    AgentQuote {
        quote,
        floor,
        ceiling,
        basis,
    }
}

/// Score one bid for allocation. Higher is better.
pub fn score_quote_bid(
    budget: i64,
    quote: i64,
    reputation: i32,
    balance: i64,
    capability_match: f64,
) -> (f64, String) {
    let budget = budget.max(1) as f64;
    let quote = quote.clamp(1, budget as i64) as f64;
    let quote_factor = (1.0 - (quote / budget)).clamp(0.0, 1.0);
    let norm_rep = normalize_reputation(reputation);
    let liquidity = (balance.max(0) as f64 / budget).clamp(0.0, 1.0);
    let cap = capability_match.clamp(0.0, 1.0);
    let score =
        W_QUOTE * quote_factor + W_REPUTATION * norm_rep + W_CAPABILITY * cap + W_LIQUIDITY * liquidity;
    let reason = format!(
        "报价¤{:.0}(值{:.2}) 声望{norm_rep:.2} 能力{cap:.2} 流动性{liquidity:.2}",
        quote, quote_factor
    );
    (score, reason)
}

fn normalize_reputation(rep: i32) -> f64 {
    let r = rep.max(0) as f64;
    r / (r + 500.0)
}

/// Build a full allocation decision from quoted bids (sorted by score desc).
pub fn decide_allocation(
    budget: i64,
    mut rows: Vec<AllocationBid>,
) -> Option<AllocationDecision> {
    if rows.is_empty() {
        return None;
    }
    rows.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.quote.cmp(&b.quote))
            .then_with(|| {
                a.bidder_pubkey
                    .to_ascii_lowercase()
                    .cmp(&b.bidder_pubkey.to_ascii_lowercase())
            })
    });
    for (i, row) in rows.iter_mut().enumerate() {
        row.won = i == 0;
    }
    let winner = rows[0].clone();
    let note = format!(
        "中标 @{} · 报价¤{} / 预算¤{} · {}",
        winner.bidder_name, winner.quote, budget, winner.reason
    );
    Some(AllocationDecision {
        winner_name: winner.bidder_name,
        winner_pubkey: winner.bidder_pubkey,
        winner_quote: winner.quote,
        budget,
        note,
        bids: rows,
    })
}

pub fn quote_for_agent(
    paths: &EconomyPaths,
    pubkey: &str,
    name: &str,
    budget: i64,
) -> Result<AgentQuote, EconomyError> {
    let balances = fold_ledger(paths)?;
    let stats_map = fold_payout_stats(paths)?;
    let bal = balances.get(pubkey).cloned().unwrap_or(AgentBalance {
        balance: crate::ledger::STARTING_BALANCE,
        reputation: crate::ledger::STARTING_REPUTATION,
        name: name.to_string(),
        tags: Vec::new(),
        achievements: Default::default(),
        capability_grants: Default::default(),
    });
    let stats = stats_map.get(pubkey).cloned().unwrap_or_default();
    Ok(compute_agent_quote(budget, &bal, &stats))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::STARTING_BALANCE;

    fn bal(rep: i32, balance: i64) -> AgentBalance {
        AgentBalance {
            balance,
            reputation: rep,
            name: "x".into(),
            tags: Vec::new(),
            achievements: Default::default(),
            capability_grants: Default::default(),
        }
    }

    #[test]
    fn quote_stays_inside_budget_band() {
        let q = compute_agent_quote(40, &bal(0, STARTING_BALANCE), &PayoutStats::default());
        assert!(q.quote >= q.floor && q.quote <= q.ceiling);
        assert_eq!(q.floor, 10);
        assert_eq!(q.ceiling, 40);
    }

    #[test]
    fn history_anchors_quote() {
        let stats = PayoutStats {
            payout_count: 4,
            total_payout: 120, // avg 30
        };
        // balance below 2× budget → no eager discount
        let q = compute_agent_quote(40, &bal(0, 40), &stats);
        assert_eq!(q.quote, 30);
    }

    #[test]
    fn lower_quote_scores_higher_all_else_equal() {
        let (hi, _) = score_quote_bid(40, 20, 100, 100, 1.0);
        let (lo, _) = score_quote_bid(40, 35, 100, 100, 1.0);
        assert!(hi > lo);
    }

    #[test]
    fn decide_picks_best_score() {
        let decision = decide_allocation(
            40,
            vec![
                AllocationBid {
                    bidder_name: "A".into(),
                    bidder_pubkey: "pk-a".into(),
                    quote: 30,
                    reputation: 50,
                    score: 0.5,
                    reason: "a".into(),
                    won: false,
                },
                AllocationBid {
                    bidder_name: "B".into(),
                    bidder_pubkey: "pk-b".into(),
                    quote: 22,
                    reputation: 80,
                    score: 0.8,
                    reason: "b".into(),
                    won: false,
                },
            ],
        )
        .unwrap();
        assert_eq!(decision.winner_name, "B");
        assert!(decision.bids[0].won);
        assert!(!decision.bids[1].won);
    }
}
