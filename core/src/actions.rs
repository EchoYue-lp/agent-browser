//! Element action module.
//!
//! Supported element actions:
//! - Click (click, double_click, right_click)
//! - Type (type)
//! - Key press (press)
//! - Hover (hover)
//! - Focus (focus)
//! - Select (select)
//! - Scroll (scroll)
//! - Drag (drag)
//! - Wait (wait)

use chromiumoxide::Page;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::error::{Error, Result};

/// Action types.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActionKind {
    /// Single click.
    Click,
    /// Double click.
    DoubleClick,
    /// Right click.
    RightClick,
    /// Type text.
    Type {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        clear_first: Option<bool>,
    },
    /// Press a key.
    Press { key: String },
    /// Hover over element.
    Hover,
    /// Focus element.
    Focus,
    /// Select options.
    Select { values: Vec<String> },
    /// Scroll page.
    Scroll {
        #[serde(skip_serializing_if = "Option::is_none")]
        direction: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        amount: Option<i32>,
    },
    /// Wait.
    Wait {
        #[serde(skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },
    /// Drag element.
    Drag { target_ref_id: String },
}

/// Action execution result.
#[derive(Debug, Clone, Serialize)]
pub struct ActionResult {
    /// Whether the action succeeded.
    pub success: bool,
    /// Result message.
    pub message: String,
}

/// Dispatch an action.
///
/// Executes the specified browser operation.
///
/// # Arguments
///
/// * `page` - Page object
/// * `ref_id` - Element reference ID
/// * `action` - Action type
/// * `snapshot_url` - Optional snapshot URL for page change detection
pub async fn dispatch_action(
    page: &Page,
    ref_id: &str,
    action: ActionKind,
    snapshot_url: Option<&str>,
) -> Result<ActionResult> {
    info!("Action {:?} on ref_id={}", action, ref_id);

    // 页面变化检测
    if let Some(expected_url) = snapshot_url {
        let current = page.url().await.ok().flatten().unwrap_or_default();
        if current != expected_url {
            return Err(Error::PageChanged {
                expected: expected_url.to_string(),
                current,
            });
        }
    }

    // 无需元素的操作
    if let ActionKind::Drag { ref target_ref_id } = action {
        inject_refs(page).await?;
        return dispatch_drag(page, ref_id, target_ref_id).await;
    }
    if let ActionKind::Scroll {
        ref direction,
        amount,
    } = action
    {
        return dispatch_scroll(page, direction.clone(), amount).await;
    }
    if let ActionKind::Wait { timeout_ms } = action {
        let ms = timeout_ms.unwrap_or(1000);
        tokio::time::sleep(Duration::from_millis(ms)).await;
        return Ok(ActionResult {
            success: true,
            message: format!("Waited {}ms", ms),
        });
    }

    // 元素查找
    let selector = format!("[data-agent-ref=\"{}\"]", ref_id);

    if page.find_element(&selector).await.is_err() {
        debug!("ref_id {} not found — injecting refs", ref_id);
        inject_refs(page).await?;
    }

    match execute_action(page, &selector, action.clone()).await {
        Ok(msg) => {
            info!("Action {:?} succeeded: {}", action, msg);
            Ok(ActionResult {
                success: true,
                message: msg,
            })
        }
        Err(e) => {
            warn!("Action {:?} failed: {}", action, e);
            Ok(ActionResult {
                success: false,
                message: e.to_string(),
            })
        }
    }
}

