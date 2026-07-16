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

/// Execution context details for actions dispatched inside an iframe.
pub struct ContextActionOptions<'a> {
    /// URL captured with the snapshot, used to detect navigation races.
    pub snapshot_url: Option<&'a str>,
    /// Snapshot identifier that owns the referenced element.
    pub snapshot_id: Option<&'a str>,
    /// Maximum time to wait for the element to become actionable.
    pub timeout_ms: u64,
    /// Frame content offset relative to the main viewport.
    pub frame_offset: (f64, f64),
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
    snapshot_id: Option<&str>,
    timeout_ms: u64,
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
        let source_selector = element_selector(ref_id, snapshot_id);
        let target_selector = element_selector(target_ref_id, snapshot_id);
        wait_for_actionable(page, None, &source_selector, false, timeout_ms).await?;
        wait_for_actionable(page, None, &target_selector, false, timeout_ms).await?;
        return dispatch_drag(page, ref_id, target_ref_id, snapshot_id).await;
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
    let selector = element_selector(ref_id, snapshot_id);

    if page.find_element(&selector).await.is_err() {
        debug!("ref_id {} not found — injecting refs", ref_id);
        inject_refs(page).await?;
    }

    wait_for_actionable(
        page,
        None,
        &selector,
        matches!(&action, ActionKind::Type { .. }),
        timeout_ms,
    )
    .await?;
    if matches!(
        &action,
        ActionKind::Click | ActionKind::DoubleClick | ActionKind::RightClick | ActionKind::Hover
    ) {
        return dispatch_pointer_action(page, None, &selector, &action, (0.0, 0.0)).await;
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
            Err(e)
        }
    }
}

async fn type_in_context(
    page: &Page,
    context_id: i64,
    selector: &str,
    text: &str,
    clear_first: bool,
) -> Result<ActionResult> {
    use chromiumoxide::cdp::browser_protocol::input::InsertTextParams;
    use chromiumoxide::cdp::js_protocol::runtime::{EvaluateParams, ExecutionContextId};

    let selector_json = serde_json::to_string(selector)?;
    let script = format!(
        r#"(() => {{
            const el = document.querySelector({selector_json});
            if (!el) return false;
            el.focus();
            if ({clear_first}) {{
                if (typeof el.select === 'function') el.select();
                else {{
                    const range = document.createRange();
                    range.selectNodeContents(el);
                    const selection = getSelection();
                    selection.removeAllRanges();
                    selection.addRange(range);
                }}
            }}
            return true;
        }})()"#
    );
    let params = EvaluateParams::builder()
        .expression(script)
        .context_id(ExecutionContextId::new(context_id))
        .return_by_value(true)
        .build()
        .map_err(Error::InvalidParameter)?;
    let focused: bool = page
        .evaluate_expression(params)
        .await
        .map_err(|error| Error::JavaScript(error.to_string()))?
        .into_value()
        .map_err(|error| Error::JavaScript(error.to_string()))?;
    if !focused {
        return Err(Error::ElementNotFound(selector.to_string()));
    }
    page.execute(InsertTextParams::new(text))
        .await
        .map_err(|error| Error::Cdp(error.to_string()))?;
    Ok(ActionResult {
        success: true,
        message: format!("Typed {} characters", text.chars().count()),
    })
}

