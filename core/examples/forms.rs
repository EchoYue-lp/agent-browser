//! Form handling and user interaction example.
//!
//! Demonstrates:
//! - Filling out forms using CSS selectors
//! - Clicking buttons
//! - Selecting dropdown options
//! - Keyboard shortcuts
//! - Waiting for elements
//!
//! Run with: cargo run --example forms

use agent_browser_core::{BrowserConfig, BrowserEngine, HeadlessMode, KeyModifier};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    println!("=== Form Handling Example ===\n");

    // Configure browser with stealth mode for better compatibility
    let config = BrowserConfig::default()
        .with_headless(HeadlessMode::New)
        .with_stealth(true);

    let engine = BrowserEngine::new(config);

    // Navigate to a form page
    println!("Navigating to example form...");
    engine.navigate("https://httpbin.org/forms/post").await?;

    // Wait for form to be ready
    engine.wait(1000).await?;

    println!("\nFilling out the form...");

    // Fill text input using CSS selector
    match engine
        .type_selector("input[name='custname']", "John Doe", true, None)
        .await
    {
        Ok(result) => println!("  Name: {} ", result.message),
        Err(e) => println!("  Name field error: {}", e),
    }

    // Fill another field
    match engine
        .type_selector("input[name='custtel']", "555-1234", true, None)
        .await
    {
        Ok(result) => println!("  Phone: {}", result.message),
        Err(e) => println!("  Phone field error: {}", e),
    }

    // Fill email field
    match engine
        .type_selector("input[name='custemail']", "john@example.com", true, None)
        .await
    {
        Ok(result) => println!("  Email: {}", result.message),
        Err(e) => println!("  Email field error: {}", e),
    }

    // Check a radio button by clicking
    println!("\nSelecting radio button...");
    match engine.click_selector("input[value='medium']", None).await {
        Ok(result) => println!("  Size: {}", result.message),
        Err(e) => println!("  Radio error: {}", e),
    }

    // Check a checkbox
    match engine
        .click_selector("input[name='topping'][value='cheese']", None)
        .await
    {
        Ok(result) => println!("  Topping: {}", result.message),
        Err(e) => println!("  Checkbox error: {}", e),
    }

    // Fill textarea
    match engine
        .type_selector(
            "textarea[name='comments']",
            "This is a test order.",
            true,
            None,
        )
        .await
    {
        Ok(result) => println!("  Comments: {}", result.message),
        Err(e) => println!("  Textarea error: {}", e),
    }

    // Demonstrate keyboard shortcuts
    println!("\nKeyboard shortcuts example:");
    println!("  Sending Ctrl+A (select all)...");

    // Press a key with modifier
    let _ = engine
        .press_with_modifiers("a", &[KeyModifier::Control])
        .await;

    // Wait before submitting
    engine.wait(500).await?;

    // Get element text
    println!("\nReading form content...");
    match engine.get_text("legend", None).await {
        Ok(text) => println!("  Form legend: {}", text.trim()),
        Err(e) => println!("  Could not read legend: {}", e),
    }

    // Check if submit button exists
    let submit_exists = engine.element_exists("button[type='submit']").await?;
    println!("  Submit button exists: {}", submit_exists);

    // Note: Not actually submitting to avoid navigation
    println!("\n(Not submitting form to preserve page state)");

    // Clean up
    println!("\nClosing browser...");
    engine.shutdown().await?;
    println!("Done!");

    Ok(())
}
