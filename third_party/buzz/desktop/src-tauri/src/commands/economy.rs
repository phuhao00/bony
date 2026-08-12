use tauri::{AppHandle, Manager};

use crate::{
    app_state::AppState,
    managed_agents::{
        load_managed_agents, EconomyAgentSnapshot, EconomyWalletView, OrgSnapshot, TenderSnapshot,
    },
};

#[tauri::command]
pub async fn economy_get_leaderboard(
    app: AppHandle,
    limit: Option<u32>,
) -> Result<Vec<EconomyAgentSnapshot>, String> {
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let records = load_managed_agents(&app)?;
        let known: Vec<(String, String)> = records
            .iter()
            .map(|record| (record.pubkey.clone(), record.name.clone()))
            .collect();
        crate::managed_agents::economy::get_leaderboard(&known, limit.map(|n| n as usize))
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[tauri::command]
pub async fn economy_get_wallet(
    pubkey_or_name: String,
) -> Result<Option<EconomyWalletView>, String> {
    tokio::task::spawn_blocking(move || crate::managed_agents::economy::get_wallet(&pubkey_or_name))
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[tauri::command]
pub async fn economy_list_orgs() -> Result<Vec<OrgSnapshot>, String> {
    tokio::task::spawn_blocking(crate::managed_agents::economy::list_orgs)
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[tauri::command]
pub async fn economy_create_org(
    name: String,
    founder_pubkey: String,
    founder_name: Option<String>,
    tags: Option<Vec<String>>,
) -> Result<OrgSnapshot, String> {
    tokio::task::spawn_blocking(move || {
        crate::managed_agents::economy::create_org(
            &name,
            &founder_pubkey,
            founder_name.as_deref(),
            tags,
        )
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[tauri::command]
pub async fn economy_join_org(
    org_id: String,
    member_pubkey: String,
    member_name: Option<String>,
) -> Result<OrgSnapshot, String> {
    tokio::task::spawn_blocking(move || {
        crate::managed_agents::economy::join_org(&org_id, &member_pubkey, member_name.as_deref())
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[tauri::command]
pub async fn economy_leave_org(
    org_id: String,
    member_pubkey: String,
) -> Result<OrgSnapshot, String> {
    tokio::task::spawn_blocking(move || {
        crate::managed_agents::economy::leave_org(&org_id, &member_pubkey)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[tauri::command]
pub async fn economy_list_tenders(status: Option<String>) -> Result<Vec<TenderSnapshot>, String> {
    tokio::task::spawn_blocking(move || {
        crate::managed_agents::economy::list_tenders(status.as_deref())
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[tauri::command]
pub async fn economy_publish_tender(
    title: String,
    capability: String,
    budget: i64,
    task_ref: String,
) -> Result<TenderSnapshot, String> {
    tokio::task::spawn_blocking(move || {
        crate::managed_agents::economy::publish_tender(&title, &capability, budget, &task_ref)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[tauri::command]
pub async fn economy_admin_adjust_balance(
    pubkey: String,
    name: Option<String>,
    delta: i64,
    note: Option<String>,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        crate::managed_agents::economy::admin_adjust_balance(
            &pubkey,
            name.as_deref(),
            delta,
            note.as_deref(),
        )
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[tauri::command]
pub async fn economy_admin_adjust_reputation(
    pubkey: String,
    name: Option<String>,
    delta: i32,
    note: Option<String>,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        crate::managed_agents::economy::admin_adjust_reputation(
            &pubkey,
            name.as_deref(),
            delta,
            note.as_deref(),
        )
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[tauri::command]
pub async fn economy_admin_set_tags(
    pubkey: String,
    name: Option<String>,
    tags: Vec<String>,
    note: Option<String>,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        crate::managed_agents::economy::admin_set_tags(
            &pubkey,
            name.as_deref(),
            tags,
            note.as_deref(),
        )
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[tauri::command]
pub async fn economy_admin_grant_achievement(
    pubkey: String,
    name: Option<String>,
    achievement_id: String,
    gold: i64,
    reputation: i32,
    note: Option<String>,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        crate::managed_agents::economy::admin_grant_achievement(
            &pubkey,
            name.as_deref(),
            &achievement_id,
            gold,
            reputation,
            note.as_deref(),
        )
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}
