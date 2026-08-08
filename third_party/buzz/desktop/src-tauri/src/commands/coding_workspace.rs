//! Local Coding Workspace project selection and recent-project persistence.
//!
//! The renderer receives validated descriptors only. Absolute paths are picked,
//! canonicalized, inspected, and persisted in Rust so future coding runtimes
//! (Grok, Codex, Claude Code, or custom ACP harnesses) share one project model.

use atomic_write_file::AtomicWriteFile;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;
use uuid::Uuid;

const HISTORY_FILE: &str = "coding-workspaces.json";
const MAX_RECENT_PROJECTS: usize = 12;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodingWorkspaceProject {
    pub id: String,
    pub name: String,
    pub path: String,
    pub repository_root: Option<String>,
    pub git_branch: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct CodingWorkspaceHistory {
    recent: Vec<PathBuf>,
}

fn history_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join(HISTORY_FILE))
        .map_err(|error| format!("resolve Coding Workspace data directory: {error}"))
}

fn load_history(path: &Path) -> CodingWorkspaceHistory {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

fn save_history(path: &Path, history: &CodingWorkspaceHistory) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Coding Workspace history path has no parent".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("create Coding Workspace data directory: {error}"))?;
    let payload = serde_json::to_vec_pretty(history)
        .map_err(|error| format!("serialize Coding Workspace history: {error}"))?;
    let mut file = AtomicWriteFile::open(path)
        .map_err(|error| format!("open Coding Workspace history: {error}"))?;
    file.write_all(&payload)
        .map_err(|error| format!("write Coding Workspace history: {error}"))?;
    file.commit()
        .map_err(|error| format!("commit Coding Workspace history: {error}"))
}

fn canonical_project_path(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("Coding Workspace project path must be absolute".to_string());
    }
    if path.to_string_lossy().chars().any(char::is_control) {
        return Err("Coding Workspace project path contains control characters".to_string());
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("project folder is not accessible: {error}"))?;
    if !canonical.is_dir() {
        return Err("Coding Workspace project path is not a directory".to_string());
    }
    if canonical.to_string_lossy().chars().any(char::is_control) {
        return Err("Coding Workspace project path contains control characters".to_string());
    }
    Ok(canonical)
}

fn repository_root(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|ancestor| ancestor.join(".git").exists())
        .map(Path::to_path_buf)
}

fn git_dir(repository: &Path) -> Option<PathBuf> {
    let dot_git = repository.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }
    let pointer = std::fs::read_to_string(dot_git).ok()?;
    let target = pointer.trim().strip_prefix("gitdir:")?.trim();
    let target = PathBuf::from(target);
    Some(if target.is_absolute() {
        target
    } else {
        repository.join(target)
    })
}

fn git_branch(repository: &Path) -> Option<String> {
    let head = std::fs::read_to_string(git_dir(repository)?.join("HEAD")).ok()?;
    let head = head.trim();
    if let Some(reference) = head.strip_prefix("ref:") {
        return reference
            .trim()
            .rsplit('/')
            .next()
            .filter(|value| !value.is_empty())
            .map(str::to_string);
    }
    (head.len() >= 8).then(|| format!("detached@{}", &head[..8]))
}

fn describe_project(path: &Path) -> Result<CodingWorkspaceProject, String> {
    let canonical = canonical_project_path(path)?;
    let repository_root = repository_root(&canonical);
    let name = canonical
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("Project")
        .to_string();
    let path_text = canonical.to_string_lossy().to_string();
    Ok(CodingWorkspaceProject {
        id: Uuid::new_v5(&Uuid::NAMESPACE_URL, path_text.as_bytes()).to_string(),
        name,
        path: path_text,
        repository_root: repository_root
            .as_ref()
            .map(|root| root.to_string_lossy().to_string()),
        git_branch: repository_root.as_deref().and_then(git_branch),
    })
}

