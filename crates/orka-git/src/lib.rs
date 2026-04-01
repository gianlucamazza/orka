//! `orka-git` — First-class git skills for Orka agents.
//!
//! Provides structured, policy-enforced git operations as [`Skill`]
//! implementations. Read operations use **gix** (pure-Rust, no process
//! overhead); write and remote operations use the **git CLI** (required for
//! push, worktrees, commit signing).
//!
//! # Quick start
//!
//! ```no_run
//! use orka_git::{GitConfig, create_git_skills};
//!
//! let config = GitConfig::default();
//! let skills = create_git_skills(&config, None).unwrap();
//! // register `skills` into your SkillRegistry
//! ```

pub mod cli;
pub mod config;
pub mod error;
pub mod guard;
pub mod repo;
pub mod skills;
pub mod worktree;

use std::{path::PathBuf, sync::Arc};

pub use config::{GitAuthorshipConfig, GitAuthorshipMode, GitConfig, GitWorktreeConfig};
use orka_core::traits::Skill;

use crate::{
    guard::GitGuard,
    skills::{
        branch::{GitBranchCreateSkill, GitBranchListSkill, GitCheckoutSkill},
        commit::GitCommitSkill,
        diff::GitDiffSkill,
        log::GitLogSkill,
        remote::{GitFetchSkill, GitMergeSkill, GitPullSkill, GitPushSkill},
        search::{GitBlameSkill, GitGrepSkill},
        stash::GitStashSkill,
        status::GitStatusSkill,
        worktree::{GitWorktreeCreateSkill, GitWorktreeListSkill, GitWorktreeRemoveSkill},
    },
    worktree::WorktreeManager,
};

/// Build all git skills from the provided [`GitConfig`].
///
/// # Parameters
/// - `config` — the `[git]` section from `orka.toml`
/// - `repo_root` — path to the main repository root; used by `WorktreeManager`.
///   If `None`, the current working directory is used.
///
/// # Returns
/// A `Vec<Arc<dyn Skill>>` ready to be registered in the `SkillRegistry`.
///
/// # Errors
/// Returns [`orka_core::Error`] if the `GitGuard` cannot be initialised (e.g.
/// invalid glob patterns).
pub fn create_git_skills(
    config: &GitConfig,
    repo_root: Option<PathBuf>,
) -> orka_core::Result<Vec<Arc<dyn Skill>>> {
    let guard = Arc::new(GitGuard::from_config(config)?);

    let repo_root =
        repo_root.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let manager = Arc::new(WorktreeManager::new(
        repo_root,
        &config.worktree.base_dir,
        config.worktree.copy_files.clone(),
        config.worktree.symlink_dirs.clone(),
        config.worktree.max_concurrent,
        config.command_timeout_secs,
    ));

    let skills: Vec<Arc<dyn Skill>> = vec![
        // Tier 1: read-only
        Arc::new(GitStatusSkill::new(guard.clone())),
        Arc::new(GitDiffSkill::new(guard.clone())),
        Arc::new(GitLogSkill::new(guard.clone())),
        Arc::new(GitBranchListSkill::new(guard.clone())),
        Arc::new(GitBlameSkill::new(guard.clone())),
        Arc::new(GitGrepSkill::new(guard.clone())),
        // Tier 2: write
        Arc::new(GitCommitSkill::new(guard.clone())),
        Arc::new(GitBranchCreateSkill::new(guard.clone())),
        Arc::new(GitCheckoutSkill::new(guard.clone())),
        Arc::new(GitStashSkill::new(guard.clone())),
        // Tier 3: remote
        Arc::new(GitFetchSkill::new(guard.clone())),
        Arc::new(GitPullSkill::new(guard.clone())),
        Arc::new(GitPushSkill::new(guard.clone())),
        Arc::new(GitMergeSkill::new(guard.clone())),
        // Tier 4: worktree
        Arc::new(GitWorktreeCreateSkill::new(guard.clone(), manager.clone())),
        Arc::new(GitWorktreeListSkill::new(guard.clone(), manager.clone())),
        Arc::new(GitWorktreeRemoveSkill::new(guard, manager)),
    ];

    Ok(skills)
}
