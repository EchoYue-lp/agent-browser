//! Browser configuration example.
//!
//! Demonstrates:
//! - Different headless modes
//! - Custom browser path
//! - Profile directory for cookie persistence
//! - Stealth mode for anti-detection
//! - Custom browser arguments
//!
//! Run with: cargo run --example configuration

use agent_browser_core::{BrowserConfig, BrowserEngine, HeadlessMode};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    println!("=== Browser Configuration Examples ===\n");

    // Example 1: Headed mode (visible browser window)
    println!("1. Headed mode (visible browser):");
    println!("   let config = BrowserConfig::headed();");

    // Example 2: Headless mode (new - recommended)
    println!("\n2. Headless mode (new, harder to detect):");
    println!("   let config = BrowserConfig::headless();");
    println!("   Uses Chrome's --headless=new flag (Chrome 112+)");

    // Example 3: Headless mode (old - for compatibility)
    println!("\n3. Headless mode (old, more compatible):");
    println!("   let config = BrowserConfig::headless_old();");

    // Example 4: Custom configuration
    println!("\n4. Custom configuration with all options:");
    println!(
        r#"
    let config = BrowserConfig::default()
        // Headless mode
        .with_headless(HeadlessMode::New)

        // Custom Chrome/Chromium path
        .with_browser_path("/usr/bin/google-chrome")

        // Profile directory for cookie persistence
        .with_profile_dir("/path/to/profile")

        // Enable anti-detection scripts
        .with_stealth(true)

        // Add custom browser arguments
        .with_arg("--disable-web-security")
        .with_arg("--window-size=1920,1080");
"#
    );

    // Example 5: Anti-detection configuration
    println!("\n5. Anti-detection configuration (recommended for scraping):");
    let anti_detect_config = BrowserConfig::default()
        .with_headless(HeadlessMode::New) // New headless is harder to detect
        .with_stealth(true); // Inject anti-detection scripts

    println!("   Creating browser with anti-detection config...");
    let engine = BrowserEngine::new(anti_detect_config);

    // Test the configuration
    println!("\n   Testing with https://example.com...");
    let result = engine.navigate("https://example.com").await?;
    println!("   Successfully loaded: {}", result.title);

    // Clean up
    engine.shutdown().await?;

    // Example 6: Show default configuration values
    println!("\n6. Default configuration values:");
    let default = BrowserConfig::default();
    println!("   Headless: {:?}", default.headless);
    println!("   Stealth: {}", default.stealth);
    println!("   Navigation timeout: {}ms", default.navigation_timeout_ms);
    println!("   Action timeout: {}ms", default.action_timeout_ms);
    println!(
        "   Browser path: {}",
        default
            .browser_path
            .unwrap_or_else(|| "auto-detect".into())
            .display()
    );

    println!("\nDone!");
    Ok(())
}
