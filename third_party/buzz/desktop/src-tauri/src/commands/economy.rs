use tauri::{AppHandle, Manager, State};

use crate::{
    app_state::AppState,
    commands::economy_fulfill::{fulfill_awarded_tender, fulfill_many},
    managed_agents::{
        load_managed_agents, record_capabilities, EconomyAgentSnapshot, EconomyWalletView,
        OrgSnapshot, TenderSnapshot, TenderSuggestion,
    },
};

fn known_roster(app: &AppHandle) -> Result<Vec<buzz_economy::RosterAgent>, String> {
    let state = app.state::<AppState>();
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let records = load_managed_agents(app)?;
    let known: Vec<(String, String, Vec<String>)> = records
        .iter()
        .map(|record| {
            (
                record.pubkey.clone(),
                record.name.clone(),
                record_capabilities(record),
            )
        })
        .collect();
    Ok(crate::managed_agents::economy::roster_from_known(&known))
}

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
pub async fn economy_suggest_tender(title: String) -> Result<TenderSuggestion, String> {
    Ok(crate::managed_agents::economy::suggest_tender(&title))
}

#[tauri::command]
pub async fn economy_publish_tender(
    app: AppHandle,
    state: State<'_, AppState>,
    title: String,
    capability: Option<String>,
    budget: Option<i64>,
    task_ref: String,
) -> Result<TenderSnapshot, String> {
    let app_for_block = app.clone();
    let tender = tokio::task::spawn_blocking(move || {
        let agents = known_roster(&app_for_block)?;
        crate::managed_agents::economy::publish_tender(
            &title,
            capability.as_deref(),
            budget,
            &task_ref,
            &agents,
        )
        .map(|result| result.tender)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))??;
    Ok(fulfill_awarded_tender(&app, &state, tender).await)
}

#[tauri::command]
pub async fn economy_invite_tender_bids(
    app: AppHandle,
    state: State<'_, AppState>,
    tender_id: String,
) -> Result<TenderSnapshot, String> {
    let app_for_block = app.clone();
    let tender = tokio::task::spawn_blocking(move || {
        let agents = known_roster(&app_for_block)?;
        crate::managed_agents::economy::invite_tender_bids(&tender_id, &agents)
            .map(|result| result.tender)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))??;
    Ok(fulfill_awarded_tender(&app, &state, tender).await)
}

#[tauri::command]
pub async fn economy_resolve_tender(
    app: AppHandle,
    state: State<'_, AppState>,
    tender_id: String,
) -> Result<TenderSnapshot, String> {
    let app_for_block = app.clone();
    let tender = tokio::task::spawn_blocking(move || {
        let agents = known_roster(&app_for_block)?;
        crate::managed_agents::economy::resolve_tender(&tender_id, &agents)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))??;
    Ok(fulfill_awarded_tender(&app, &state, tender).await)
}

#[tauri::command]
pub async fn economy_sweep_tenders(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<TenderSnapshot>, String> {
    let app_for_block = app.clone();
    let finished = tokio::task::spawn_blocking(move || {
        let agents = known_roster(&app_for_block)?;
        crate::managed_agents::economy::sweep_open_tenders(&agents)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))??;
    Ok(fulfill_many(&app, &state, finished).await)
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
