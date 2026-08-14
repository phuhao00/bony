//! Authoritative settlement reward schedule.
//!
//! One truth for gold / reputation / title unlocks. Auction settle and tender
//! fulfillment both call [`compute_settlement_reward`] — do not hard-code
//! payouts at call sites.

use serde::Serialize;

/// Outcome quality after delivery inspection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum QualityGrade {
    /// Concise correct answer or substantial deliverable.
    Excellent,
    /// Usable answer of normal length.
    Pass,
    /// Barely usable / too thin.
    Thin,
    /// Empty, error-like, or declared failure.
    Fail,
}

impl QualityGrade {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Excellent => "excellent",
            Self::Pass => "pass",
            Self::Thin => "thin",
            Self::Fail => "fail",
        }
    }

    pub fn label_zh(self) -> &'static str {
        match self {
            Self::Excellent => "优秀",
            Self::Pass => "合格",
            Self::Thin => "偏薄",
            Self::Fail => "不合格",
        }
    }
}

/// Inputs for the fixed settlement schedule.
#[derive(Debug, Clone)]
pub struct SettlementRewardInput<'a> {
    pub budget: i64,
    pub capability: &'a str,
    pub mismatch: bool,
    pub declared_success: bool,
    pub outcome: &'a str,
}

/// Computed payout — write these to ledger / tender snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettlementReward {
    pub grade: QualityGrade,
    pub gold: i64,
    pub reputation: i32,
    pub grant_title: bool,
    pub title_tag: Option<String>,
    pub note: String,
}

/// Base tender budgets by capability (also used by title → field suggestion).
pub fn base_budget_for_capability(capability: &str) -> i64 {
    match capability {
        "coordination.route" => 20,
        "research.web" => 40,
        "document.create" => 50,
        "code.repo.read" => 60,
        _ => 30,
    }
}

/// Performance title unlocked only on Pass+ success.
pub fn title_for_capability(capability: &str) -> &'static str {
    match capability {
        "coordination.route" => "quick-wit",
        "research.web" => "scout",
        "document.create" => "scribe",
        "code.repo.read" => "patcher",
        _ => "finisher",
    }
}

/// Grade an outcome string. Pure — no I/O.
pub fn grade_outcome(outcome: &str, declared_success: bool) -> QualityGrade {
    if !declared_success {
        return QualityGrade::Fail;
    }
    let trimmed = outcome.trim();
    if trimmed.is_empty() {
        return QualityGrade::Fail;
    }
    if looks_like_failure(trimmed) {
        return QualityGrade::Fail;
    }
    let chars = trimmed.chars().count();
    // Short numeric / formula answers (e.g. `2`, `42`, `3.14`) count as excellent.
    if is_concise_factual(trimmed) {
        return QualityGrade::Excellent;
    }
    if chars < 6 {
        return QualityGrade::Thin;
    }
    if chars >= 120 {
        return QualityGrade::Excellent;
    }
    QualityGrade::Pass
}

/// Gold + reputation + title decision from the schedule tables below.
///
/// # Schedule (match / in-capability)
/// | Grade     | Gold            | Reputation |
/// |-----------|-----------------|------------|
/// | Excellent | 100% budget     | +12        |
/// | Pass      | 100% budget     | +8         |
/// | Thin      | 70% budget      | +3         |
/// | Fail      | 0               | −5         |
///
/// Mismatch (out-of-capability) success pays the same gold but higher rep
/// (harder job): Excellent +18 / Pass +14 / Thin +6. Fail mismatch: −18.
/// Fail + mismatch also applies a gold penalty of 25% budget (capped later
/// by settle against available balance).
pub fn compute_settlement_reward(input: SettlementRewardInput<'_>) -> SettlementReward {
    let budget = input.budget.max(0);
    let grade = grade_outcome(input.outcome, input.declared_success);
    let (gold_bps, rep_match, rep_mismatch) = match grade {
        QualityGrade::Excellent => (10_000, 12, 18),
        QualityGrade::Pass => (10_000, 8, 14),
        QualityGrade::Thin => (7_000, 3, 6),
        QualityGrade::Fail => (0, -5, -18),
    };
    let reputation = if input.mismatch {
        rep_mismatch
    } else {
        rep_match
    };
    let gold = budget.saturating_mul(gold_bps as i64) / 10_000;
    let grant_title = matches!(grade, QualityGrade::Excellent | QualityGrade::Pass);
    let title_tag = if grant_title {
        Some(title_for_capability(input.capability).to_string())
    } else {
        None
    };
    let match_label = if input.mismatch {
        "跨能力"
    } else {
        "能力匹配"
    };
    let note = format!(
        "{} · {} · ¤{} · 声望 {:+}{}",
        grade.label_zh(),
        match_label,
        gold,
        reputation,
        title_tag
            .as_deref()
            .map(|t| format!(" · 头衔 {t}"))
            .unwrap_or_default()
    );
    SettlementReward {
        grade,
        gold,
        reputation,
        grant_title,
        title_tag,
        note,
    }
}

