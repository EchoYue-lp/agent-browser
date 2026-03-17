//! Basic browser automation example.
//!
//! Demonstrates:
//! - Creating a browser instance
//! - Navigating to a URL
//! - Getting a page snapshot
//! - Performing element actions
//! - Using CSS selectors for direct operations
//!
//! Run with: cargo run --example basic

use agent_browser_core::{BrowserConfig, BrowserEngine};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logger (optional, for debugging)
    tracing_subscriber::fmt::init();

    println!("=== Basic Browser Automation Example ===\n");

    // Create a browser engine with visible window (headed mode)
    // Use BrowserConfig::headless() for headless mode
    let engine = BrowserEngine::new(BrowserConfig::headed());

    // Navigate to a webpage
    println!("Navigating to https://example.com...");
    let result = engine.navigate("https://example.com").await?;
    println!("Page title: {}", result.title);
    println!("Final URL: {}", result.final_url);

    // Get page snapshot (Accessibility Tree)
    println!("\nGetting page snapshot...");
    let snapshot = engine.snapshot().await?;
    println!("Snapshot ID: {}", snapshot.snapshot_id);
    println!("Total elements: {}", count_nodes(&snapshot.nodes));

    // Print first few elements
    println!("\nFirst 5 interactive elements:");
    print_elements(&snapshot.nodes, 0, 5);

    // Take a screenshot
    println!("\nTaking screenshot...");
    let screenshot = engine.screenshot().await?;
    println!(
        "Screenshot: {}x{} {}",
        screenshot.width, screenshot.height, screenshot.format
    );

    // Example: Using CSS selector operations (recommended approach)
    // These don't require getting ref_id from snapshot first
    println!("\n=== CSS Selector Operations ===");

    // Check if element exists
    let exists = engine.element_exists("h1").await?;
    println!("H1 element exists: {}", exists);

    // Get text content
    if exists {
        let text = engine.get_text("h1", None).await?;
        println!("H1 text: {}", text);
    }

    // Clean up
    println!("\nClosing browser...");
    engine.shutdown().await?;
    println!("Done!");

    Ok(())
}

/// Count total nodes in the snapshot tree
fn count_nodes(nodes: &[agent_browser_core::SnapshotNode]) -> usize {
    nodes.iter().map(|n| 1 + count_nodes(&n.children)).sum()
}

/// Print first N interactive elements from the tree
fn print_elements(nodes: &[agent_browser_core::SnapshotNode], count: usize, max: usize) -> usize {
    let mut current = count;
    for node in nodes {
        if current >= max {
            break;
        }
        if node.attributes.contains_key("interactive") {
            println!("  [{}] {} \"{}\"", node.ref_id, node.role, node.name);
            current += 1;
        }
        current = print_elements(&node.children, current, max);
    }
    current
}