async fn press_in_context(
    page: &Page,
    context_id: i64,
    selector: &str,
    key: &str,
) -> Result<ActionResult> {
    use chromiumoxide::cdp::browser_protocol::input::{
        DispatchKeyEventParams, DispatchKeyEventType,
    };
    use chromiumoxide::cdp::js_protocol::runtime::{EvaluateParams, ExecutionContextId};

    let selector_json = serde_json::to_string(selector)?;
    let params = EvaluateParams::builder()
        .expression(format!(
            "(() => {{ const el=document.querySelector({selector_json}); if (!el) return false; el.focus(); return true; }})()"
        ))
        .context_id(ExecutionContextId::new(context_id))
        .return_by_value(true)
        .build()
        .map_err(Error::InvalidParameter)?;
    let focused: bool = page
        .evaluate_expression(params)
        .await
        .map_err(|error| Error::JavaScript(error.to_string()))?
        .into_value()
        .map_err(|error| Error::JavaScript(error.to_string()))?;
    if !focused {
        return Err(Error::ElementNotFound(selector.to_string()));
    }

    let definition = chromiumoxide::keys::get_key_definition(key)
        .ok_or_else(|| Error::InvalidParameter(format!("Unknown key: {key}")))?;
    let mut command = DispatchKeyEventParams::builder();
    let key_down_type = if let Some(text) = definition.text {
        command = command.text(text);
        DispatchKeyEventType::KeyDown
    } else if definition.key.len() == 1 {
        command = command.text(definition.key);
        DispatchKeyEventType::KeyDown
    } else {
        DispatchKeyEventType::RawKeyDown
    };
    command = command
        .key(definition.key)
        .code(definition.code)
        .windows_virtual_key_code(definition.key_code)
        .native_virtual_key_code(definition.key_code);
    page.execute(
        command
            .clone()
            .r#type(key_down_type)
            .build()
            .map_err(Error::InvalidParameter)?,
    )
    .await
    .map_err(|error| Error::Cdp(error.to_string()))?;
    page.execute(
        command
            .r#type(DispatchKeyEventType::KeyUp)
            .build()
            .map_err(Error::InvalidParameter)?,
    )
    .await
    .map_err(|error| Error::Cdp(error.to_string()))?;
    Ok(ActionResult {
        success: true,
        message: format!("Pressed {key}"),
    })
}

