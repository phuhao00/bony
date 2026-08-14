//! Room task-log memory — append-only JSONL store of what got done, by whom,
//! and how it went, read back by keyword at the start of a new task so the
//! room "remembers" past preferences/pitfalls without a black-box model.
//!
//! Design: `docs/buzz-room-agent-orchestration-plan.md` §6. Grok is the sole
//! writer (single authoritative implementation point — the coordinator sees
//! every task chain, so it is the one agent that can always append a summary
//! regardless of which specialist(s) did the work).

use crate::shell::SharedState;
use chrono::Utc;
use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

const DEFAULT_SEARCH_LIMIT: u32 = 5;
const MAX_SEARCH_LIMIT: u32 = 20;
/// Bound the number of historical lines scanned per search so an ever-growing
/// task log cannot make a single tool call unbounded (mirrors buzz-db's
/// `LIST_MAX_LIMIT` bounded-query convention).
const MAX_SCAN_LINES: usize = 5000;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TaskLogEntry {
    /// RFC3339 UTC timestamp, stamped server-side at append time.
    pub ts: String,
    pub topic: String,
    #[serde(default)]
    pub agents: Vec<String>,
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Free-form outcome, e.g. "done" / "blocked" / "failed". Absent implies "done".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Who/why when status is "blocked" or "failed" — failure attribution,
    /// not a raw error dump.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryAppendParams {
    /// Free-text topic/subject of the task. This is the primary search key —
    /// write it the way a future request about the same thing would phrase it.
    pub topic: String,
    /// Agents involved, in execution order (e.g. `["ZeroClaw", "DocSmith"]`).
    #[serde(default)]
    pub agents: Vec<String>,
    /// Produced artifact paths or links, if any.
    #[serde(default)]
    pub outputs: Vec<String>,
    /// The user's in-the-moment feedback, verbatim, if any was given.
    #[serde(default)]
    pub feedback: Option<String>,
    /// One-sentence lesson or preference for next time. Factual summary only —
    /// never a secret, key, or large excerpt of the original content.
    #[serde(default)]
    pub notes: Option<String>,
    /// Outcome: "done" (default when omitted), "blocked", or "failed".
    #[serde(default)]
    pub status: Option<String>,
    /// Who/why when blocked or failed (failure attribution).
    #[serde(default)]
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemorySearchParams {
    /// Keyword(s) to match against topic/notes/agents/outputs, case-insensitive substring.
    pub query: String,
    /// Max entries to return, most recent first. Defaults to 5, capped at 20.
    #[serde(default)]
    pub limit: Option<u32>,
}

pub fn append(state: &SharedState, p: MemoryAppendParams) -> Result<String, ErrorData> {
    let topic = p.topic.trim();
    if topic.is_empty() {
        return Err(ErrorData::invalid_params("topic must not be empty", None));
    }
    let entry = TaskLogEntry {
        ts: Utc::now().to_rfc3339(),
        topic: topic.to_string(),
        agents: p.agents,
        outputs: p.outputs,
        feedback: non_empty(p.feedback),
        notes: non_empty(p.notes),
        status: non_empty(p.status),
        blocked_reason: non_empty(p.blocked_reason),
    };

    let path = memory_path(state);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| io_err("create memory dir for", &path, e))?;
    }
    let line = serde_json::to_string(&entry)
        .map_err(|e| ErrorData::internal_error(format!("serialize task-log entry: {e}"), None))?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| io_err("open memory file", &path, e))?;
    writeln!(file, "{line}").map_err(|e| io_err("append memory file", &path, e))?;

    Ok(format!("appended task-log entry to {}", path.display()))
}

