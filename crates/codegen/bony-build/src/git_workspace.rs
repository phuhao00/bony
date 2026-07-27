//! Read-mostly Git integration and isolated task worktrees.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
}

#[derive(Debug, Clone)]
pub struct FileChange {
    pub path: PathBuf,
    pub old_path: Option<PathBuf>,
    pub kind: ChangeKind,
    pub staged: bool,
}

#[derive(Debug, Clone)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: String,
}

#[derive(Debug, Clone)]
pub struct CommitFileChange {
    pub path: PathBuf,
    pub old_path: Option<PathBuf>,
    pub status: char,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone)]
pub struct CommitDetail {
    pub hash: String,
    pub author: String,
    pub date: String,
    pub subject: String,
    pub body: String,
    pub files: Vec<CommitFileChange>,
    pub summary: String,
}

pub struct GitWorkspaceService;

impl GitWorkspaceService {
    pub fn primary_repo_root(path: &Path) -> Result<Option<PathBuf>, String> {
        main_repo_root(path)
    }

    pub fn repo_root(path: &Path) -> Result<Option<PathBuf>, String> {
        let out = git(path, ["rev-parse", "--show-toplevel"])?;
        Ok(out
            .status
            .success()
            .then(|| PathBuf::from(out.stdout.trim())))
    }

    pub fn changes(path: &Path) -> Result<Vec<FileChange>, String> {
        let out = git(path, ["status", "--porcelain=v1", "-z"])?;
        if !out.status.success() {
            return Err(out.stderr);
        }
        Ok(merge_working_copy(parse_porcelain_z(out.stdout.as_bytes())?))
    }