/// Dispatch an action inside a specific CDP execution context.
pub async fn dispatch_action_in_context(
    page: &Page,
    context_id: i64,
    ref_id: &str,
    action: ActionKind,
    options: ContextActionOptions<'_>,
) -> Result<ActionResult> {
    use chromiumoxide::cdp::js_protocol::runtime::{EvaluateParams, ExecutionContextId};

    if let Some(expected_url) = options.snapshot_url {
        let current = page.url().await.ok().flatten().unwrap_or_default();
        if current != expected_url {
            return Err(Error::PageChanged {
                expected: expected_url.to_string(),
                current,
            });
        }
    }

    if let ActionKind::Wait { timeout_ms } = action {
        let ms = timeout_ms.unwrap_or(1000);
        tokio::time::sleep(Duration::from_millis(ms)).await;
        return Ok(ActionResult {
            success: true,
            message: format!("Waited {ms}ms"),
        });
    }

    let selector = element_selector(ref_id, options.snapshot_id);
    if let ActionKind::Drag { target_ref_id } = &action {
        let target = element_selector(target_ref_id, options.snapshot_id);
        wait_for_actionable(page, Some(context_id), &target, false, options.timeout_ms).await?;
    }
    wait_for_actionable(
        page,
        Some(context_id),
        &selector,
        matches!(&action, ActionKind::Type { .. }),
        options.timeout_ms,
    )
    .await?;
    if let ActionKind::Drag { target_ref_id } = &action {
        let target = element_selector(target_ref_id, options.snapshot_id);
        return dispatch_drag_pointer(
            page,
            Some(context_id),
            &selector,
            &target,
            options.frame_offset,
        )
        .await;
    }
    if let ActionKind::Type { text, clear_first } = &action {
        return type_in_context(
            page,
            context_id,
            &selector,
            text,
            clear_first.unwrap_or(false),
        )
        .await;
    }
    if let ActionKind::Press { key } = &action {
        return press_in_context(page, context_id, &selector, key).await;
    }
    if matches!(
        &action,
        ActionKind::Click | ActionKind::DoubleClick | ActionKind::RightClick | ActionKind::Hover
    ) {
        return dispatch_pointer_action(
            page,
            Some(context_id),
            &selector,
            &action,
            options.frame_offset,
        )
        .await;
    }
    let selector_json = serde_json::to_string(&selector)?;
    let script = match action {
        ActionKind::Click => format!(
            "(() => {{ const el = document.querySelector({selector_json}); if (!el) return {{ok:false,error:'Element not found'}}; el.click(); return {{ok:true,message:'Clicked'}}; }})()"
        ),
        ActionKind::DoubleClick => format!(
            "(() => {{ const el = document.querySelector({selector_json}); if (!el) return {{ok:false,error:'Element not found'}}; el.dispatchEvent(new MouseEvent('dblclick', {{bubbles:true,cancelable:true,view:window}})); return {{ok:true,message:'Double-clicked'}}; }})()"
        ),
        ActionKind::RightClick => format!(
            "(() => {{ const el = document.querySelector({selector_json}); if (!el) return {{ok:false,error:'Element not found'}}; el.dispatchEvent(new MouseEvent('contextmenu', {{bubbles:true,cancelable:true,button:2}})); return {{ok:true,message:'Right-clicked'}}; }})()"
        ),
        ActionKind::Hover => format!(
            "(() => {{ const el = document.querySelector({selector_json}); if (!el) return {{ok:false,error:'Element not found'}}; el.dispatchEvent(new MouseEvent('mouseover', {{bubbles:true,cancelable:true,view:window}})); el.dispatchEvent(new MouseEvent('mouseenter', {{bubbles:false,cancelable:true,view:window}})); return {{ok:true,message:'Hovered'}}; }})()"
        ),
        ActionKind::Focus => format!(
            "(() => {{ const el = document.querySelector({selector_json}); if (!el) return {{ok:false,error:'Element not found'}}; el.focus(); return {{ok:true,message:'Focused'}}; }})()"
        ),
        ActionKind::Type { text, clear_first } => {
            let text_json = serde_json::to_string(&text)?;
            format!(
                "(() => {{ const el = document.querySelector({selector_json}); if (!el) return {{ok:false,error:'Element not found'}}; el.focus(); if ({clear}) el.value = ''; el.value += {text_json}; el.dispatchEvent(new Event('input', {{bubbles:true}})); el.dispatchEvent(new Event('change', {{bubbles:true}})); return {{ok:true,message:'Typed text'}}; }})()",
                clear = clear_first.unwrap_or(false)
            )
        }
        ActionKind::Press { key } => {
            let key_json = serde_json::to_string(&key)?;
            format!(
                "(() => {{ const el = document.querySelector({selector_json}); if (!el) return {{ok:false,error:'Element not found'}}; el.focus(); el.dispatchEvent(new KeyboardEvent('keydown', {{key:{key_json},bubbles:true}})); el.dispatchEvent(new KeyboardEvent('keyup', {{key:{key_json},bubbles:true}})); return {{ok:true,message:'Pressed key'}}; }})()"
            )
        }
        ActionKind::Select { values } => {
            let values_json = serde_json::to_string(&values)?;
            format!(
                "(() => {{ const el = document.querySelector({selector_json}); if (!el) return {{ok:false,error:'Element not found'}}; if (!el.options) return {{ok:false,error:'Element is not a select'}}; const values = {values_json}; for (const option of el.options) option.selected = values.includes(option.value); el.dispatchEvent(new Event('change', {{bubbles:true}})); return {{ok:true,message:'Selected options'}}; }})()"
            )
        }
        ActionKind::Scroll { direction, amount } => {
            let amount = amount.unwrap_or(300);
            let (dx, dy) = match direction.as_deref().unwrap_or("down") {
                "up" => (0, -amount),
                "down" => (0, amount),
                "left" => (-amount, 0),
                "right" => (amount, 0),
                invalid => {
                    return Err(Error::InvalidParameter(format!(
                        "Invalid scroll direction: {invalid}"
                    )));
                }
            };
            format!(
                "(() => {{ window.scrollBy({dx},{dy}); return {{ok:true,message:'Scrolled'}}; }})()"
            )
        }
        ActionKind::Drag { target_ref_id } => {
            let target = element_selector(&target_ref_id, options.snapshot_id);
            let target_json = serde_json::to_string(&target)?;
            format!(
                "(() => {{ const src = document.querySelector({selector_json}); const target = document.querySelector({target_json}); if (!src || !target) return {{ok:false,error:'Drag source or target not found'}}; const data = new DataTransfer(); src.dispatchEvent(new DragEvent('dragstart', {{bubbles:true,dataTransfer:data}})); target.dispatchEvent(new DragEvent('dragover', {{bubbles:true,dataTransfer:data}})); target.dispatchEvent(new DragEvent('drop', {{bubbles:true,dataTransfer:data}})); src.dispatchEvent(new DragEvent('dragend', {{bubbles:true,dataTransfer:data}})); return {{ok:true,message:'Dragged'}}; }})()"
            )
        }
        ActionKind::Wait { .. } => {
            return Err(Error::Other("Wait action was not handled".to_string()));
        }
    };

    let params = EvaluateParams::builder()
        .expression(script)
        .context_id(ExecutionContextId::new(context_id))
        .await_promise(true)
        .return_by_value(true)
        .build()
        .map_err(Error::InvalidParameter)?;
    let result: serde_json::Value = page
        .evaluate_expression(params)
        .await
        .map_err(|e| Error::JavaScript(e.to_string()))?
        .into_value()
        .map_err(|e| Error::JavaScript(e.to_string()))?;

    if result["ok"].as_bool().unwrap_or(false) {
        Ok(ActionResult {
            success: true,
            message: result["message"]
                .as_str()
                .unwrap_or("Action completed")
                .to_string(),
        })
    } else {
        Err(Error::ElementNotFound(
            result["error"]
                .as_str()
                .unwrap_or("Element action failed")
                .to_string(),
        ))
    }
}

