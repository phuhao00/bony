use crate::chain::ChainedRow;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LedgerKind {
    Award,
    Payout,
    Brokerage,
    Penalty,
    Refund,
    Seed,
    ManualAdjustment,
    Achievement,
    TagAssign,
    CapabilityGrant,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LedgerEntry {
    pub ts: String,
    pub pubkey: String,
    pub kind: LedgerKind,
    pub amount: i64,
    #[serde(default)]
    pub reputation_delta: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Absolute tag set when kind == TagAssign (replace semantics).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Newly unlocked achievement ids when kind == Achievement.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub achievements: Vec<String>,
    /// Capability ids granted for routing/bidding only (never ACP permissions).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capability_grants: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

impl ChainedRow for LedgerEntry {
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
pub struct ContractRecord {
    pub ts: String,
    pub contract_id: String,
    pub task_ref: String,
    pub capability: String,
    pub budget: i64,
    pub winner_name: String,
    pub winner_pubkey: String,
    pub effective_score: f64,
    pub mismatch: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_contract_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cut_bp: Option<u32>,
    #[serde(default)]
    pub depth: u32,
    /// awarded | subcontracted | settled_success | settled_failed
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bidder_kind: Option<BidderKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

impl ChainedRow for ContractRecord {
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BidderKind {
    Agent,
    Org,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RosterAgent {
    pub name: String,
    #[serde(default)]
    pub pubkey: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OrgEventKind {
    OrgCreate,
    OrgJoin,
    OrgLeave,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OrgEvent {
    pub ts: String,
    pub kind: OrgEventKind,
    pub org_id: String,
    pub org_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_pubkey: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

impl ChainedRow for OrgEvent {
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