pub fn search(state: &SharedState, p: MemorySearchParams) -> Result<String, ErrorData> {
    let query = p.query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return Err(ErrorData::invalid_params("query must not be empty", None));
    }
    let limit = p
        .limit
        .unwrap_or(DEFAULT_SEARCH_LIMIT)
        .clamp(1, MAX_SEARCH_LIMIT) as usize;

    let path = memory_path(state);
    let file = match fs::File::open(&path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(format!("no task-log memory yet at {}", path.display()));
        }
        Err(e) => return Err(io_err("open memory file", &path, e)),
    };

    let lines: Vec<String> = BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter(|l| !l.trim().is_empty())
        .collect();
    // Bound the scan window to the most recent MAX_SCAN_LINES entries.
    let start = lines.len().saturating_sub(MAX_SCAN_LINES);

    let mut matches: Vec<TaskLogEntry> = Vec::new();
    for line in lines[start..].iter().rev() {
        // Tolerate hand-edited or partially-written lines rather than
        // failing the whole search on one bad row.
        let Ok(entry) = serde_json::from_str::<TaskLogEntry>(line) else {
            continue;
        };
        if entry_matches(&entry, &query) {
            matches.push(entry);
            if matches.len() >= limit {
                break;
            }
        }
    }

    if matches.is_empty() {
        return Ok(format!("no task-log entries match \"{}\"", p.query));
    }
    let mut out = format!(
        "{} matching task-log entr{} (most recent first):\n",
        matches.len(),
        if matches.len() == 1 { "y" } else { "ies" }
    );
    for entry in &matches {
        out.push_str(&format_entry(entry));
        out.push('\n');
    }
    Ok(out)
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryPreferencesParams {
    /// Optional topic keyword filter (same substring rules as memory_search).
    /// Empty = scan recent task-log for repeated notes/feedback globally.
    #[serde(default)]
    pub query: Option<String>,
    /// Minimum times a normalized preference must appear to be reported. Default 2.
    #[serde(default)]
    pub min_count: Option<u32>,
    /// Max preferences to return. Default 10, capped at 20.
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Scan the task-log for notes/feedback that repeat, so the coordinator can
/// harden recurring preferences into routing/format decisions (Phase 4).
pub fn preferences_extract(
    state: &SharedState,
    p: MemoryPreferencesParams,
) -> Result<String, ErrorData> {
    let min_count = p.min_count.unwrap_or(2).max(2) as usize;
    let limit = p.limit.unwrap_or(10).clamp(1, MAX_SEARCH_LIMIT) as usize;
    let query = p
        .query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase());

    let path = memory_path(state);
    let file = match fs::File::open(&path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(format!("no task-log memory yet at {}", path.display()));
        }
        Err(e) => return Err(io_err("open memory file", &path, e)),
    };

    let lines: Vec<String> = BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter(|l| !l.trim().is_empty())
        .collect();
    let start = lines.len().saturating_sub(MAX_SCAN_LINES);

    // key = normalized preference text → (count, example agents, last topic)
    let mut tallies: std::collections::BTreeMap<String, PreferenceTally> =
        std::collections::BTreeMap::new();
    for line in &lines[start..] {
        let Ok(entry) = serde_json::from_str::<TaskLogEntry>(line) else {
            continue;
        };
        if let Some(ref q) = query {
            if !entry_matches(&entry, q) {
                continue;
            }
        }
        for raw in [&entry.notes, &entry.feedback]
            .into_iter()
            .flatten()
            .map(String::as_str)
        {
            let key = normalize_preference(raw);
            if key.is_empty() {
                continue;
            }
            let tally = tallies.entry(key).or_default();
            tally.count += 1;
            for agent in &entry.agents {
                if !tally.agents.iter().any(|a| a.eq_ignore_ascii_case(agent)) {
                    tally.agents.push(agent.clone());
                }
            }
            tally.last_topic = entry.topic.clone();
            tally.example = raw.trim().to_string();
        }
    }

    let mut ranked: Vec<(String, PreferenceTally)> = tallies
        .into_iter()
        .filter(|(_, tally)| tally.count >= min_count)
        .collect();
    ranked.sort_by(|left, right| {
        right
            .1
            .count
            .cmp(&left.1.count)
            .then_with(|| left.0.cmp(&right.0))
    });
    ranked.truncate(limit);

    if ranked.is_empty() {
        return Ok(format!(
            "no repeated preferences (min_count={min_count}){}",
            match query {
                Some(ref q) => format!(" matching \"{q}\""),
                None => String::new(),
            }
        ));
    }

    let mut out = format!(
        "{} repeated preference(s) (count ≥ {min_count}):\n",
        ranked.len()
    );
    for (_, tally) in &ranked {
        out.push_str(&format!(
            "- ×{} | {} | agents: {} | last_topic: {}\n",
            tally.count,
            tally.example,
            if tally.agents.is_empty() {
                "-".into()
            } else {
                tally.agents.join(", ")
            },
            tally.last_topic
        ));
    }
    out.push_str(
        "Use these as soft routing/format hints only — do not rewrite specialist prompts yourself.\n",
    );
    Ok(out)
}