/// Dispatch a trusted CDP pointer action after resolving the target in a frame context.
pub async fn dispatch_pointer_action(
    page: &Page,
    context_id: Option<i64>,
    selector: &str,
    action: &ActionKind,
    frame_offset: (f64, f64),
) -> Result<ActionResult> {
    use chromiumoxide::cdp::browser_protocol::input::{
        DispatchMouseEventParams, DispatchMouseEventType, MouseButton,
    };
    use chromiumoxide::cdp::js_protocol::runtime::{EvaluateParams, ExecutionContextId};

    let selector_json = serde_json::to_string(selector)?;
    let script = format!(
        "(() => {{ const el = document.querySelector({selector_json}); if (!el) return null; el.scrollIntoView({{block:'center',inline:'center'}}); const r=el.getBoundingClientRect(); return {{x:r.left+r.width/2,y:r.top+r.height/2}}; }})()"
    );
    let point: serde_json::Value = if let Some(context_id) = context_id {
        let params = EvaluateParams::builder()
            .expression(script)
            .context_id(ExecutionContextId::new(context_id))
            .return_by_value(true)
            .build()
            .map_err(Error::InvalidParameter)?;
        page.evaluate_expression(params)
            .await
            .map_err(|error| Error::JavaScript(error.to_string()))?
            .into_value()
            .map_err(|error| Error::JavaScript(error.to_string()))?
    } else {
        page.evaluate(script)
            .await
            .map_err(|error| Error::JavaScript(error.to_string()))?
            .into_value()
            .map_err(|error| Error::JavaScript(error.to_string()))?
    };
    if point.is_null() {
        return Err(Error::ElementNotFound(selector.to_string()));
    }
    let x = point["x"].as_f64().unwrap_or_default() + frame_offset.0;
    let y = point["y"].as_f64().unwrap_or_default() + frame_offset.1;

    page.execute(DispatchMouseEventParams::new(
        DispatchMouseEventType::MouseMoved,
        x,
        y,
    ))
    .await
    .map_err(|error| Error::Cdp(error.to_string()))?;
    if matches!(action, ActionKind::Hover) {
        return Ok(ActionResult {
            success: true,
            message: format!("Hovered at ({x:.1}, {y:.1})"),
        });
    }

    let (button, buttons, click_count, message) = match action {
        ActionKind::RightClick => (MouseButton::Right, 2, 1, "Right-clicked"),
        ActionKind::DoubleClick => (MouseButton::Left, 1, 2, "Double-clicked"),
        _ => (MouseButton::Left, 1, 1, "Clicked"),
    };
    let mut pressed = DispatchMouseEventParams::new(DispatchMouseEventType::MousePressed, x, y);
    pressed.button = Some(button.clone());
    pressed.buttons = Some(buttons);
    pressed.click_count = Some(click_count);
    page.execute(pressed)
        .await
        .map_err(|error| Error::Cdp(error.to_string()))?;
    let mut released = DispatchMouseEventParams::new(DispatchMouseEventType::MouseReleased, x, y);
    released.button = Some(button);
    released.buttons = Some(0);
    released.click_count = Some(click_count);
    page.execute(released)
        .await
        .map_err(|error| Error::Cdp(error.to_string()))?;

    Ok(ActionResult {
        success: true,
        message: message.to_string(),
    })
}

