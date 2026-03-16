use orka_workspace::config::*;
use orka_workspace::loader::WorkspaceLoader;
use orka_workspace::parse::parse_document;
use std::sync::Arc;
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
fn parse_tools_frontmatter() {
    let raw = "---\ntools:\n  - name: search\n    enabled: true\n  - name: calc\n    enabled: false\n---\nTool docs";
    let doc = parse_document::<ToolsFrontmatter>(raw).unwrap();
    assert_eq!(doc.frontmatter.tools.len(), 2);
    assert_eq!(doc.frontmatter.tools[0].name, "search");
    assert!(doc.frontmatter.tools[0].enabled);
    assert!(!doc.frontmatter.tools[1].enabled);
}

#[test]
fn parse_heartbeat_default_interval() {
    let raw = "---\nversion: \"2.0\"\n---\nheartbeat info";
    let doc = parse_document::<HeartbeatFrontmatter>(raw).unwrap();
    assert_eq!(doc.frontmatter.interval_secs, 30);
    assert_eq!(doc.frontmatter.version.as_deref(), Some("2.0"));
}

#[test]
fn parse_identity_frontmatter() {
    let raw = "---\nagent_id: agent-42\ndisplay_name: Orka Bot\n---\n";
    let doc = parse_document::<IdentityFrontmatter>(raw).unwrap();
    assert_eq!(doc.frontmatter.agent_id.as_deref(), Some("agent-42"));
    assert_eq!(doc.frontmatter.display_name.as_deref(), Some("Orka Bot"));
}

#[test]
fn parse_memory_frontmatter() {
    let raw = "---\nbackend: redis\nmax_entries: 1000\n---\nmemory config";
    let doc = parse_document::<MemoryFrontmatter>(raw).unwrap();
    assert_eq!(doc.frontmatter.backend.as_deref(), Some("redis"));
    assert_eq!(doc.frontmatter.max_entries, Some(1000));
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
        "---\ntools:\n  - name: t1\n    enabled: true\n---\n",
    )
    .await
    .unwrap();
    fs::write(dir.path().join("IDENTITY.md"), "---\nagent_id: a1\n---\n")
        .await
        .unwrap();
    fs::write(
        dir.path().join("HEARTBEAT.md"),
        "---\ninterval_secs: 10\n---\n",
    )
    .await
    .unwrap();
    fs::write(dir.path().join("MEMORY.md"), "---\nbackend: sqlite\n---\n")
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
    assert!(state.tools.is_some());
    assert_eq!(state.tools.as_ref().unwrap().frontmatter.tools.len(), 1);
    assert!(state.identity.is_some());
    assert!(state.heartbeat.is_some());
    assert_eq!(
        state.heartbeat.as_ref().unwrap().frontmatter.interval_secs,
        10
    );
    assert!(state.memory.is_some());
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
    assert!(state.tools.is_none());
    assert!(state.identity.is_none());
    assert!(state.heartbeat.is_none());
    assert!(state.memory.is_none());
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
    use orka_workspace::loader::WorkspaceEvent;
    use orka_workspace::watcher::WorkspaceWatcher;

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
            if let Ok(WorkspaceEvent::FileChanged(f)) = rx.recv().await {
                if f == "SOUL.md" {
                    return true;
                }
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

    watcher.stop().await;
}
