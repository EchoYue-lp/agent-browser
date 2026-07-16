use agent_browser_core::{ActionKind, BrowserConfig, BrowserEngine, Error, SnapshotNode};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const TEST_PAGE: &str = r#"<!doctype html>
<html>
  <head><title>Agent Browser E2E</title></head>
  <body>
    <label>Name <input aria-label="Name" onkeydown="if(event.key==='Enter') document.querySelector('#status').textContent='key'" /></label>
    <button onclick="document.querySelector('#status').textContent='clicked'">Submit</button>
    <button draggable="true">Drag source</button>
    <button ondragover="event.preventDefault()" ondrop="event.preventDefault(); document.querySelector('#status').textContent='dropped'">Drop target</button>
    <div id="status">idle</div>
    <div style="height: 1600px"></div>
    <button onclick="document.querySelector('#status').textContent='offscreen'">Offscreen</button>
  </body>
</html>"#;

async fn local_page() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut request = [0_u8; 4096];
                let _ = stream.read(&mut request).await;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    TEST_PAGE.len(),
                    TEST_PAGE
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    (format!("http://{address}"), task)
}

fn find_ref(nodes: &[SnapshotNode], role: &str, name: &str) -> Option<String> {
    for node in nodes {
        if node.role.eq_ignore_ascii_case(role) && node.name == name {
            return Some(node.ref_id.clone());
        }
        if let Some(found) = find_ref(&node.children, role, name) {
            return Some(found);
        }
    }
    None
}

#[tokio::test]
#[ignore = "requires a locally installed Chrome/Chromium browser"]
async fn browser_agent_workflow_and_recovery() {
    let (url, server) = local_page().await;

    let blocked = BrowserEngine::new(BrowserConfig::headless().with_stealth(false));
    let error = blocked.navigate(&url).await.unwrap_err();
    assert!(matches!(error, Error::NetworkAccessDenied(_)));
    assert!(!blocked.is_launched().await);

    let engine = BrowserEngine::new(
        BrowserConfig::headless()
            .with_stealth(false)
            .with_private_networks(true),
    );
    let navigation = engine.navigate(&url).await.unwrap();
    assert_eq!(navigation.title, "Agent Browser E2E");

    let snapshot = engine.snapshot().await.unwrap();
    let input_ref = find_ref(&snapshot.nodes, "textbox", "Name").unwrap();
    let button_ref = find_ref(&snapshot.nodes, "button", "Submit").unwrap();
    let drag_ref = find_ref(&snapshot.nodes, "button", "Drag source").unwrap();
    let drop_ref = find_ref(&snapshot.nodes, "button", "Drop target").unwrap();
    let offscreen_ref = find_ref(&snapshot.nodes, "button", "Offscreen").unwrap();

    engine
        .act_with_snapshot(
            &snapshot.snapshot_id,
            &input_ref,
            ActionKind::Type {
                text: "Codex".to_string(),
                clear_first: Some(true),
            },
        )
        .await
        .unwrap();
    engine
        .act_with_snapshot(
            &snapshot.snapshot_id,
            &input_ref,
            ActionKind::Press {
                key: "Enter".to_string(),
            },
        )
        .await
        .unwrap();
    let key_state = engine
        .evaluate("document.querySelector('#status').textContent")
        .await
        .unwrap();
    assert_eq!(key_state, "key");
    engine
        .act_with_snapshot(&snapshot.snapshot_id, &button_ref, ActionKind::Click)
        .await
        .unwrap();

    let state = engine
        .evaluate(
            "({name: document.querySelector('input').value, status: document.querySelector('#status').textContent})",
        )
        .await
        .unwrap();
    assert_eq!(state["name"], "Codex");
    assert_eq!(state["status"], "clicked");

    engine
        .act_with_snapshot(
            &snapshot.snapshot_id,
            &drag_ref,
            ActionKind::Drag {
                target_ref_id: drop_ref,
            },
        )
        .await
        .unwrap();
    let drag_state = engine
        .evaluate("document.querySelector('#status').textContent")
        .await
        .unwrap();
    assert_eq!(drag_state, "dropped");

    engine
        .act_with_snapshot(&snapshot.snapshot_id, &offscreen_ref, ActionKind::Click)
        .await
        .unwrap();
    let offscreen_state = engine
        .evaluate("document.querySelector('#status').textContent")
        .await
        .unwrap();
    assert_eq!(offscreen_state, "offscreen");

    let next_snapshot = engine.snapshot().await.unwrap();
    let stale = engine
        .act_with_snapshot(&snapshot.snapshot_id, &button_ref, ActionKind::Click)
        .await
        .unwrap_err();
    assert!(matches!(stale, Error::StaleSnapshot { .. }));
    assert_ne!(snapshot.snapshot_id, next_snapshot.snapshot_id);

    engine.shutdown().await.unwrap();
    assert!(!engine.is_launched().await);
    engine.navigate(&url).await.unwrap();
    assert!(engine.is_launched().await);
    engine.shutdown().await.unwrap();

    server.abort();
}