fn remember_project(path: &Path, history: &mut CodingWorkspaceHistory) {
    history.recent.retain(|candidate| candidate != path);
    history.recent.insert(0, path.to_path_buf());
    history.recent.truncate(MAX_RECENT_PROJECTS);
}

#[tauri::command]
pub fn list_coding_workspace_projects(
    app: AppHandle,
) -> Result<Vec<CodingWorkspaceProject>, String> {
    let path = history_path(&app)?;
    let mut history = load_history(&path);
    let projects = history
        .recent
        .iter()
        .filter_map(|project| describe_project(project).ok())
        .collect::<Vec<_>>();
    history.recent = projects
        .iter()
        .map(|project| PathBuf::from(&project.path))
        .collect();
    save_history(&path, &history)?;
    Ok(projects)
}

#[tauri::command]
pub async fn open_coding_workspace_project(
    app: AppHandle,
    path: Option<String>,
) -> Result<Option<CodingWorkspaceProject>, String> {
    let selected = if let Some(path) = path {
        PathBuf::from(path)
    } else {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        app.dialog().file().pick_folder(move |path| {
            let _ = sender.send(path);
        });
        let Some(path) = receiver
            .await
            .map_err(|_| "project folder dialog closed unexpectedly".to_string())?
        else {
            return Ok(None);
        };
        path.as_path()
            .ok_or_else(|| "project folder dialog returned an invalid path".to_string())?
            .to_path_buf()
    };

    let project = describe_project(&selected)?;
    let history_path = history_path(&app)?;
    let mut history = load_history(&history_path);
    remember_project(Path::new(&project.path), &mut history);
    save_history(&history_path, &history)?;
    Ok(Some(project))
}

#[tauri::command]
pub fn forget_coding_workspace_project(app: AppHandle, path: String) -> Result<(), String> {
    let canonical = canonical_project_path(Path::new(&path)).ok();
    let history_path = history_path(&app)?;
    let mut history = load_history(&history_path);
    history.recent.retain(|candidate| {
        if candidate == Path::new(&path) {
            return false;
        }
        match (&canonical, candidate.canonicalize()) {
            (Some(expected), Ok(actual)) => expected != &actual,
            _ => true,
        }
    });
    save_history(&history_path, &history)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_plain_and_git_projects_without_running_git() {
        let temp = tempfile::tempdir().unwrap();
        let plain = temp.path().join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        let plain_project = describe_project(&plain).unwrap();
        assert_eq!(plain_project.name, "plain");
        assert_eq!(plain_project.repository_root, None);

        let repo = temp.path().join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::write(repo.join(".git").join("HEAD"), "ref: refs/heads/main\n").unwrap();
        let nested = repo.join("crates").join("app");
        std::fs::create_dir_all(&nested).unwrap();
        let project = describe_project(&nested).unwrap();
        assert_eq!(project.name, "app");
        assert_eq!(project.git_branch.as_deref(), Some("main"));
        assert_eq!(
            project.repository_root,
            Some(repo.canonicalize().unwrap().to_string_lossy().to_string())
        );
    }

    #[test]
    fn recent_projects_are_mru_and_bounded() {
        let mut history = CodingWorkspaceHistory::default();
        for index in 0..=MAX_RECENT_PROJECTS {
            remember_project(Path::new(&format!("C:/projects/{index}")), &mut history);
        }
        assert_eq!(history.recent.len(), MAX_RECENT_PROJECTS);
        assert_eq!(history.recent[0], PathBuf::from("C:/projects/12"));
        remember_project(Path::new("C:/projects/6"), &mut history);
        assert_eq!(history.recent[0], PathBuf::from("C:/projects/6"));
    }

    #[test]
    fn rejects_relative_project_paths() {
        let error = canonical_project_path(Path::new("relative/project")).unwrap_err();
        assert!(error.contains("absolute"));
    }

    #[test]
    fn rejects_project_paths_with_prompt_control_characters() {
        let error = canonical_project_path(Path::new("C:/project\nignore-rules")).unwrap_err();
        assert!(error.contains("control characters"));
    }
}
