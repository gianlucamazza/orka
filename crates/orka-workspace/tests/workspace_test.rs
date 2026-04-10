#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use orka_workspace::{
    Document, SoulFrontmatter, WorkspaceRegistry,
    loader::WorkspaceLoader,
    parse::{parse_document, serialize_document, strip_frontmatter},
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

// --- Serialize tests ---

#[test]
fn serialize_document_roundtrip() {
    let raw = "---\nname: TestAgent\nversion: '1.0'\ndescription: A test agent\n---\nHello body";
    let doc = parse_document::<SoulFrontmatter>(raw).unwrap();
    let serialized = serialize_document(&doc).unwrap();
    let reparsed = parse_document::<SoulFrontmatter>(&serialized).unwrap();
    assert_eq!(reparsed.frontmatter.name, doc.frontmatter.name);
    assert_eq!(reparsed.frontmatter.version, doc.frontmatter.version);
    assert_eq!(
        reparsed.frontmatter.description,
        doc.frontmatter.description
    );
    assert_eq!(reparsed.body, doc.body);
}

#[test]
fn serialize_document_empty_body() {
    let doc = Document {
        frontmatter: SoulFrontmatter {
            name: Some("Agent".to_string()),
            version: None,
            description: None,
        },
        body: String::new(),
    };
    let out = serialize_document(&doc).unwrap();
    assert!(out.starts_with("---\n"));
    let reparsed = parse_document::<SoulFrontmatter>(&out).unwrap();
    assert_eq!(reparsed.frontmatter.name.as_deref(), Some("Agent"));
    assert_eq!(reparsed.body, "");
}

#[test]
fn serialize_document_preserves_body_content() {
    let doc = Document {
        frontmatter: SoulFrontmatter {
            name: Some("A".to_string()),
            version: None,
            description: None,
        },
        body: "Line 1\nLine 2\n## Section\n".to_string(),
    };
    let out = serialize_document(&doc).unwrap();
    let reparsed = parse_document::<SoulFrontmatter>(&out).unwrap();
    assert_eq!(reparsed.body, "Line 1\nLine 2\n## Section\n");
}

// --- Save method tests ---

#[tokio::test]
async fn save_soul_writes_file_and_updates_state() {
    let dir = TempDir::new().unwrap();
    let loader = WorkspaceLoader::new(dir.path());

    let doc = Document {
        frontmatter: SoulFrontmatter {
            name: Some("SavedAgent".to_string()),
            version: Some("2.0".to_string()),
            description: Some("Saved description".to_string()),
        },
        body: "Saved body content".to_string(),
    };

    loader.save_soul(&doc).await.unwrap();

    // File on disk must exist and be valid.
    let on_disk = fs::read_to_string(dir.path().join("SOUL.md"))
        .await
        .unwrap();
    assert!(on_disk.contains("SavedAgent"));
    assert!(on_disk.contains("Saved body content"));

    // In-memory state must reflect the update.
    let state = loader.state();
    let state = state.read().await;
    assert_eq!(
        state.soul.as_ref().unwrap().frontmatter.name.as_deref(),
        Some("SavedAgent")
    );
    assert_eq!(state.soul.as_ref().unwrap().body, "Saved body content");
}

#[tokio::test]
async fn save_tools_writes_file_and_updates_state() {
    let dir = TempDir::new().unwrap();
    let loader = WorkspaceLoader::new(dir.path());

    loader
        .save_tools("## My Tools\n\nUse them wisely.")
        .await
        .unwrap();

    let on_disk = fs::read_to_string(dir.path().join("TOOLS.md"))
        .await
        .unwrap();
    assert_eq!(on_disk, "## My Tools\n\nUse them wisely.");

    let state = loader.state();
    let state = state.read().await;
    assert_eq!(
        state.tools_body.as_deref(),
        Some("## My Tools\n\nUse them wisely.")
    );
}

#[tokio::test]
async fn save_soul_preserves_tools_body() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("TOOLS.md"), "## Existing tools")
        .await
        .unwrap();
    let loader = WorkspaceLoader::new(dir.path());
    loader.load_all().await.unwrap();

    let doc = Document {
        frontmatter: SoulFrontmatter {
            name: Some("NewAgent".to_string()),
            version: None,
            description: None,
        },
        body: String::new(),
    };
    loader.save_soul(&doc).await.unwrap();

    // TOOLS.md must be untouched.
    let state = loader.state();
    let state = state.read().await;
    assert_eq!(state.tools_body.as_deref(), Some("## Existing tools"));
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

// --- WorkspaceRegistry create/remove tests ---

