//! File download example.
//!
//! Demonstrates:
//! - Downloading files by URL
//! - Clicking download buttons and waiting for download
//! - Tracking download progress
//!
//! Run with: cargo run --example download

use agent_browser_core::{BrowserConfig, BrowserEngine, DownloadOptions};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    println!("=== File Download Example ===\n");

    let engine = BrowserEngine::new(BrowserConfig::headless());

    // Example 1: Direct download from URL
    println!("1. Direct download from URL...");
    let download_url = "https://www.w3.org/WAI/WCAG21/Techniques/pdf/img/table-word.pdf";

    let options = DownloadOptions {
        save_path: None, // Use temp directory
        timeout_ms: Some(60_000),
    };

    match engine.download_file(download_url, Some(options)).await {
        Ok(result) => {
            println!("   Download successful!");
            println!("   File: {}", result.filename);
            println!("   Path: {}", result.file_path);
            println!("   Size: {} bytes", result.size.unwrap_or(0));
            println!("   Status: {:?}", result.status);
        }
        Err(e) => {
            println!("   Download failed: {}", e);
        }
    }

    // Example 2: Navigate to page and click download button
    println!("\n2. Click to download...");
    println!("   (This would download by clicking a link on a page)");

    // Example showing the API (commented out to avoid actual download)
    /*
    engine.navigate("https://example.com/downloads").await?;

    // First get snapshot to find the download link
    let snapshot = engine.snapshot().await?;

    // Find the download button ref_id from snapshot...
    // Then:
    let result = engine
        .click_and_download("ax5", Some(DownloadOptions {
            save_path: Some("/path/to/save".to_string()),
            timeout_ms: Some(30_000),
        }))
        .await?;
    */

    println!("   (Skipped - see code comments for usage)");

    // Example 3: Upload file (reverse operation)
    println!("\n3. File upload example...");
    println!("   To upload a file:");
    println!("   1. Get snapshot to find the file input ref_id");
    println!("   2. Call engine.upload_file(ref_id, \"/path/to/file.txt\")");

    /*
    // Actual upload example:
    engine.navigate("https://httpbin.org/forms/post").await?;

    // Find file input in snapshot
    let snapshot = engine.snapshot().await?;

    // Upload to the file input element
    engine.upload_file("ref_id_of_input", "/path/to/local/file.txt").await?;
    */

    // Clean up
    println!("\nClosing browser...");
    engine.shutdown().await?;
    println!("Done!");

    Ok(())
}
