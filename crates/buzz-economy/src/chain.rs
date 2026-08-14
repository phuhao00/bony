//! Exclusive-locked hash-chained JSONL append + verify.
//!
//! Each line stores `prev_hash` / `hash` (hex blake3). Hash input is
//! `prev_hash_bytes ‖ canonical_json(entry without hash fields)`.

use crate::error::EconomyError;
use fs2::FileExt;
use serde::{de::DeserializeOwned, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::Path;

pub const GENESIS_HASH_HEX: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainReport {
    pub ok: bool,
    pub entries: usize,
    pub tip_hash: String,
    pub broken_at: Option<usize>,
    pub reason: Option<String>,
}

/// Append `value` as a hash-chained JSONL row. Mutates the value's
/// `prev_hash` / `hash` fields (must be present as Option<String> via
/// [`ChainedRow`]).
pub fn append_chained<T>(path: &Path, value: &T) -> Result<T, EconomyError>
where
    T: Serialize + DeserializeOwned + ChainedRow + Clone,
{
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| EconomyError::io("create dir for", path, e))?;
    }

    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)
        .map_err(|e| EconomyError::io("open", path, e))?;

    file.lock_exclusive()
        .map_err(|e| EconomyError::io("lock", path, e))?;

    let result = (|| {
        let prev = read_tip_hash_locked(&mut file)?;
        let mut row = value.clone();
        row.set_prev_hash(Some(prev.clone()));
        row.set_hash(None);
        let hash = compute_row_hash(&row, &prev)?;
        row.set_hash(Some(hash));
        let line = serde_json::to_string(&row)
            .map_err(|e| EconomyError::Serialize(format!("serialize: {e}")))?;
        // Seek to end after reading tip (read may have moved cursor).
        file.seek(SeekFrom::End(0))
            .map_err(|e| EconomyError::io("seek end", path, e))?;
        writeln!(file, "{line}").map_err(|e| EconomyError::io("append", path, e))?;
        file.flush()
            .map_err(|e| EconomyError::io("flush", path, e))?;
        Ok(row)
    })();

    let _ = file.unlock();
    result
}

pub fn last_hash(path: &Path) -> Result<String, EconomyError> {
    if !path.exists() {
        return Ok(GENESIS_HASH_HEX.to_string());
    }
    let mut file = OpenOptions::new()
        .read(true)
        .open(path)
        .map_err(|e| EconomyError::io("open", path, e))?;
    file.lock_shared()
        .map_err(|e| EconomyError::io("lock shared", path, e))?;
    let tip = read_tip_hash_locked(&mut file);
    let _ = file.unlock();
    tip
}

pub fn verify_chain<T>(path: &Path) -> Result<ChainReport, EconomyError>
where
    T: Serialize + DeserializeOwned + ChainedRow,
{
    if !path.exists() {
        return Ok(ChainReport {
            ok: true,
            entries: 0,
            tip_hash: GENESIS_HASH_HEX.to_string(),
            broken_at: None,
            reason: None,
        });
    }
    let file = fs::File::open(path).map_err(|e| EconomyError::io("open", path, e))?;
    let lines: Vec<String> = BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter(|l| !l.trim().is_empty())
        .collect();

    let mut expected_prev = GENESIS_HASH_HEX.to_string();
    let mut tip = expected_prev.clone();
    for (idx, line) in lines.iter().enumerate() {
        let row: T = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => {
                return Ok(ChainReport {
                    ok: false,
                    entries: idx,
                    tip_hash: tip,
                    broken_at: Some(idx + 1),
                    reason: Some(format!("parse error: {e}")),
                });
            }
        };
        let stored_prev = row.prev_hash().unwrap_or(GENESIS_HASH_HEX);
        if stored_prev != expected_prev {
            return Ok(ChainReport {
                ok: false,
                entries: idx,
                tip_hash: tip,
                broken_at: Some(idx + 1),
                reason: Some(format!(
                    "prev_hash mismatch: stored={stored_prev} expected={expected_prev}"
                )),
            });
        }
        let computed = match compute_row_hash(&row, &expected_prev) {
            Ok(h) => h,
            Err(e) => {
                return Ok(ChainReport {
                    ok: false,
                    entries: idx,
                    tip_hash: tip,
                    broken_at: Some(idx + 1),
                    reason: Some(e.to_string()),
                });
            }
        };
        let stored_hash = row.hash().unwrap_or("");
        if stored_hash != computed {
            return Ok(ChainReport {
                ok: false,
                entries: idx,
                tip_hash: tip,
                broken_at: Some(idx + 1),
                reason: Some(format!(
                    "hash mismatch: stored={stored_hash} computed={computed}"
                )),
            });
        }
        tip = computed.clone();
        expected_prev = computed;
    }
    Ok(ChainReport {
        ok: true,
        entries: lines.len(),
        tip_hash: tip,
        broken_at: None,
        reason: None,
    })
}