fn make_registry_with_base(base: &TempDir) -> Arc<WorkspaceRegistry> {
    Arc::new(WorkspaceRegistry::new(
        "default".into(),
        Some(base.path().to_path_buf()),
    ))
}

#[tokio::test]
async fn create_workspace_creates_dir_and_soul_md() {
    let base = TempDir::new().unwrap();
    let registry = make_registry_with_base(&base);

    let soul = Document {
        frontmatter: SoulFrontmatter {
            name: Some("My Agent".to_string()),
            description: Some("Does stuff".to_string()),
            version: None,
        },
        body: "Agent persona".to_string(),
    };
    let loader = registry
        .create_workspace("my-agent", Some(soul), None)
        .await
        .unwrap();

    // Directory created
    assert!(base.path().join("my-agent").is_dir());

    // SOUL.md written to disk
    let soul_md = fs::read_to_string(base.path().join("my-agent").join("SOUL.md"))
        .await
        .unwrap();
    assert!(soul_md.contains("My Agent"));
    assert!(soul_md.contains("Agent persona"));

    // Loader state loaded
    let state = loader.state();
    let state = state.read().await;
    assert_eq!(
        state.soul.as_ref().unwrap().frontmatter.name.as_deref(),
        Some("My Agent")
    );

    // Registered in registry
    assert!(registry.contains("my-agent").await);
}

#[tokio::test]
async fn create_workspace_with_tools() {
    let base = TempDir::new().unwrap();
    let registry = make_registry_with_base(&base);

    registry
        .create_workspace("with-tools", None, Some("## Tools\n\nSome tools."))
        .await
        .unwrap();

    let tools_md = fs::read_to_string(base.path().join("with-tools").join("TOOLS.md"))
        .await
        .unwrap();
    assert!(tools_md.contains("Some tools."));
}

#[tokio::test]
async fn create_workspace_rejects_invalid_name() {
    let base = TempDir::new().unwrap();
    let registry = make_registry_with_base(&base);

    for bad in &[
        "",
        "default",
        "My-Workspace",
        "-starts-with-hyphen",
        "ends-with-hyphen-",
        "has space",
    ] {
        let result = registry.create_workspace(bad, None, None).await;
        assert!(result.is_err(), "'{bad}' should be rejected");
    }
}

#[tokio::test]
async fn create_workspace_rejects_duplicate() {
    let base = TempDir::new().unwrap();
    let registry = make_registry_with_base(&base);

    registry
        .create_workspace("alpha", None, None)
        .await
        .unwrap();
    match registry.create_workspace("alpha", None, None).await {
        Err(e) => assert!(
            e.to_string().contains("already exists"),
            "unexpected error: {e}"
        ),
        Ok(_) => panic!("expected duplicate error"),
    }
}

#[tokio::test]
async fn create_workspace_rejects_single_mode() {
    let registry = Arc::new(WorkspaceRegistry::new("default".into(), None));
    match registry.create_workspace("new-ws", None, None).await {
        Err(e) => assert!(
            e.to_string().contains("single-workspace"),
            "unexpected error: {e}"
        ),
        Ok(_) => panic!("expected error for single-workspace mode"),
    }
}

#[tokio::test]
async fn remove_workspace_archives_dir() {
    let base = TempDir::new().unwrap();
    let registry = Arc::new(WorkspaceRegistry::new(
        "main".into(),
        Some(base.path().to_path_buf()),
    ));
    registry
        .create_workspace("removable", None, None)
        .await
        .unwrap();
    assert!(base.path().join("removable").is_dir());

    registry.remove_workspace("removable").await.unwrap();

    // Original dir gone, archived dir exists
    assert!(!base.path().join("removable").is_dir());
    let entries: Vec<_> = std::fs::read_dir(base.path())
        .unwrap()
        .filter_map(std::result::Result::ok)
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("removable.archived-")
        })
        .collect();
    assert_eq!(entries.len(), 1, "archived directory should exist");

    // No longer in registry
    assert!(!registry.contains("removable").await);
}

#[tokio::test]
async fn remove_workspace_rejects_default() {
    let base = TempDir::new().unwrap();
    let registry = Arc::new(WorkspaceRegistry::new(
        "default".into(),
        Some(base.path().to_path_buf()),
    ));
    let result = registry.remove_workspace("default").await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("cannot delete the default"),
        "unexpected error: {msg}"
    );
}

#[tokio::test]
async fn remove_workspace_not_found() {
    let base = TempDir::new().unwrap();
    let registry = make_registry_with_base(&base);
    let result = registry.remove_workspace("nonexistent").await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("not found"), "unexpected error: {msg}");
}