/// Wait until an element can be interacted with reliably.
pub async fn wait_for_actionable(
    page: &Page,
    context_id: Option<i64>,
    selector: &str,
    require_editable: bool,
    timeout_ms: u64,
) -> Result<()> {
    use chromiumoxide::cdp::js_protocol::runtime::{EvaluateParams, ExecutionContextId};

    let selector_json = serde_json::to_string(selector)?;
    let script = format!(
        r#"(async () => {{
            const selector = {selector_json};
            const requireEditable = {require_editable};
            const deadline = performance.now() + {timeout_ms};
            let lastReason = 'element not found';
            while (performance.now() <= deadline) {{
                const el = document.querySelector(selector);
                if (!el) {{ lastReason = 'element not found'; await new Promise(r => setTimeout(r, 50)); continue; }}
                const style = getComputedStyle(el);
                const rect1 = el.getBoundingClientRect();
                const visible = style.visibility !== 'hidden' && style.display !== 'none' &&
                    style.opacity !== '0' && style.pointerEvents !== 'none' && rect1.width > 0 && rect1.height > 0;
                if (!visible) {{ lastReason = 'element is not visible'; await new Promise(r => setTimeout(r, 50)); continue; }}
                const inViewport = rect1.bottom > 0 && rect1.right > 0 && rect1.top < innerHeight && rect1.left < innerWidth;
                if (!inViewport) {{
                    lastReason = 'element is outside the viewport';
                    el.scrollIntoView({{block:'center', inline:'center', behavior:'instant'}});
                    await new Promise(requestAnimationFrame);
                    await new Promise(requestAnimationFrame);
                    continue;
                }}
                await new Promise(requestAnimationFrame);
                await new Promise(requestAnimationFrame);
                const rect2 = el.getBoundingClientRect();
                const stable = Math.abs(rect1.x - rect2.x) < 0.5 && Math.abs(rect1.y - rect2.y) < 0.5 &&
                    Math.abs(rect1.width - rect2.width) < 0.5 && Math.abs(rect1.height - rect2.height) < 0.5;
                if (!stable) {{ lastReason = 'element is moving or animating'; continue; }}
                const disabled = el.disabled === true || el.matches?.(':disabled') || el.getAttribute('aria-disabled') === 'true';
                if (disabled) {{ lastReason = 'element is disabled'; await new Promise(r => setTimeout(r, 50)); continue; }}
                if (requireEditable) {{
                    const editable = el.isContentEditable ||
                        ((el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) && !el.readOnly);
                    if (!editable) {{ lastReason = 'element is not editable'; await new Promise(r => setTimeout(r, 50)); continue; }}
                }}
                const x = Math.max(0, Math.min(innerWidth - 1, rect2.left + rect2.width / 2));
                const y = Math.max(0, Math.min(innerHeight - 1, rect2.top + rect2.height / 2));
                const hit = document.elementFromPoint(x, y);
                if (!hit || (hit !== el && !el.contains(hit))) {{
                    lastReason = 'element does not receive pointer events';
                    await new Promise(r => setTimeout(r, 50));
                    continue;
                }}
                return {{ok:true}};
            }}
            return {{ok:false, reason:lastReason}};
        }})()"#
    );

    let value: serde_json::Value = if let Some(context_id) = context_id {
        let params = EvaluateParams::builder()
            .expression(script)
            .context_id(ExecutionContextId::new(context_id))
            .await_promise(true)
            .return_by_value(true)
            .build()
            .map_err(Error::InvalidParameter)?;
        page.evaluate_expression(params)
            .await
            .map_err(|error| Error::JavaScript(error.to_string()))?
            .into_value()
            .map_err(|error| Error::JavaScript(error.to_string()))?
    } else {
        page.evaluate(script)
            .await
            .map_err(|error| Error::JavaScript(error.to_string()))?
            .into_value()
            .map_err(|error| Error::JavaScript(error.to_string()))?
    };

    if value["ok"].as_bool().unwrap_or(false) {
        Ok(())
    } else {
        Err(Error::ElementNotActionable(
            value["reason"]
                .as_str()
                .unwrap_or("actionability check timed out")
                .to_string(),
        ))
    }
}

