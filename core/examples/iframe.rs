//! iframe handling example.
//!
//! Demonstrates:
//! - Detecting iframes in the page snapshot
//! - Entering iframe context
//! - Operating on elements inside iframes
//! - Exiting iframe context
//!
//! Run with: cargo run --example iframe

use agent_browser_core::{BrowserConfig, BrowserEngine};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    println!("=== iframe Handling Example ===\n");

    let engine = BrowserEngine::new(BrowserConfig::headed());

    // Navigate to a page with iframes (using a common example site)
    println!("Navigating to a page with iframes...");
    engine
        .navigate("https://www.w3schools.com/html/html_iframe.asp")
        .await?;

    // Wait for page to load
    engine.wait(2000).await?;

    // Get snapshot to find iframes
    println!("\nGetting page snapshot...");
    let snapshot = engine.snapshot().await?;
    println!("Found {} iframe(s)", snapshot.iframe_count);

    // Print iframe information
    for mapping in &snapshot.iframe_mappings {
        println!(
            "  - ref_id: {}, frame_id: {}",
            mapping.ref_id, mapping.frame_id
        );
        if let Some(ref name) = mapping.name {
            println!("    name: {}", name);
        }
        if let Some(ref src) = mapping.src {
            println!("    src: {}", src);
        }
    }

    // If there are iframes, demonstrate entering them
    if let Some(first_iframe) = snapshot.iframe_mappings.first() {
        println!("\nEntering iframe: {}...", first_iframe.ref_id);

        match engine.enter_iframe(&first_iframe.ref_id).await {
            Ok(depth) => {
                println!("Successfully entered iframe, depth: {}", depth);

                // Get snapshot of iframe content
                let iframe_snapshot = engine.snapshot().await?;
                println!("iframe URL: {}", iframe_snapshot.url);
                println!("iframe elements: {}", iframe_snapshot.nodes.len());

                // Exit back to main document
                println!("\nExiting iframe...");
                let depth = engine.exit_iframe().await?;
                println!("Back to depth: {}", depth);
            }
            Err(e) => {
                println!("Could not enter iframe: {}", e);
            }
        }
    }

    // Demonstrate exit_all_iframes
    println!("\nEnsuring we're in main document...");
    engine.exit_all_iframes().await?;

    // Clean up
    println!("\nClosing browser...");
    engine.shutdown().await?;
    println!("Done!");

    Ok(())
}