    pub fn diff(path: &Path, file: Option<&Path>, staged: bool) -> Result<String, String> {
        let mut cmd = crate::process::command("git");
        cmd.current_dir(path)
            .arg("diff")
            .arg("--no-ext-diff")
            .arg("--no-color");
        if staged {
            cmd.arg("--cached");
        }
        if let Some(file) = file {
            cmd.arg("--").arg(file);
        }
        let out = cmd.output().map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).into_owned())
        }
    }

    /// Prefer worktree (+ untracked) diff; fall back to staged when worktree is empty.
    pub fn working_diff(path: &Path, file: &Path) -> Result<String, String> {
        let wt = Self::diff(path, Some(file), false)?;
        if !wt.trim().is_empty() {
            return Ok(wt);
        }
        Self::diff(path, Some(file), true)
    }

    pub fn current_branch(path: &Path) -> Result<String, String> {
        let out = git(path, ["branch", "--show-current"])?;
        if !out.status.success() {
            return Err(out.stderr);
        }
        let name = out.stdout.trim().to_string();
        if name.is_empty() {
            Ok("(detached)".into())
        } else {
            Ok(name)
        }
    }

    pub fn commit_all(path: &Path, message: &str) -> Result<(), String> {
        let msg = message.trim();
        if msg.is_empty() {
            return Err("提交说明不能为空".into());
        }
        git_write(path, ["add", "-A"], None)?;
        let out = crate::process::command("git")
            .current_dir(path)
            .args(["commit", "-m", msg])
            .output()
            .map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).into_owned())
        }
    }

    pub fn restore_file(path: &Path, file: &Path) -> Result<(), String> {
        // Tracked: drop index + worktree edits.
        let restored = git_write(path, ["restore", "--worktree", "--staged"], Some(file));
        if restored.is_ok() {
            return Ok(());
        }
        // Untracked: remove the path.
        let out = crate::process::command("git")
            .current_dir(path)
            .args(["clean", "-f", "--"])
            .arg(file)
            .output()
            .map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).into_owned())
        }
    }

    pub fn log_oneline(path: &Path, limit: usize) -> Result<Vec<(String, String)>, String> {
        let n = format!("-n{}", limit.max(1));
        let out = crate::process::command("git")
            .current_dir(path)
            .args(["log", &n, "--pretty=format:%h\t%s"])
            .output()
            .map_err(|e| e.to_string())?;
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        if !out.status.success() {
            if stderr.contains("does not have any commits")
                || stderr.contains("unknown revision")
                || stderr.contains("bad default revision")
            {
                return Ok(Vec::new());
            }
            return Err(stderr);
        }
        let stdout = String::from_utf8_lossy(&out.stdout);
        Ok(stdout
            .lines()
            .filter_map(|line| {
                let (hash, subject) = line.split_once('\t')?;
                Some((hash.to_string(), subject.to_string()))
            })
            .collect())
    }

    /// Commit metadata + changed file list (Fork-style drill-down; no full patch dump).
    pub fn show_commit(path: &Path, rev: &str) -> Result<CommitDetail, String> {
        let meta = crate::process::command("git")
            .current_dir(path)
            .args([
                "log",
                "-1",
                "--pretty=format:%h%n%an <%ae>%n%ci%n%s%n%n%b",
                rev,
            ])
            .output()
            .map_err(|e| e.to_string())?;
        if !meta.status.success() {
            return Err(String::from_utf8_lossy(&meta.stderr).into_owned());
        }
        let meta_text = String::from_utf8_lossy(&meta.stdout);
        let mut lines = meta_text.lines();
        let hash = lines.next().unwrap_or(rev).to_string();
        let author = lines.next().unwrap_or("").to_string();
        let date = lines.next().unwrap_or("").to_string();
        let subject = lines.next().unwrap_or("").to_string();
        let _ = lines.next();
        let body: String = lines.collect::<Vec<_>>().join("\n").trim().to_string();

        let status_out = crate::process::command("git")
            .current_dir(path)
            .args([
                "diff-tree",
                "--no-commit-id",
                "-r",
                "-M",
                "--name-status",
                rev,
            ])
            .output()
            .map_err(|e| e.to_string())?;
        if !status_out.status.success() {
            return Err(String::from_utf8_lossy(&status_out.stderr).into_owned());
        }

        let num_out = crate::process::command("git")
            .current_dir(path)
            .args(["diff-tree", "--no-commit-id", "-r", "-M", "--numstat", rev])
            .output()
            .map_err(|e| e.to_string())?;
        if !num_out.status.success() {
            return Err(String::from_utf8_lossy(&num_out.stderr).into_owned());
        }

        let mut stats: std::collections::HashMap<String, (u32, u32)> =
            std::collections::HashMap::new();
        for line in String::from_utf8_lossy(&num_out.stdout).lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() < 3 {
                continue;
            }
            let adds = parts[0].parse::<u32>().unwrap_or(0);
            let dels = parts[1].parse::<u32>().unwrap_or(0);
            // rename: adds\tdels\told\tnew → key is new path
            let key = if parts.len() >= 4 {
                parts[3].to_string()
            } else {
                parts[2].to_string()
            };
            stats.insert(key, (adds, dels));
        }

        let mut files = Vec::new();
        for line in String::from_utf8_lossy(&status_out.stdout).lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.is_empty() || parts[0].is_empty() {
                continue;
            }
            let status_code = parts[0].chars().next().unwrap_or('M');
            let (path, old_path) = if parts[0].starts_with('R') || parts[0].starts_with('C') {
                if parts.len() >= 3 {
                    (PathBuf::from(parts[2]), Some(PathBuf::from(parts[1])))
                } else {
                    continue;
                }
            } else if parts.len() >= 2 {
                (PathBuf::from(parts[1]), None)
            } else {
                continue;
            };
            let key = path.to_string_lossy().to_string();
            let (additions, deletions) = stats.get(&key).copied().unwrap_or((0, 0));
            files.push(CommitFileChange {
                path,
                old_path,
                status: status_code,
                additions,
                deletions,
            });
        }

        let total_files = files.len();
        let total_add: u32 = files.iter().map(|f| f.additions).sum();
        let total_del: u32 = files.iter().map(|f| f.deletions).sum();
        let summary = format!(
            "{total_files} file{} changed, {total_add} insertions(+), {total_del} deletions(-)",
            if total_files == 1 { "" } else { "s" }
        );

        Ok(CommitDetail {
            hash,
            author,
            date,
            subject,
            body,
            files,
            summary,
        })
    }

    /// Diff for a single path inside a commit (`git show REV -- path`).
    pub fn commit_file_diff(path: &Path, rev: &str, file: &Path) -> Result<String, String> {
        let out = crate::process::command("git")
            .current_dir(path)
            .args(["show", "--format=", "--no-color", "-p", rev, "--"])
            .arg(file)
            .output()
            .map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).into_owned())
        }
    }

    pub fn create_worktree(project: &Path, task_id: &str, title: &str) -> Result<Worktree, String> {
        let root = main_repo_root(project)?.ok_or_else(|| "目录不是 Git 仓库".to_string())?;
        let slug = slug(title);
        let short = &task_id[..task_id.len().min(8)];
        let branch = format!("codex/{slug}-{short}");
        let parent = worktree_parent(&root)?;
        let path = parent.join(short);
        let branch_ref = format!("refs/heads/{branch}");
        let exists = crate::process::command("git")
            .current_dir(&root)
            .args(["show-ref", "--verify", "--quiet", &branch_ref])
            .status()
            .map_err(|e| e.to_string())?
            .success();
        if exists {
            return Err(format!("任务分支已存在：{branch}"));
        }
        let out = crate::process::command("git")
            .current_dir(&root)
            .args(["-c", "core.longpaths=true", "worktree", "add", "-b"])
            .arg(&branch)
            .arg(&path)
            .arg("HEAD")
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            rollback_failed_worktree(&root, &parent, &path, &branch);
            return Err(String::from_utf8_lossy(&out.stderr).into_owned());
        }
        Ok(Worktree { path, branch })
    }

    pub fn stage(path: &Path, file: &Path) -> Result<(), String> {
        git_write(path, ["add"], Some(file))
    }
    pub fn unstage(path: &Path, file: &Path) -> Result<(), String> {
        git_write(path, ["restore", "--staged"], Some(file))
    }
}

