use crate::chain::append_chained;
use crate::error::EconomyError;
use crate::paths::EconomyPaths;
use crate::types::{OrgEvent, OrgEventKind, RosterAgent};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OrgSnapshot {
    pub org_id: String,
    pub name: String,
    pub member_pubkeys: Vec<String>,
    pub member_names: Vec<String>,
    pub tags: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OrgCreateParams {
    pub name: String,
    pub founder_pubkey: String,
    pub founder_name: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OrgJoinParams {
    pub org_id: String,
    pub member_pubkey: String,
    pub member_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OrgLeaveParams {
    pub org_id: String,
    pub member_pubkey: String,
}

pub fn create_org(paths: &EconomyPaths, p: OrgCreateParams) -> Result<OrgSnapshot, EconomyError> {
    let name = p.name.trim();
    let founder = p.founder_pubkey.trim();
    if name.is_empty() {
        return Err(EconomyError::invalid("org name must not be empty"));
    }
    if founder.is_empty() {
        return Err(EconomyError::invalid("founder_pubkey must not be empty"));
    }
    let slug = slugify(name);
    let org_id = format!("org:{slug}");
    let existing = fold_orgs(paths)?;
    if existing.contains_key(&org_id) {
        return Err(EconomyError::invalid(format!(
            "organization {org_id} already exists"
        )));
    }
    let tags = p.tags.unwrap_or_default();
    let ts = Utc::now().to_rfc3339();
    append_chained(
        &paths.orgs,
        &OrgEvent {
            ts: ts.clone(),
            kind: OrgEventKind::OrgCreate,
            org_id: org_id.clone(),
            org_name: name.to_string(),
            member_pubkey: Some(founder.to_string()),
            member_name: p.founder_name.clone(),
            tags: tags.clone(),
            prev_hash: None,
            hash: None,
        },
    )?;
    Ok(OrgSnapshot {
        org_id,
        name: name.to_string(),
        member_pubkeys: vec![founder.to_string()],
        member_names: vec![p.founder_name.unwrap_or_else(|| founder.to_string())],
        tags,
        created_at: ts,
    })
}

pub fn join_org(paths: &EconomyPaths, p: OrgJoinParams) -> Result<OrgSnapshot, EconomyError> {
    let org_id = normalize_org_id(p.org_id.trim());
    let member = p.member_pubkey.trim();
    if org_id.is_empty() || member.is_empty() {
        return Err(EconomyError::invalid(
            "org_id and member_pubkey must not be empty",
        ));
    }
    let orgs = fold_orgs(paths)?;
    let org = orgs
        .get(&org_id)
        .ok_or_else(|| EconomyError::invalid(format!("unknown org {org_id}")))?;
    if org
        .member_pubkeys
        .iter()
        .any(|m| m.eq_ignore_ascii_case(member))
    {
        return Ok(org.clone());
    }
    append_chained(
        &paths.orgs,
        &OrgEvent {
            ts: Utc::now().to_rfc3339(),
            kind: OrgEventKind::OrgJoin,
            org_id: org_id.clone(),
            org_name: org.name.clone(),
            member_pubkey: Some(member.to_string()),
            member_name: p.member_name.clone(),
            tags: Vec::new(),
            prev_hash: None,
            hash: None,
        },
    )?;
    fold_orgs(paths)?
        .get(&org_id)
        .cloned()
        .ok_or_else(|| EconomyError::invalid("org vanished after join"))
}

pub fn leave_org(paths: &EconomyPaths, p: OrgLeaveParams) -> Result<OrgSnapshot, EconomyError> {
    let org_id = normalize_org_id(p.org_id.trim());
    let member = p.member_pubkey.trim();
    if org_id.is_empty() || member.is_empty() {
        return Err(EconomyError::invalid(
            "org_id and member_pubkey must not be empty",
        ));
    }
    let orgs = fold_orgs(paths)?;
    let org = orgs
        .get(&org_id)
        .ok_or_else(|| EconomyError::invalid(format!("unknown org {org_id}")))?;
    append_chained(
        &paths.orgs,
        &OrgEvent {
            ts: Utc::now().to_rfc3339(),
            kind: OrgEventKind::OrgLeave,
            org_id: org_id.clone(),
            org_name: org.name.clone(),
            member_pubkey: Some(member.to_string()),
            member_name: None,
            tags: Vec::new(),
            prev_hash: None,
            hash: None,
        },
    )?;
    fold_orgs(paths)?
        .get(&org_id)
        .cloned()
        .ok_or_else(|| EconomyError::invalid("org vanished after leave"))
}

pub fn list_orgs(paths: &EconomyPaths) -> Result<Vec<OrgSnapshot>, EconomyError> {
    let mut rows: Vec<OrgSnapshot> = fold_orgs(paths)?.into_values().collect();
    rows.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
    });
    Ok(rows)
}

/// Union of member capabilities for org bidding.
pub fn org_member_capabilities(org: &OrgSnapshot, agents: &[RosterAgent]) -> Vec<String> {
    let mut caps = BTreeSet::new();
    for pk in &org.member_pubkeys {
        if let Some(agent) = agents.iter().find(|a| a.pubkey.eq_ignore_ascii_case(pk)) {
            for c in &agent.capabilities {
                caps.insert(c.clone());
            }
        }
    }
    caps.into_iter().collect()
}

pub fn fold_orgs(paths: &EconomyPaths) -> Result<BTreeMap<String, OrgSnapshot>, EconomyError> {
    let file = match fs::File::open(&paths.orgs) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(EconomyError::io("open organizations", &paths.orgs, e)),
    };
    let mut map: BTreeMap<String, OrgSnapshot> = BTreeMap::new();
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(ev) = serde_json::from_str::<OrgEvent>(&line) else {
            continue;
        };
        match ev.kind {
            OrgEventKind::OrgCreate => {
                let mut members = Vec::new();
                let mut names = Vec::new();
                if let Some(pk) = &ev.member_pubkey {
                    members.push(pk.clone());
                    names.push(ev.member_name.clone().unwrap_or_else(|| pk.clone()));
                }
                map.insert(
                    ev.org_id.clone(),
                    OrgSnapshot {
                        org_id: ev.org_id,
                        name: ev.org_name,
                        member_pubkeys: members,
                        member_names: names,
                        tags: ev.tags,
                        created_at: ev.ts,
                    },
                );
            }
            OrgEventKind::OrgJoin => {
                if let Some(org) = map.get_mut(&ev.org_id) {
                    if let Some(pk) = &ev.member_pubkey {
                        if !org.member_pubkeys.iter().any(|m| m == pk) {
                            org.member_pubkeys.push(pk.clone());
                            org.member_names
                                .push(ev.member_name.clone().unwrap_or_else(|| pk.clone()));
                        }
                    }
                }
            }
            OrgEventKind::OrgLeave => {
                if let Some(org) = map.get_mut(&ev.org_id) {
                    if let Some(pk) = &ev.member_pubkey {
                        if let Some(idx) = org
                            .member_pubkeys
                            .iter()
                            .position(|m| m.eq_ignore_ascii_case(pk))
                        {
                            org.member_pubkeys.remove(idx);
                            if idx < org.member_names.len() {
                                org.member_names.remove(idx);
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(map)
}

fn normalize_org_id(raw: &str) -> String {
    if raw.starts_with("org:") {
        raw.to_string()
    } else {
        format!("org:{}", slugify(raw))
    }
}

fn slugify(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if ch == '-' || ch == '_' || ch.is_whitespace() {
            if !out.ends_with('-') && !out.is_empty() {
                out.push('-');
            }
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "unnamed".into()
    } else {
        trimmed
    }
}