/// Fail+mismatch gold clawback fraction (basis points of budget).
pub const FAIL_MISMATCH_PENALTY_BPS: u32 = 2_500;

fn looks_like_failure(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    const NEEDLES: &[&str] = &[
        "error:",
        "failed",
        "failure",
        "cannot complete",
        "unable to",
        "i can't",
        "i cannot",
        "抱歉",
        "无法完成",
        "做不到",
        "失败",
        "出错",
    ];
    NEEDLES.iter().any(|n| {
        if n.is_ascii() {
            lower.contains(n)
        } else {
            text.contains(n)
        }
    })
}

fn is_concise_factual(text: &str) -> bool {
    let t = text.trim();
    let chars = t.chars().count();
    if chars == 0 || chars > 48 {
        return false;
    }
    // Pure number / simple arithmetic result (e.g. `2`, `42`, `3.14`, `1/2`).
    let alnum: String = t
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | '/' | '=' | '%'))
        .collect();
    alnum.chars().count() == chars && t.chars().any(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grades_math_answer_excellent() {
        assert_eq!(grade_outcome("2", true), QualityGrade::Excellent);
        assert_eq!(grade_outcome("42", true), QualityGrade::Excellent);
    }

    #[test]
    fn grades_empty_and_error_as_fail() {
        assert_eq!(grade_outcome("", true), QualityGrade::Fail);
        assert_eq!(grade_outcome("Error: boom", true), QualityGrade::Fail);
        assert_eq!(grade_outcome("没法完成", false), QualityGrade::Fail);
        assert_eq!(
            grade_outcome("⚠️ 📝 **处理文档** · 失败 · `tool` · failed", true),
            QualityGrade::Fail
        );
    }

    #[test]
    fn schedule_scales_gold_and_rep() {
        let excellent = compute_settlement_reward(SettlementRewardInput {
            budget: 40,
            capability: "research.web",
            mismatch: false,
            declared_success: true,
            outcome: "2",
        });
        assert_eq!(excellent.grade, QualityGrade::Excellent);
        assert_eq!(excellent.gold, 40);
        assert_eq!(excellent.reputation, 12);
        assert!(excellent.grant_title);
        assert_eq!(excellent.title_tag.as_deref(), Some("scout"));

        let thin = compute_settlement_reward(SettlementRewardInput {
            budget: 40,
            capability: "research.web",
            mismatch: false,
            declared_success: true,
            outcome: "嗯",
        });
        assert_eq!(thin.grade, QualityGrade::Thin);
        assert_eq!(thin.gold, 28);
        assert_eq!(thin.reputation, 3);
        assert!(!thin.grant_title);

        let fail = compute_settlement_reward(SettlementRewardInput {
            budget: 40,
            capability: "research.web",
            mismatch: true,
            declared_success: false,
            outcome: "failed",
        });
        assert_eq!(fail.grade, QualityGrade::Fail);
        assert_eq!(fail.gold, 0);
        assert_eq!(fail.reputation, -18);
    }

    #[test]
    fn base_budgets_are_stable() {
        assert_eq!(base_budget_for_capability("coordination.route"), 20);
        assert_eq!(base_budget_for_capability("research.web"), 40);
        assert_eq!(base_budget_for_capability("document.create"), 50);
    }
}