/// Collapse porcelain rows so each path appears once (jj-style working copy).
fn merge_working_copy(rows: Vec<FileChange>) -> Vec<FileChange> {
    use std::collections::BTreeMap;
    let mut map: BTreeMap<String, FileChange> = BTreeMap::new();
    for row in rows {
        let key = row.path.to_string_lossy().replace('\\', "/");
        map.entry(key)
            .and_modify(|existing| {
                // Prefer conflicted / richer kind; keep path from either.
                if existing.kind != ChangeKind::Conflicted
                    && (row.kind == ChangeKind::Conflicted
                        || (existing.kind == ChangeKind::Untracked
                            && row.kind != ChangeKind::Untracked))
                {
                    existing.kind = row.kind;
                    if row.old_path.is_some() {
                        existing.old_path = row.old_path.clone();
                    }
                }
                existing.staged = existing.staged || row.staged;
            })
            .or_insert(row);
    }
    map.into_values().collect()
}

/// Resolve the primary checkout even when `project` is itself a linked worktree.
fn main_repo_root(project: &Path) -> Result<Option<PathBuf>, String> {
    let out = git(
        project,
        ["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    if !out.status.success() {
        return Ok(None);
    }
    Ok(PathBuf::from(out.stdout.trim())
        .parent()
        .map(Path::to_path_buf))
}

fn worktree_parent(root: &Path) -> Result<PathBuf, String> {
    let fallback = root.parent().unwrap_or(root).join(".bwt");
    #[cfg(target_os = "windows")]
    let candidates = {
        let drive = root
            .components()
            .next()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .unwrap_or_else(|| "C:".into());
        vec![PathBuf::from(format!(r"{drive}\tmp\bwt")), fallback]
    };
    #[cfg(not(target_os = "windows"))]
    let candidates = vec![fallback];

    for candidate in candidates {
        if std::fs::create_dir_all(&candidate).is_ok() {
            return Ok(candidate);
        }
    }
    Err("无法创建 worktree 根目录".into())
}

fn rollback_failed_worktree(root: &Path, managed_parent: &Path, path: &Path, branch: &str) {
    let _ = crate::process::command("git")
        .current_dir(root)
        .args(["worktree", "remove", "--force"])
        .arg(path)
        .status();
    let _ = crate::process::command("git")
        .current_dir(root)
        .args(["worktree", "prune"])
        .status();
    if path.parent() == Some(managed_parent) && path.is_dir() {
        let _ = std::fs::remove_dir_all(path);
    }
    let _ = crate::process::command("git")
        .current_dir(root)
        .args(["branch", "-D", branch])
        .status();
}

struct GitOutput {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}
fn git<const N: usize>(cwd: &Path, args: [&str; N]) -> Result<GitOutput, String> {
    let out = crate::process::command("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    Ok(GitOutput {
        status: out.status,
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}
fn git_write<const N: usize>(
    cwd: &Path,
    args: [&str; N],
    file: Option<&Path>,
) -> Result<(), String> {
    let mut cmd = crate::process::command("git");
    cmd.current_dir(cwd).args(args);
    if let Some(file) = file {
        cmd.arg("--").arg(file);
    }
    let out = cmd.output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

fn parse_porcelain_z(bytes: &[u8]) -> Result<Vec<FileChange>, String> {
    let fields: Vec<&[u8]> = bytes.split(|b| *b == 0).filter(|s| !s.is_empty()).collect();
    let mut result = Vec::new();
    let mut i = 0;
    while i < fields.len() {
        let field = String::from_utf8_lossy(fields[i]);
        if field.len() < 4 {
            return Err("无效的 git status 输出".into());
        }
        let code = &field[..2];
        let path = PathBuf::from(&field[3..]);
        let kind = if code == "??" {
            ChangeKind::Untracked
        } else if code.contains('U') || code == "AA" || code == "DD" {
            ChangeKind::Conflicted
        } else if code.contains('R') {
            ChangeKind::Renamed
        } else if code.contains('A') {
            ChangeKind::Added
        } else if code.contains('D') {
            ChangeKind::Deleted
        } else {
            ChangeKind::Modified
        };
        let old_path = if kind == ChangeKind::Renamed && i + 1 < fields.len() {
            i += 1;
            Some(PathBuf::from(String::from_utf8_lossy(fields[i]).as_ref()))
        } else {
            None
        };
        let staged = code
            .as_bytes()
            .first()
            .is_some_and(|c| *c != b' ' && *c != b'?');
        result.push(FileChange {
            path,
            old_path,
            kind,
            staged,
        });
        i += 1;
    }
    Ok(result)
}

fn slug(value: &str) -> String {
    let s: String = value
        .chars()
        .flat_map(char::to_lowercase)
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let s = s
        .split('-')
        .filter(|p| !p.is_empty())
        .take(5)
        .collect::<Vec<_>>()
        .join("-");
    if s.is_empty() { "task".into() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_changes() {
        let rows = parse_porcelain_z(b" M src/a.rs\0?? new.txt\0R  next.rs\0old.rs\0").unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1].kind, ChangeKind::Untracked);
        assert_eq!(rows[2].old_path.as_deref(), Some(Path::new("old.rs")));
    }
    #[test]
    fn safe_slug() {
        assert_eq!(slug("Fix: login flow!"), "fix-login-flow");
        assert_eq!(slug("中文"), "task");
    }

    #[test]
    fn creates_isolated_worktree_and_reads_changes() {
        let dir = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| {
            crate::process::command("git")
                .current_dir(dir.path())
                .args(args)
                .output()
                .unwrap()
        };
        if !run(&["init"]).status.success() {
            return;
        }
        run(&["config", "user.email", "test@example.com"]);
        run(&["config", "user.name", "Bony Test"]);
        std::fs::write(dir.path().join("README.md"), "hello").unwrap();
        assert!(run(&["add", "README.md"]).status.success());
        assert!(run(&["commit", "-m", "init"]).status.success());

        let worktree = GitWorkspaceService::create_worktree(
            dir.path(),
            &uuid::Uuid::new_v4().to_string(),
            "Test task",
        )
        .unwrap();
        assert!(worktree.path.is_dir());
        assert!(worktree.branch.starts_with("codex/test-task-"));
        assert_eq!(
            main_repo_root(&worktree.path).unwrap(),
            Some(dir.path().to_path_buf())
        );
        std::fs::write(worktree.path.join("README.md"), "changed").unwrap();
        let changes = GitWorkspaceService::changes(&worktree.path).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].kind, ChangeKind::Modified);
        let _ = crate::process::command("git")
            .current_dir(dir.path())
            .args(["worktree", "remove", "--force"])
            .arg(&worktree.path)
            .status();
    }
}
