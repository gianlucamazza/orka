#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use orka_workspace::{
    config::*,
    loader::WorkspaceLoader,
    parse::{parse_document, strip_frontmatter},
};
use tempfile::TempDir;
use tokio::fs;

// --- Frontmatter parsing tests ---

#[test]
fn parse_valid_frontmatter() {
    let raw = "---\nname: TestAgent\nversion: \"1.0\"\n---\nHello body";
    let doc = parse_document::<SoulFrontmatter>(raw).unwrap();
    assert_eq!(doc.frontmatter.name.as_deref(), Some("TestAgent"));
    assert_eq!(doc.frontmatter.version.as_deref(), Some("1.0"));
    assert_eq!(doc.body, "Hello body");
}

#[test]
fn parse_missing_opening_delimiter() {
    let raw = "name: TestAgent\n---\nHello body";
    let result = parse_document::<SoulFrontmatter>(raw);
    assert!(result.is_err());
}

#[test]
fn parse_missing_closing_delimiter() {
    let raw = "---\nname: TestAgent\nHello body";
    let result = parse_document::<SoulFrontmatter>(raw);
    assert!(result.is_err());
}

#[test]
fn parse_empty_body() {
    let raw = "---\nname: TestAgent\n---\n";
    let doc = parse_document::<SoulFrontmatter>(raw).unwrap();
    assert_eq!(doc.frontmatter.name.as_deref(), Some("TestAgent"));
    assert_eq!(doc.body, "");
}

#[test]
fn parse_empty_frontmatter() {
    let raw = "---\n\n---\nSome body text";
    let doc = parse_document::<SoulFrontmatter>(raw).unwrap();
    assert!(doc.frontmatter.name.is_none());
    assert_eq!(doc.body, "Some body text");
}

#[test]
fn strip_frontmatter_removes_yaml() {
    let raw = "---\ntools:\n  - name: echo\n---\n\n## Tools body";
    let body = strip_frontmatter(raw);
    assert_eq!(body, "\n## Tools body");
}

#[test]
fn strip_frontmatter_no_frontmatter() {
    let raw = "## Just markdown\n\nSome content";
    let body = strip_frontmatter(raw);
    assert_eq!(body, raw);
}

#[test]
fn strip_frontmatter_unclosed_returns_raw() {
    let raw = "---\nname: TestAgent\nNo closing delimiter";
    let body = strip_frontmatter(raw);
    assert_eq!(body, raw);
}

// --- Loader tests ---

#[tokio::test]
async fn loader_load_all() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("SOUL.md"),
        "---\nname: TestSoul\n---\nSoul body",
    )
    .await
    .unwrap();
    fs::write(
        dir.path().join("TOOLS.md"),
        "---\ntools:\n  - name: t1\n    enabled: true\n---\nTool instructions",
    )
    .await
    .unwrap();

    let loader = WorkspaceLoader::new(dir.path());
    loader.load_all().await.unwrap();

    let binding = loader.state();
    let state = binding.read().await;
    assert!(state.soul.is_some());
    assert_eq!(
        state.soul.as_ref().unwrap().frontmatter.name.as_deref(),
        Some("TestSoul")
    );
    assert!(state.tools_body.is_some());
    assert_eq!(state.tools_body.as_deref().unwrap(), "Tool instructions");
}

#[tokio::test]
async fn loader_tools_md_without_frontmatter() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("TOOLS.md"),
        "## Tool instructions\n\nUse wisely.",
    )
    .await
    .unwrap();

    let loader = WorkspaceLoader::new(dir.path());
    loader.load_all().await.unwrap();

    let binding = loader.state();
    let state = binding.read().await;
    assert_eq!(
        state.tools_body.as_deref().unwrap(),
        "## Tool instructions\n\nUse wisely."
    );
}

#[tokio::test]
async fn loader_missing_file() {
    let dir = TempDir::new().unwrap();
    // No files created — all should be None after load_all
    let loader = WorkspaceLoader::new(dir.path());
    loader.load_all().await.unwrap();

    let binding = loader.state();
    let state = binding.read().await;
    assert!(state.soul.is_none());
    assert!(state.tools_body.is_none());
}

#[tokio::test]
async fn loader_malformed_file() {
    let dir = TempDir::new().unwrap();
    // Missing closing delimiter
    fs::write(dir.path().join("SOUL.md"), "no frontmatter here")
        .await
        .unwrap();

    let loader = WorkspaceLoader::new(dir.path());
    loader.load_all().await.unwrap();

    let binding = loader.state();
    let state = binding.read().await;
    assert!(state.soul.is_none());
}

// --- Watcher test ---

#[tokio::test]
async fn watcher_detects_change() {
    use orka_workspace::{loader::WorkspaceEvent, watcher::WorkspaceWatcher};

    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("SOUL.md"), "---\nname: Original\n---\nBody")
        .await
        .unwrap();

    let loader = Arc::new(WorkspaceLoader::new(dir.path()));
    loader.load_all().await.unwrap();

    let mut rx = loader.subscribe();
    let watcher = WorkspaceWatcher::start(loader.clone()).unwrap();

    // Modify the file
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    fs::write(
        dir.path().join("SOUL.md"),
        "---\nname: Updated\n---\nNew body",
    )
    .await
    .unwrap();

    // Wait for the event (up to 2s)
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if let Ok(WorkspaceEvent::FileChanged(f)) = rx.recv().await
                && f == "SOUL.md"
            {
                return true;
            }
        }
    })
    .await;

    assert!(
        result.is_ok(),
        "watcher should detect file change within 2s"
    );

    let binding = loader.state();
    let state = binding.read().await;
    assert_eq!(
        state.soul.as_ref().unwrap().frontmatter.name.as_deref(),
        Some("Updated")
    );

    watcher.stop();
}