fn element_selector(ref_id: &str, snapshot_id: Option<&str>) -> String {
    match snapshot_id {
        Some(snapshot_id) => {
            format!("[data-agent-ref={ref_id:?}][data-agent-snapshot={snapshot_id:?}]")
        }
        None => format!("[data-agent-ref={ref_id:?}]"),
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
            use chromiumoxide::cdp::browser_protocol::input::InsertTextParams;

            let clear_first = clear_first.unwrap_or(false);
            let script = format!(
                r#"(() => {{
                    const el = document.querySelector({selector:?});
                    if (!el) return false;
                    el.focus();
                    if ({clear_first}) {{
                        if (typeof el.select === 'function') el.select();
                        else {{
                            const range = document.createRange();
                            range.selectNodeContents(el);
                            const selection = getSelection();
                            selection.removeAllRanges();
                            selection.addRange(range);
                        }}
                    }} else if (typeof el.setSelectionRange === 'function') {{
                        const end = String(el.value || '').length;
                        el.setSelectionRange(end, end);
                    }}
                    return true;
                }})()"#
            );
            let focused: bool = page
                .evaluate(script)
                .await
                .map_err(|error| Error::JavaScript(error.to_string()))?
                .into_value()
                .map_err(|error| Error::JavaScript(error.to_string()))?;
            if !focused {
                return Err(Error::ElementNotFound(selector.to_string()));
            }
            page.execute(InsertTextParams::new(&text))
                .await
                .map_err(|error| Error::Cdp(error.to_string()))?;

            Ok(format!("Typed {} characters", text.chars().count()))
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

    // 验证滚动方向
    let (dx, dy) = match dir {
        "up" => (0, -n),
        "down" => (0, n),
        "left" => (-n, 0),
        "right" => (n, 0),
        invalid => {
            return Err(Error::InvalidParameter(format!(
                "Invalid scroll direction '{}'. Must be one of: up, down, left, right",
                invalid
            )));
        }
    };

    page.evaluate(format!("window.scrollBy({},{});", dx, dy))
        .await
        .map_err(|e| Error::JavaScript(e.to_string()))?;

    Ok(ActionResult {
        success: true,
        message: format!("Scrolled {} by {}px", dir, n),
    })
}

/// Handle drag action.
async fn dispatch_drag(
    page: &Page,
    source_ref_id: &str,
    target_ref_id: &str,
    snapshot_id: Option<&str>,
) -> Result<ActionResult> {
    let src_attr = element_selector(source_ref_id, snapshot_id);
    let tgt_attr = element_selector(target_ref_id, snapshot_id);
    dispatch_drag_pointer(page, None, &src_attr, &tgt_attr, (0.0, 0.0)).await
}