/// Inject ref attributes into DOM elements.
async fn inject_refs(page: &Page) -> Result<()> {
    const INJECT_REFS_JS: &str = r#"
    (() => {
        let counter = parseInt(document.body?.getAttribute('data-agent-counter') || '0', 10);
        function isVisible(el) {
            const s = window.getComputedStyle(el);
            return s.display!=='none' && s.visibility!=='hidden' && s.opacity!=='0';
        }
        function isInteractive(el) {
            const tag = el.tagName.toLowerCase();
            if (['a','button','input','select','textarea','details','summary'].includes(tag)) return true;
            const role = el.getAttribute('role');
            if (['button','link','checkbox','radio','combobox','textbox','menuitem','tab','option','slider'].includes(role)) return true;
            if (el.hasAttribute('onclick') || el.getAttribute('tabindex')==='0') return true;
            return false;
        }
        const queue = [document.body];
        while (queue.length) {
            const el = queue.shift();
            if (!el || el.nodeType !== 1) continue;
            if (isVisible(el) && isInteractive(el) && !el.getAttribute('data-agent-ref')) {
                el.setAttribute('data-agent-ref', 'e' + (++counter));
            }
            for (const c of el.children) queue.push(c);
        }
        if (document.body) document.body.setAttribute('data-agent-counter', String(counter));
        return counter;
    })()
    "#;

    page.evaluate(INJECT_REFS_JS)
        .await
        .map_err(|e| Error::JavaScript(e.to_string()))?;

    Ok(())
}

/// Execute an action on an element.
async fn execute_action(page: &Page, selector: &str, action: ActionKind) -> Result<String> {
    let find_elem = || page.find_element(selector);

    match action {
        ActionKind::Click => {
            find_elem()
                .await
                .map_err(|_| Error::ElementNotFound(selector.to_string()))?
                .click()
                .await
                .map_err(|e| Error::Cdp(e.to_string()))?;
            Ok("Clicked".to_string())
        }

        ActionKind::DoubleClick => {
            let js = format!(
                r#"(() => {{
                    const el = document.querySelector({sel:?});
                    if (!el) throw new Error('Not found');
                    el.dispatchEvent(new MouseEvent('dblclick', {{bubbles:true,cancelable:true,view:window}}));
                }})()"#,
                sel = selector
            );
            page.evaluate(js)
                .await
                .map_err(|e| Error::JavaScript(e.to_string()))?;
            Ok("Double-clicked".to_string())
        }

        ActionKind::RightClick => {
            let js = format!(
                r#"(() => {{
                    const el = document.querySelector({sel:?});
                    if (!el) throw new Error('Not found');
                    el.dispatchEvent(new MouseEvent('contextmenu', {{bubbles:true,cancelable:true,button:2}}));
                }})()"#,
                sel = selector
            );
            page.evaluate(js)
                .await
                .map_err(|e| Error::JavaScript(e.to_string()))?;
            Ok("Right-clicked".to_string())
        }

        ActionKind::Type { text, clear_first } => {
            let elem = find_elem()
                .await
                .map_err(|_| Error::ElementNotFound(selector.to_string()))?;

            if clear_first.unwrap_or(false) {
                elem.click().await.map_err(|e| Error::Cdp(e.to_string()))?;
                elem.press_key("Control+a")
                    .await
                    .map_err(|e| Error::Cdp(e.to_string()))?;
                elem.press_key("Backspace")
                    .await
                    .map_err(|e| Error::Cdp(e.to_string()))?;
            }

            elem.type_str(&text)
                .await
                .map_err(|e| Error::Cdp(e.to_string()))?;

            Ok(format!("Typed: {}", text))
        }

        ActionKind::Press { key } => {
            find_elem()
                .await
                .map_err(|_| Error::ElementNotFound(selector.to_string()))?
                .press_key(&key)
                .await
                .map_err(|e| Error::Cdp(e.to_string()))?;
            Ok(format!("Pressed: {}", key))
        }

        ActionKind::Hover => {
            find_elem()
                .await
                .map_err(|_| Error::ElementNotFound(selector.to_string()))?
                .hover()
                .await
                .map_err(|e| Error::Cdp(e.to_string()))?;
            Ok("Hovered".to_string())
        }

        ActionKind::Focus => {
            let js = format!(
                r#"(() => {{
                    const el = document.querySelector({sel:?});
                    if (!el) throw new Error('Not found');
                    el.focus();
                }})()"#,
                sel = selector
            );
            page.evaluate(js)
                .await
                .map_err(|e| Error::JavaScript(e.to_string()))?;
            Ok("Focused".to_string())
        }

        ActionKind::Select { values } => {
            let values_json = serde_json::to_string(&values)?;
            let js = format!(
                r#"(() => {{
                    const el = document.querySelector({sel:?});
                    if (!el) throw new Error('Not found');
                    const vals = {vj};
                    for (let i = 0; i < el.options.length; i++)
                        el.options[i].selected = vals.includes(el.options[i].value);
                    el.dispatchEvent(new Event('change', {{bubbles:true}}));
                }})()"#,
                sel = selector,
                vj = values_json
            );
            page.evaluate(js)
                .await
                .map_err(|e| Error::JavaScript(e.to_string()))?;
            Ok(format!("Selected: {:?}", values))
        }

        ActionKind::Scroll { .. } | ActionKind::Wait { .. } | ActionKind::Drag { .. } => {
            Err(Error::Other(format!(
                "{:?} must be handled before execute_action",
                action
            )))
        }
    }
}