pub trait ChainedRow {
    fn prev_hash(&self) -> Option<&str>;
    fn hash(&self) -> Option<&str>;
    fn set_prev_hash(&mut self, v: Option<String>);
    fn set_hash(&mut self, v: Option<String>);
}

fn read_tip_hash_locked(file: &mut fs::File) -> Result<String, EconomyError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|e| EconomyError::Io(format!("seek start: {e}")))?;
    let mut last_hash = GENESIS_HASH_HEX.to_string();
    for line in BufReader::new(
        file.try_clone()
            .map_err(|e| EconomyError::Io(e.to_string()))?,
    )
    .lines()
    .map_while(Result::ok)
    {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(h) = v.get("hash").and_then(|x| x.as_str()) {
                if !h.is_empty() {
                    last_hash = h.to_string();
                }
            }
        }
    }
    Ok(last_hash)
}

fn compute_row_hash<T: Serialize + ChainedRow>(
    row: &T,
    prev_hash: &str,
) -> Result<String, EconomyError> {
    let mut value =
        serde_json::to_value(row).map_err(|e| EconomyError::Serialize(format!("to_value: {e}")))?;
    if let Some(obj) = value.as_object_mut() {
        obj.remove("hash");
        obj.insert(
            "prev_hash".into(),
            serde_json::Value::String(prev_hash.to_string()),
        );
    }
    let canonical =
        canonical_json(&value).map_err(|e| EconomyError::Serialize(format!("canonical: {e}")))?;
    let mut hasher = blake3::Hasher::new();
    let prev_bytes = hex::decode(prev_hash).unwrap_or_else(|_| vec![0u8; 32]);
    hasher.update(&prev_bytes);
    hasher.update(canonical.as_bytes());
    Ok(hex::encode(hasher.finalize().as_bytes()))
}

fn canonical_json(value: &serde_json::Value) -> Result<String, serde_json::Error> {
    use serde_json::Value;
    use std::collections::BTreeMap;
    match value {
        Value::Object(map) => {
            let sorted: BTreeMap<&str, &Value> = map.iter().map(|(k, v)| (k.as_str(), v)).collect();
            let mut out = String::from("{");
            let mut first = true;
            for (k, v) in &sorted {
                if !first {
                    out.push(',');
                }
                first = false;
                out.push_str(&serde_json::to_string(k)?);
                out.push(':');
                out.push_str(&canonical_json(v)?);
            }
            out.push('}');
            Ok(out)
        }
        Value::Array(arr) => {
            let mut out = String::from("[");
            let mut first = true;
            for v in arr {
                if !first {
                    out.push(',');
                }
                first = false;
                out.push_str(&canonical_json(v)?);
            }
            out.push(']');
            Ok(out)
        }
        other => serde_json::to_string(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{LedgerEntry, LedgerKind};
    use tempfile::tempdir;

    #[test]
    fn append_and_verify_chain() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("ledger.jsonl");
        let e1 = LedgerEntry {
            ts: "t1".into(),
            pubkey: "pk1".into(),
            kind: LedgerKind::Payout,
            amount: 10,
            reputation_delta: 1,
            task_ref: None,
            note: None,
            name: Some("A".into()),
            tags: Vec::new(),
            achievements: Vec::new(),
            capability_grants: Vec::new(),
            prev_hash: None,
            hash: None,
        };
        let written = append_chained(&path, &e1).unwrap();
        assert!(written.hash.is_some());
        assert_eq!(written.prev_hash.as_deref(), Some(GENESIS_HASH_HEX));

        let e2 = LedgerEntry {
            ts: "t2".into(),
            amount: 5,
            ..e1.clone()
        };
        append_chained(&path, &e2).unwrap();
        let report = verify_chain::<LedgerEntry>(&path).unwrap();
        assert!(report.ok, "{report:?}");
        assert_eq!(report.entries, 2);
    }
}
