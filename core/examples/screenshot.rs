//! Screenshot and snapshot example.
//!
//! Demonstrates:
//! - Taking full-page screenshots
//! - Taking element-specific screenshots
//! - Getting page snapshots with different options
//!
//! Run with: cargo run --example screenshot

use agent_browser_core::{BrowserConfig, BrowserEngine, ScreenshotOptions};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    println!("=== Screenshot Example ===\n");

    // Create browser in headless mode (no visible window)
    let engine = BrowserEngine::new(BrowserConfig::headless());

    // Navigate to a page
    println!("Navigating to https://example.com...");
    engine.navigate("https://example.com").await?;

    // Take a viewport screenshot (visible area only)
    println!("\n1. Viewport screenshot...");
    let viewport = engine.screenshot().await?;
    println!("   Size: {}x{} pixels", viewport.width, viewport.height);
    println!("   Format: {}", viewport.format);
    println!("   Data length: {} bytes (base64)", viewport.data.len());

    // Take a full-page screenshot
    println!("\n2. Full-page screenshot...");
    let full_page = engine
        .screenshot_with_options(ScreenshotOptions {
            full_page: Some(true),
            selector: None,
        })
        .await?;
    println!("   Size: {}x{} pixels", full_page.width, full_page.height);

    // Take a screenshot of a specific element
    println!("\n3. Element-specific screenshot...");
    let element_shot = engine
        .screenshot_with_options(ScreenshotOptions {
            full_page: None,
            selector: Some("h1".to_string()),
        })
        .await;
    match element_shot {
        Ok(shot) => {
            println!("   H1 element: {}x{} pixels", shot.width, shot.height);
        }
        Err(e) => {
            println!("   Could not capture h1: {}", e);
        }
    }

    // Get page snapshot and analyze structure
    println!("\n4. Page snapshot analysis...");
    let snapshot = engine.snapshot().await?;
    println!("   URL: {}", snapshot.url);
    println!("   Title: {}", snapshot.title);

    // Count elements by role
    let mut role_counts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    count_roles(&snapshot.nodes, &mut role_counts);

    println!("   Elements by role:");
    let mut roles: Vec<_> = role_counts.iter().collect();
    roles.sort_by(|a, b| b.1.cmp(a.1));
    for (role, count) in roles.iter().take(10) {
        println!("     - {}: {}", role, count);
    }

    // Clean up
    println!("\nClosing browser...");
    engine.shutdown().await?;
    println!("Done!");

    Ok(())
}

/// Count elements by their role in the accessibility tree
fn count_roles(
    nodes: &[agent_browser_core::SnapshotNode],
    counts: &mut std::collections::HashMap<String, usize>,
) {
    for node in nodes {
        *counts.entry(node.role.clone()).or_insert(0) += 1;
        count_roles(&node.children, counts);
    }
}
