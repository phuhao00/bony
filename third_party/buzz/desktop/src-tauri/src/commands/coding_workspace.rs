//! Local Coding Workspace project selection and recent-project persistence.
//!
//! The renderer receives validated descriptors only. Absolute paths are picked,
//! canonicalized, inspected, and persisted in Rust so future coding runtimes
//! (Grok, Codex, Claude Code, or custom ACP harnesses) share one project model.

use atomic_write_file::AtomicWriteFile;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;
use uuid::Uuid;

use super::project_git_exec::{build_git_auth_config, run_git, GitAuthConfig};
use crate::app_state::AppState;

const HISTORY_FILE: &str = "coding-workspaces.json";
const MAX_RECENT_PROJECTS: usize = 12;
const MAX_WORKSPACE_FILES: usize = 500;
const MAX_WORKSPACE_CHANGES: usize = 500;
const MAX_WORKSPACE_COMMITS: usize = 30;
const MAX_DIFF_LINES: usize = 2_000;
const MAX_UNTRACKED_DIFF_BYTES: u64 = 512 * 1024;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodingWorkspaceProject {
    pub id: String,
    pub name: String,
    pub path: String,
    pub repository_root: Option<String>,
    pub git_branch: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodingWorkspaceFile {
    pub path: String,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodingWorkspaceChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    Untracked,
    Conflict,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodingWorkspaceChange {
    pub path: String,
    pub original_path: Option<String>,
    pub kind: CodingWorkspaceChangeKind,
    pub staged: bool,
    pub index_status: String,
    pub worktree_status: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodingWorkspaceCommit {
    pub hash: String,
    pub short_hash: String,
    pub author_name: String,
    pub timestamp: i64,
    pub subject: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodingWorkspaceSnapshot {
    pub files: Vec<CodingWorkspaceFile>,
    pub changes: Vec<CodingWorkspaceChange>,
    pub commits: Vec<CodingWorkspaceCommit>,
    pub git_branch: Option<String>,
    pub is_git_repository: bool,
    pub files_truncated: bool,
    pub changes_truncated: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodingWorkspaceFileDiff {
    pub path: String,
    pub additions: usize,
    pub deletions: usize,
    pub patch: String,
    pub truncated: bool,
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

fn normalized_relative_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn collect_plain_files(
    root: &Path,
    directory: &Path,
    files: &mut Vec<CodingWorkspaceFile>,
    truncated: &mut bool,
) -> Result<(), String> {
    if files.len() >= MAX_WORKSPACE_FILES {
        *truncated = true;
        return Ok(());
    }

    let mut entries = std::fs::read_dir(directory)
        .map_err(|error| format!("read project folder: {error}"))?
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        if files.len() >= MAX_WORKSPACE_FILES {
            *truncated = true;
            break;
        }
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("inspect project entry: {error}"))?;
        if file_type.is_dir() {
            let name = entry.file_name();
            if matches!(name.to_str(), Some(".git" | "target" | "node_modules")) {
                continue;
            }
            collect_plain_files(root, &path, files, truncated)?;
        } else if file_type.is_file() || file_type.is_symlink() {
            let relative = path.strip_prefix(root).unwrap_or(&path);
            files.push(CodingWorkspaceFile {
                path: normalized_relative_path(relative),
                size: entry.metadata().ok().map(|metadata| metadata.len()),
            });
        }
    }
    Ok(())
}

fn parse_git_files(root: &Path, output: &str) -> (Vec<CodingWorkspaceFile>, bool) {
    let mut truncated = false;
    let files = output
        .split('\0')
        .filter(|path| !path.is_empty())
        .enumerate()
        .filter_map(|(index, path)| {
            if index >= MAX_WORKSPACE_FILES {
                truncated = true;
                return None;
            }
            Some(CodingWorkspaceFile {
                path: path.replace('\\', "/"),
                size: root
                    .join(path)
                    .metadata()
                    .ok()
                    .map(|metadata| metadata.len()),
            })
        })
        .collect();
    (files, truncated)
}

fn change_kind(index: char, worktree: char) -> CodingWorkspaceChangeKind {
    let pair = [index, worktree];
    if matches!(
        pair,
        ['D', 'D'] | ['A', 'U'] | ['U', 'D'] | ['U', 'A'] | ['D', 'U'] | ['A', 'A'] | ['U', 'U']
    ) {
        CodingWorkspaceChangeKind::Conflict
    } else if pair == ['?', '?'] {
        CodingWorkspaceChangeKind::Untracked
    } else if pair.contains(&'R') {
        CodingWorkspaceChangeKind::Renamed
    } else if pair.contains(&'C') {
        CodingWorkspaceChangeKind::Copied
    } else if pair.contains(&'A') {
        CodingWorkspaceChangeKind::Added
    } else if pair.contains(&'D') {
        CodingWorkspaceChangeKind::Deleted
    } else {
        CodingWorkspaceChangeKind::Modified
    }
}

fn parse_git_changes(output: &str) -> (Vec<CodingWorkspaceChange>, bool) {
    let mut records = output.split('\0');
    let mut changes = Vec::new();
    let mut truncated = false;
    while let Some(record) = records.next() {
        if changes.len() >= MAX_WORKSPACE_CHANGES {
            truncated = true;
            break;
        }
        if record.len() < 3 {
            continue;
        }
        let mut chars = record.chars();
        let index = chars.next().unwrap_or(' ');
        let worktree = chars.next().unwrap_or(' ');
        let path = chars.as_str().trim_start().replace('\\', "/");
        if path.is_empty() || [index, worktree] == ['!', '!'] {
            continue;
        }
        let original_path = if matches!(index, 'R' | 'C') || matches!(worktree, 'R' | 'C') {
            records
                .next()
                .filter(|value| !value.is_empty())
                .map(|value| value.replace('\\', "/"))
        } else {
            None
        };
        changes.push(CodingWorkspaceChange {
            path,
            original_path,
            kind: change_kind(index, worktree),
            staged: !matches!(index, ' ' | '?' | '!'),
            index_status: index.to_string(),
            worktree_status: worktree.to_string(),
        });
    }
    (changes, truncated)
}

fn parse_git_commits(output: &str) -> Vec<CodingWorkspaceCommit> {
    output
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\0');
            Some(CodingWorkspaceCommit {
                hash: parts.next()?.to_string(),
                short_hash: parts.next()?.to_string(),
                author_name: parts.next()?.to_string(),
                timestamp: parts.next()?.parse().ok()?,
                subject: parts.next().unwrap_or_default().to_string(),
            })
        })
        .take(MAX_WORKSPACE_COMMITS)
        .collect()
}

fn clean_workspace_relative_path(path: &str) -> Result<&str, String> {
    if path.is_empty() || path.chars().any(char::is_control) {
        return Err("workspace file path is invalid".to_string());
    }
    let candidate = Path::new(path);
    if candidate.is_absolute()
        || candidate
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("workspace file path must stay inside the project".to_string());
    }
    Ok(path)
}

fn patch_stats(patch: &str) -> (usize, usize) {
    patch.lines().fold((0, 0), |(additions, deletions), line| {
        if line.starts_with('+') && !line.starts_with("+++") {
            (additions + 1, deletions)
        } else if line.starts_with('-') && !line.starts_with("---") {
            (additions, deletions + 1)
        } else {
            (additions, deletions)
        }
    })
}

fn truncate_patch(patch: String) -> (String, bool) {
    let mut lines = patch.lines();
    let retained = lines.by_ref().take(MAX_DIFF_LINES).collect::<Vec<_>>();
    let truncated = lines.next().is_some();
    let mut patch = retained.join("\n");
    if !patch.is_empty() {
        patch.push('\n');
    }
    (patch, truncated)
}

fn untracked_file_patch(root: &Path, relative_path: &str) -> Result<(String, bool), String> {
    let root = root
        .canonicalize()
        .map_err(|error| format!("resolve project folder: {error}"))?;
    let file_path = root.join(relative_path);
    let canonical = file_path
        .canonicalize()
        .map_err(|error| format!("open changed file: {error}"))?;
    if !canonical.starts_with(&root) || !canonical.is_file() {
        return Err("changed file is outside the project".to_string());
    }

    let mut bytes = Vec::new();
    std::fs::File::open(&canonical)
        .map_err(|error| format!("open changed file: {error}"))?
        .take(MAX_UNTRACKED_DIFF_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read changed file: {error}"))?;
    let bytes_truncated = bytes.len() as u64 > MAX_UNTRACKED_DIFF_BYTES;
    bytes.truncate(MAX_UNTRACKED_DIFF_BYTES as usize);
    if bytes.contains(&0) {
        return Ok((
            format!("Binary file b/{relative_path} has been added.\n"),
            bytes_truncated,
        ));
    }
    let content = String::from_utf8_lossy(&bytes);
    let lines = content.lines().collect::<Vec<_>>();
    let mut patch = format!(
        "diff --git a/{0} b/{0}\n--- /dev/null\n+++ b/{0}\n@@ -0,0 +1,{1} @@\n",
        relative_path,
        lines.len()
    );
    for line in lines.iter().take(MAX_DIFF_LINES) {
        patch.push('+');
        patch.push_str(line);
        patch.push('\n');
    }
    Ok((patch, bytes_truncated || lines.len() > MAX_DIFF_LINES))
}

fn build_workspace_file_diff(
    root: PathBuf,
    auth: GitAuthConfig,
    relative_path: String,
) -> Result<CodingWorkspaceFileDiff, String> {
    clean_workspace_relative_path(&relative_path)?;
    let tracked = run_git(
        &["ls-files", "--error-unmatch", "--", &relative_path],
        Some(&root),
        &auth,
    )
    .is_ok();

    let (full_patch, already_truncated) = if tracked {
        let has_head = run_git(&["rev-parse", "--verify", "HEAD"], Some(&root), &auth).is_ok();
        let patch = if has_head {
            run_git(
                &[
                    "diff",
                    "--no-ext-diff",
                    "--unified=3",
                    "HEAD",
                    "--",
                    &relative_path,
                ],
                Some(&root),
                &auth,
            )?
        } else {
            let staged = run_git(
                &[
                    "diff",
                    "--cached",
                    "--no-ext-diff",
                    "--unified=3",
                    "--",
                    &relative_path,
                ],
                Some(&root),
                &auth,
            )?;
            let unstaged = run_git(
                &["diff", "--no-ext-diff", "--unified=3", "--", &relative_path],
                Some(&root),
                &auth,
            )?;
            format!("{staged}{unstaged}")
        };
        (patch, false)
    } else {
        untracked_file_patch(&root, &relative_path)?
    };
    let (additions, deletions) = patch_stats(&full_patch);
    let (patch, patch_truncated) = truncate_patch(full_patch);
    Ok(CodingWorkspaceFileDiff {
        path: relative_path,
        additions,
        deletions,
        patch,
        truncated: already_truncated || patch_truncated,
    })
}

fn build_workspace_snapshot(
    root: PathBuf,
    repository: Option<PathBuf>,
    auth: Option<GitAuthConfig>,
) -> Result<CodingWorkspaceSnapshot, String> {
    let Some(repository) = repository else {
        let mut files = Vec::new();
        let mut files_truncated = false;
        collect_plain_files(&root, &root, &mut files, &mut files_truncated)?;
        return Ok(CodingWorkspaceSnapshot {
            files,
            changes: Vec::new(),
            commits: Vec::new(),
            git_branch: None,
            is_git_repository: false,
            files_truncated,
            changes_truncated: false,
        });
    };
    let auth = auth.ok_or_else(|| "git configuration is unavailable".to_string())?;
    let file_output = run_git(
        &[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
        ],
        Some(&root),
        &auth,
    )?;
    let (files, files_truncated) = parse_git_files(&root, &file_output);
    let (changes, changes_truncated) = parse_git_changes(&run_git(
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--",
        ],
        Some(&root),
        &auth,
    )?);
    let commits = parse_git_commits(
        &run_git(
            &[
                "log",
                "--max-count=30",
                "--format=%H%x00%h%x00%an%x00%at%x00%s",
            ],
            Some(&root),
            &auth,
        )
        .unwrap_or_default(),
    );
    Ok(CodingWorkspaceSnapshot {
        files,
        changes,
        commits,
        git_branch: git_branch(&repository),
        is_git_repository: true,
        files_truncated,
        changes_truncated,
    })
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

#[tauri::command]
pub async fn get_coding_workspace_snapshot(
    path: String,
    state: State<'_, AppState>,
) -> Result<CodingWorkspaceSnapshot, String> {
    let root = canonical_project_path(Path::new(&path))?;
    let repository = repository_root(&root);
    let auth = repository
        .as_ref()
        .map(|_| build_git_auth_config(&state))
        .transpose()?;
    tauri::async_runtime::spawn_blocking(move || build_workspace_snapshot(root, repository, auth))
        .await
        .map_err(|error| format!("Coding Workspace snapshot task failed: {error}"))?
}

#[tauri::command]
pub async fn get_coding_workspace_file_diff(
    path: String,
    file_path: String,
    state: State<'_, AppState>,
) -> Result<CodingWorkspaceFileDiff, String> {
    let root = canonical_project_path(Path::new(&path))?;
    if repository_root(&root).is_none() {
        return Err("project folder is not a Git repository".to_string());
    }
    let relative_path = clean_workspace_relative_path(&file_path)?.to_string();
    let auth = build_git_auth_config(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        build_workspace_file_diff(root, auth, relative_path)
    })
    .await
    .map_err(|error| format!("Coding Workspace diff task failed: {error}"))?
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

    #[test]
    fn parses_git_changes_including_renames_and_conflicts() {
        let (changes, truncated) = parse_git_changes(
            " M src/lib.rs\0R  src/new.rs\0src/old.rs\0?? notes.txt\0UU conflict.rs\0",
        );
        assert!(!truncated);
        assert_eq!(changes.len(), 4);
        assert_eq!(changes[0].kind, CodingWorkspaceChangeKind::Modified);
        assert!(!changes[0].staged);
        assert_eq!(changes[1].kind, CodingWorkspaceChangeKind::Renamed);
        assert_eq!(changes[1].original_path.as_deref(), Some("src/old.rs"));
        assert!(changes[1].staged);
        assert_eq!(changes[2].kind, CodingWorkspaceChangeKind::Untracked);
        assert_eq!(changes[3].kind, CodingWorkspaceChangeKind::Conflict);
    }

    #[test]
    fn plain_folder_snapshot_skips_generated_directories() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("src")).unwrap();
        std::fs::create_dir_all(temp.path().join("target")).unwrap();
        std::fs::write(
            temp.path().join("src").join("lib.rs"),
            "pub fn value() {}\n",
        )
        .unwrap();
        std::fs::write(temp.path().join("target").join("artifact"), "generated").unwrap();
        let snapshot = build_workspace_snapshot(temp.path().to_path_buf(), None, None).unwrap();
        assert_eq!(snapshot.files.len(), 1);
        assert_eq!(snapshot.files[0].path, "src/lib.rs");
        assert!(!snapshot.is_git_repository);
    }

    #[test]
    fn rejects_diff_paths_outside_the_workspace() {
        assert!(clean_workspace_relative_path("../secret.txt").is_err());
        assert!(clean_workspace_relative_path("C:/secret.txt").is_err());
        assert_eq!(
            clean_workspace_relative_path("src/lib.rs").unwrap(),
            "src/lib.rs"
        );
    }

    #[test]
    fn untracked_patch_has_reviewable_line_stats() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("src")).unwrap();
        std::fs::write(temp.path().join("src").join("new.rs"), "one\ntwo\n").unwrap();
        let (patch, truncated) = untracked_file_patch(temp.path(), "src/new.rs").unwrap();
        assert!(!truncated);
        assert_eq!(patch_stats(&patch), (2, 0));
        assert!(patch.contains("@@ -0,0 +1,2 @@"));
    }

    #[test]
    fn tracked_file_diff_reviews_worktree_against_head() {
        let temp = tempfile::tempdir().unwrap();
        let auth = crate::commands::project_git_exec::build_test_git_auth_config().unwrap();
        run_git(&["init"], Some(temp.path()), &auth).unwrap();
        run_git(
            &["config", "user.name", "Coding Workspace Test"],
            Some(temp.path()),
            &auth,
        )
        .unwrap();
        run_git(
            &["config", "user.email", "coding-workspace@example.test"],
            Some(temp.path()),
            &auth,
        )
        .unwrap();
        std::fs::write(temp.path().join("review.txt"), "old\ncontext\n").unwrap();
        run_git(&["add", "review.txt"], Some(temp.path()), &auth).unwrap();
        run_git(&["commit", "-m", "initial"], Some(temp.path()), &auth).unwrap();
        std::fs::write(temp.path().join("review.txt"), "new\ncontext\n").unwrap();

        let diff =
            build_workspace_file_diff(temp.path().to_path_buf(), auth, "review.txt".to_string())
                .unwrap();
        assert_eq!((diff.additions, diff.deletions), (1, 1));
        assert!(diff.patch.contains("-old"));
        assert!(diff.patch.contains("+new"));
        assert!(!diff.truncated);
    }
}