async fn dispatch_drag_pointer(
    page: &Page,
    context_id: Option<i64>,
    source_selector: &str,
    target_selector: &str,
    frame_offset: (f64, f64),
) -> Result<ActionResult> {
    use chromiumoxide::cdp::browser_protocol::input::{
        DispatchDragEventParams, DispatchDragEventType, DispatchMouseEventParams,
        DispatchMouseEventType, DragDataItem, EventDragIntercepted, MouseButton,
        SetInterceptDragsParams,
    };
    use chromiumoxide::cdp::js_protocol::runtime::{EvaluateParams, ExecutionContextId};
    use futures::StreamExt;

    let source_json = serde_json::to_string(source_selector)?;
    let target_json = serde_json::to_string(target_selector)?;
    let expression = format!(
        r#"(() => {{
            const source = document.querySelector({source_json});
            const target = document.querySelector({target_json});
            if (!source || !target) return null;
            const sourceRect = source.getBoundingClientRect();
            const targetRect = target.getBoundingClientRect();
            return {{
                source: {{x: sourceRect.left + sourceRect.width / 2, y: sourceRect.top + sourceRect.height / 2}},
                target: {{x: targetRect.left + targetRect.width / 2, y: targetRect.top + targetRect.height / 2}}
            }};
        }})()"#
    );
    let value: serde_json::Value = if let Some(context_id) = context_id {
        let params = EvaluateParams::builder()
            .expression(expression)
            .context_id(ExecutionContextId::new(context_id))
            .return_by_value(true)
            .build()
            .map_err(Error::InvalidParameter)?;
        page.evaluate_expression(params)
            .await
            .map_err(|error| Error::JavaScript(error.to_string()))?
            .into_value()
            .map_err(|error| Error::JavaScript(error.to_string()))?
    } else {
        page.evaluate(expression)
            .await
            .map_err(|error| Error::JavaScript(error.to_string()))?
            .into_value()
            .map_err(|error| Error::JavaScript(error.to_string()))?
    };
    if value.is_null() {
        return Err(Error::ElementNotFound(
            "Drag source or target not found".to_string(),
        ));
    }

    let source_x = value["source"]["x"].as_f64().unwrap_or_default() + frame_offset.0;
    let source_y = value["source"]["y"].as_f64().unwrap_or_default() + frame_offset.1;
    let target_x = value["target"]["x"].as_f64().unwrap_or_default() + frame_offset.0;
    let target_y = value["target"]["y"].as_f64().unwrap_or_default() + frame_offset.1;

    let mut drag_events = page
        .event_listener::<EventDragIntercepted>()
        .await
        .map_err(|error| Error::Cdp(error.to_string()))?;
    page.execute(SetInterceptDragsParams::new(true))
        .await
        .map_err(|error| Error::Cdp(error.to_string()))?;

    let drag_result = async {
        page.execute(DispatchMouseEventParams::new(
            DispatchMouseEventType::MouseMoved,
            source_x,
            source_y,
        ))
        .await
        .map_err(|error| Error::Cdp(error.to_string()))?;
        let mut pressed =
            DispatchMouseEventParams::new(DispatchMouseEventType::MousePressed, source_x, source_y);
        pressed.button = Some(MouseButton::Left);
        pressed.buttons = Some(1);
        pressed.click_count = Some(1);
        page.execute(pressed)
            .await
            .map_err(|error| Error::Cdp(error.to_string()))?;

        for step in 1..=10 {
            let progress = f64::from(step) / 10.0;
            let mut moved = DispatchMouseEventParams::new(
                DispatchMouseEventType::MouseMoved,
                source_x + (target_x - source_x) * progress,
                source_y + (target_y - source_y) * progress,
            );
            moved.button = Some(MouseButton::Left);
            moved.buttons = Some(1);
            page.execute(moved)
                .await
                .map_err(|error| Error::Cdp(format!("drag move failed: {error}")))?;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        if let Ok(Some(intercepted)) =
            tokio::time::timeout(Duration::from_secs(2), drag_events.next()).await
        {
            for event_type in [
                DispatchDragEventType::DragEnter,
                DispatchDragEventType::DragOver,
                DispatchDragEventType::Drop,
            ] {
                let event_label = format!("{event_type:?}");
                let mut drag_data = intercepted.data.clone();
                if drag_data.drag_operations_mask < 0 {
                    // Chrome reports -1 for "all operations", while
                    // Input.dispatchDragEvent accepts only a non-negative mask.
                    drag_data.drag_operations_mask = 17; // Copy | Move
                }
                if drag_data.items.is_empty() {
                    // chromiumoxide omits an empty items array, but Chrome
                    // requires Input.DragData.items to be present.
                    drag_data.items.push(DragDataItem::new("text/plain", ""));
                }
                page.execute(DispatchDragEventParams::new(
                    event_type, target_x, target_y, drag_data,
                ))
                .await
                .map_err(|error| Error::Cdp(format!("drag event {event_label} failed: {error}")))?;
            }
        }

        let mut released = DispatchMouseEventParams::new(
            DispatchMouseEventType::MouseReleased,
            target_x,
            target_y,
        );
        released.button = Some(MouseButton::Left);
        released.buttons = Some(0);
        released.click_count = Some(1);
        page.execute(released)
            .await
            .map_err(|error| Error::Cdp(error.to_string()))?;
        Ok::<(), Error>(())
    }
    .await;

    let disable_result = page
        .execute(SetInterceptDragsParams::new(false))
        .await
        .map_err(|error| Error::Cdp(error.to_string()));
    drag_result?;
    disable_result?;

    Ok(ActionResult {
        success: true,
        message: "Dragged element".to_string(),
    })
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