#[derive(Default)]
struct PreferenceTally {
    count: usize,
    agents: Vec<String>,
    last_topic: String,
    example: String,
}

fn normalize_preference(raw: &str) -> String {
    raw.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn entry_matches(entry: &TaskLogEntry, query_lower: &str) -> bool {
    let haystack = format!(
        "{} {} {} {}",
        entry.topic,
        entry.notes.as_deref().unwrap_or_default(),
        entry.agents.join(" "),
        entry.outputs.join(" ")
    )
    .to_ascii_lowercase();
    haystack.contains(query_lower)
}

fn format_entry(entry: &TaskLogEntry) -> String {
    let mut s = format!("- [{}] {}", entry.ts, entry.topic);
    if !entry.agents.is_empty() {
        s.push_str(&format!(" | agents: {}", entry.agents.join(", ")));
    }
    if !entry.outputs.is_empty() {
        s.push_str(&format!(" | outputs: {}", entry.outputs.join(", ")));
    }
    if let Some(status) = &entry.status {
        s.push_str(&format!(" | status: {status}"));
    }
    if let Some(reason) = &entry.blocked_reason {
        s.push_str(&format!(" | blocked_reason: {reason}"));
    }
    if let Some(feedback) = &entry.feedback {
        s.push_str(&format!(" | feedback: {feedback}"));
    }
    if let Some(notes) = &entry.notes {
        s.push_str(&format!(" | notes: {notes}"));
    }
    s
}

fn non_empty(v: Option<String>) -> Option<String> {
    v.and_then(|s| {
        let t = s.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    })
}

fn io_err(action: &str, path: &Path, e: std::io::Error) -> ErrorData {
    ErrorData::internal_error(format!("{action} {}: {e}", path.display()), None)
}

/// Resolve the task-log path: explicit `BONY_ROOM_MEMORY_PATH` env override,
/// else `<home>/.bony-build/room-memory/task-log.jsonl`.
///
/// Home-anchored rather than server-cwd-relative: Grok's MCP server cwd
/// tracks whichever Coding Workspace project is currently selected (it can
/// change every session), while room memory is a single durable store meant
/// to be shared across every project and channel.
fn memory_path(state: &SharedState) -> PathBuf {
    if let Ok(p) = std::env::var("BONY_ROOM_MEMORY_PATH") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        return PathBuf::from(home)
            .join(".bony-build")
            .join("room-memory")
            .join("task-log.jsonl");
    }
    // Last-resort fallback for an exotic environment with neither
    // USERPROFILE nor HOME set.
    state
        .cwd
        .join(".bony-build")
        .join("room-memory")
        .join("task-log.jsonl")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // `memory_path` reads process-wide env vars; serialize the tests that
    // set BONY_ROOM_MEMORY_PATH so they cannot interleave with each other
    // (this crate's test binary runs tests on multiple threads by default).
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn make_state() -> SharedState {
        let shim = crate::shim::Shim::install().expect("shim install");
        SharedState::new(std::env::temp_dir(), shim).expect("state new")
    }

    fn with_memory_path<T>(path: &Path, f: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("BONY_ROOM_MEMORY_PATH", path);
        let result = f();
        std::env::remove_var("BONY_ROOM_MEMORY_PATH");
        result
    }

    #[test]
    fn append_then_search_round_trips() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("task-log.jsonl");
        with_memory_path(&path, || {
            let state = make_state();
            let out = append(
                &state,
                MemoryAppendParams {
                    topic: "今天AI资讯PDF".to_string(),
                    agents: vec!["ZeroClaw".to_string(), "DocSmith".to_string()],
                    outputs: vec!["docs/ai-news-2026-08-07.pdf".to_string()],
                    feedback: Some("排版满意".to_string()),
                    notes: Some("用户偏好紧凑排版".to_string()),
                    status: None,
                    blocked_reason: None,
                },
            )
            .expect("append ok");
            assert!(out.contains("task-log.jsonl"), "out: {out}");

            let found = search(
                &state,
                MemorySearchParams {
                    query: "AI资讯".to_string(),
                    limit: None,
                },
            )
            .expect("search ok");
            assert!(found.contains("1 matching"), "found: {found}");
            assert!(found.contains("ZeroClaw, DocSmith"), "found: {found}");
            assert!(found.contains("紧凑排版"), "found: {found}");
        });
    }

    #[test]
    fn search_returns_most_recent_first_and_respects_limit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("task-log.jsonl");
        with_memory_path(&path, || {
            let state = make_state();
            for i in 0..3 {
                append(
                    &state,
                    MemoryAppendParams {
                        topic: format!("widget task {i}"),
                        agents: vec![],
                        outputs: vec![],
                        feedback: None,
                        notes: None,
                        status: None,
                        blocked_reason: None,
                    },
                )
                .expect("append ok");
            }
            let found = search(
                &state,
                MemorySearchParams {
                    query: "widget".to_string(),
                    limit: Some(2),
                },
            )
            .expect("search ok");
            assert!(found.contains("2 matching"), "found: {found}");
            // Most recent (i=2) must appear before the older (i=1) entry.
            let pos2 = found.find("widget task 2").expect("has task 2");
            let pos1 = found.find("widget task 1").expect("has task 1");
            assert!(pos2 < pos1, "expected newest-first ordering: {found}");
            assert!(
                !found.contains("widget task 0"),
                "limit=2 must drop the oldest: {found}"
            );
        });
    }

    #[test]
    fn search_on_missing_file_is_not_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("does-not-exist.jsonl");
        with_memory_path(&path, || {
            let state = make_state();
            let found = search(
                &state,
                MemorySearchParams {
                    query: "anything".to_string(),
                    limit: None,
                },
            )
            .expect("search ok even when file is missing");
            assert!(found.contains("no task-log memory yet"), "found: {found}");
        });
    }

    #[test]
    fn append_rejects_empty_topic() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("task-log.jsonl");
        with_memory_path(&path, || {
            let state = make_state();
            let err = append(
                &state,
                MemoryAppendParams {
                    topic: "   ".to_string(),
                    agents: vec![],
                    outputs: vec![],
                    feedback: None,
                    notes: None,
                    status: None,
                    blocked_reason: None,
                },
            )
            .unwrap_err();
            assert!(format!("{err:?}").contains("topic must not be empty"));
        });
    }

    #[test]
    fn search_tolerates_malformed_lines() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("task-log.jsonl");
        std::fs::write(&path, "not json\n{\"ts\":\"x\",\"topic\":\"widget ok\"}\n").unwrap();
        with_memory_path(&path, || {
            let state = make_state();
            let found = search(
                &state,
                MemorySearchParams {
                    query: "widget".to_string(),
                    limit: None,
                },
            )
            .expect("search ok despite malformed line");
            assert!(found.contains("widget ok"), "found: {found}");
        });
    }

    #[test]
    fn preferences_extract_requires_repetition() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("task-log.jsonl");
        with_memory_path(&path, || {
            let state = make_state();
            for _ in 0..2 {
                append(
                    &state,
                    MemoryAppendParams {
                        topic: "PDF layout".into(),
                        agents: vec!["DocSmith".into()],
                        outputs: vec![],
                        feedback: None,
                        notes: Some("用户偏好紧凑排版".into()),
                        status: None,
                        blocked_reason: None,
                    },
                )
                .unwrap();
            }
            append(
                &state,
                MemoryAppendParams {
                    topic: "other".into(),
                    agents: vec![],
                    outputs: vec![],
                    feedback: None,
                    notes: Some("once only note".into()),
                    status: None,
                    blocked_reason: None,
                },
            )
            .unwrap();
            let found = preferences_extract(
                &state,
                MemoryPreferencesParams {
                    query: None,
                    min_count: Some(2),
                    limit: None,
                },
            )
            .expect("extract");
            assert!(found.contains("紧凑排版"), "found: {found}");
            assert!(!found.contains("once only note"), "found: {found}");
        });
    }
}