/// Handle scroll action.
async fn dispatch_scroll(
    page: &Page,
    direction: Option<String>,
    amount: Option<i32>,
) -> Result<ActionResult> {
    let n = amount.unwrap_or(300);
    let dir = direction.as_deref().unwrap_or("down");
    let (dx, dy) = match dir {
        "up" => (0, -n),
        "down" => (0, n),
        "left" => (-n, 0),
        "right" => (n, 0),
        _ => (0, n),
    };

    page.evaluate(format!("window.scrollBy({},{});", dx, dy))
        .await
        .map_err(|e| Error::JavaScript(e.to_string()))?;

    Ok(ActionResult {
        success: true,
        message: format!("Scrolled {} by {}", dir, n),
    })
}

/// Handle drag action.
async fn dispatch_drag(
    page: &Page,
    source_ref_id: &str,
    target_ref_id: &str,
) -> Result<ActionResult> {
    let src_attr = format!("[data-agent-ref={:?}]", source_ref_id);
    let tgt_attr = format!("[data-agent-ref={:?}]", target_ref_id);
    let js = format!(
        r#"
        (() => {{
            const src = document.querySelector({src_sel:?});
            const tgt = document.querySelector({tgt_sel:?});
            if (!src) return {{ok:false,error:'source not found'}};
            if (!tgt) return {{ok:false,error:'target not found'}};
            const dt = new DataTransfer();
            src.dispatchEvent(new DragEvent('dragstart',  {{bubbles:true,cancelable:true,dataTransfer:dt}}));
            tgt.dispatchEvent(new DragEvent('dragenter',  {{bubbles:true,cancelable:true,dataTransfer:dt}}));
            tgt.dispatchEvent(new DragEvent('dragover',   {{bubbles:true,cancelable:true,dataTransfer:dt}}));
            tgt.dispatchEvent(new DragEvent('drop',       {{bubbles:true,cancelable:true,dataTransfer:dt}}));
            src.dispatchEvent(new DragEvent('dragend',    {{bubbles:true,cancelable:true,dataTransfer:dt}}));
            return {{ok:true}};
        }})()
        "#,
        src_sel = src_attr,
        tgt_sel = tgt_attr
    );

    let result: serde_json::Value = page
        .evaluate(js)
        .await
        .map_err(|e| Error::JavaScript(e.to_string()))?
        .into_value()
        .map_err(|e| Error::JavaScript(e.to_string()))?;

    if result["ok"].as_bool().unwrap_or(false) {
        Ok(ActionResult {
            success: true,
            message: format!("Dragged {} → {}", source_ref_id, target_ref_id),
        })
    } else {
        let err = result["error"].as_str().unwrap_or("unknown").to_string();
        Ok(ActionResult {
            success: false,
            message: format!("Drag failed: {}", err),
        })
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_action_kind_click_serialization() {
        let json = r#"{"type":"click"}"#;
        let action: ActionKind = serde_json::from_str(json).unwrap();
        assert!(matches!(action, ActionKind::Click));

        let serialized = serde_json::to_string(&action).unwrap();
        assert_eq!(serialized, r#"{"type":"click"}"#);
    }

    #[test]
    fn test_action_kind_double_click_serialization() {
        let json = r#"{"type":"double_click"}"#;
        let action: ActionKind = serde_json::from_str(json).unwrap();
        assert!(matches!(action, ActionKind::DoubleClick));
    }

    #[test]
    fn test_action_kind_right_click_serialization() {
        let json = r#"{"type":"right_click"}"#;
        let action: ActionKind = serde_json::from_str(json).unwrap();
        assert!(matches!(action, ActionKind::RightClick));
    }

    #[test]
    fn test_action_kind_type_serialization() {
        let json = r#"{"type":"type","text":"hello"}"#;
        let action: ActionKind = serde_json::from_str(json).unwrap();

        if let ActionKind::Type { text, clear_first } = action {
            assert_eq!(text, "hello");
            assert!(clear_first.is_none());
        } else {
            panic!("Expected Type action");
        }
    }

    #[test]
    fn test_action_kind_type_with_clear_serialization() {
        let json = r#"{"type":"type","text":"hello","clear_first":true}"#;
        let action: ActionKind = serde_json::from_str(json).unwrap();

        if let ActionKind::Type { text, clear_first } = action {
            assert_eq!(text, "hello");
            assert_eq!(clear_first, Some(true));
        } else {
            panic!("Expected Type action");
        }
    }

    #[test]
    fn test_action_kind_press_serialization() {
        let json = r#"{"type":"press","key":"Enter"}"#;
        let action: ActionKind = serde_json::from_str(json).unwrap();

        if let ActionKind::Press { key } = action {
            assert_eq!(key, "Enter");
        } else {
            panic!("Expected Press action");
        }
    }

    #[test]
    fn test_action_kind_hover_serialization() {
        let json = r#"{"type":"hover"}"#;
        let action: ActionKind = serde_json::from_str(json).unwrap();
        assert!(matches!(action, ActionKind::Hover));
    }

    #[test]
    fn test_action_kind_focus_serialization() {
        let json = r#"{"type":"focus"}"#;
        let action: ActionKind = serde_json::from_str(json).unwrap();
        assert!(matches!(action, ActionKind::Focus));
    }

    #[test]
    fn test_action_kind_select_serialization() {
        let json = r#"{"type":"select","values":["option1","option2"]}"#;
        let action: ActionKind = serde_json::from_str(json).unwrap();

        if let ActionKind::Select { values } = action {
            assert_eq!(values, vec!["option1", "option2"]);
        } else {
            panic!("Expected Select action");
        }
    }

    #[test]
    fn test_action_kind_scroll_serialization() {
        let json = r#"{"type":"scroll","direction":"down","amount":500}"#;
        let action: ActionKind = serde_json::from_str(json).unwrap();

        if let ActionKind::Scroll { direction, amount } = action {
            assert_eq!(direction, Some("down".to_string()));
            assert_eq!(amount, Some(500));
        } else {
            panic!("Expected Scroll action");
        }
    }

    #[test]
    fn test_action_kind_scroll_minimal_serialization() {
        let json = r#"{"type":"scroll"}"#;
        let action: ActionKind = serde_json::from_str(json).unwrap();

        if let ActionKind::Scroll { direction, amount } = action {
            assert!(direction.is_none());
            assert!(amount.is_none());
        } else {
            panic!("Expected Scroll action");
        }
    }

    #[test]
    fn test_action_kind_wait_serialization() {
        let json = r#"{"type":"wait","timeout_ms":2000}"#;
        let action: ActionKind = serde_json::from_str(json).unwrap();

        if let ActionKind::Wait { timeout_ms } = action {
            assert_eq!(timeout_ms, Some(2000));
        } else {
            panic!("Expected Wait action");
        }
    }

    #[test]
    fn test_action_kind_drag_serialization() {
        let json = r#"{"type":"drag","target_ref_id":"ax42"}"#;
        let action: ActionKind = serde_json::from_str(json).unwrap();

        if let ActionKind::Drag { target_ref_id } = action {
            assert_eq!(target_ref_id, "ax42");
        } else {
            panic!("Expected Drag action");
        }
    }

    #[test]
    fn test_action_result_success() {
        let result = ActionResult {
            success: true,
            message: "Clicked".to_string(),
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"success\":true"));
        assert!(json.contains("\"message\":\"Clicked\""));
    }

    #[test]
    fn test_action_result_failure() {
        let result = ActionResult {
            success: false,
            message: "Element not found".to_string(),
        };

        assert!(!result.success);
        assert_eq!(result.message, "Element not found");
    }
}
