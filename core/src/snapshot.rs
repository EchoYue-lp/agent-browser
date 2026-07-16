//! Page snapshot generation.
//!
//! Accessibility Tree extraction based on CDP Accessibility API.
//!
//! ## Why CDP over pure JS
//!
//! - **Role/name/description** computed by browser engine, accurate and reliable
//! - **State attributes** (checked, expanded, disabled, etc.) from authoritative source
//! - **AxNodeId** stable within page lifecycle
//! - **Hidden elements** automatically filtered by engine
//!
//! ## iframe Support
//!
//! - Same-origin iframes: direct access via contentDocument
//! - Cross-origin iframes: separate AX Tree fetch via CDP for each frame

use chromiumoxide::Page;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::types::Bounds;

/// Snapshot node.
///
/// Represents a node in the Accessibility Tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotNode {
    /// Stable reference ID.
    ///
    /// Format: `ax<n>` (e.g. `ax1`, `ax42`) or `e<n>` (JS fallback)
    /// Injected as `data-agent-ref` attribute on DOM elements.
    pub ref_id: String,

    /// Accessibility role (e.g. "button", "link", "textbox").
    pub role: String,

    /// Accessibility name (e.g. button text, link text).
    pub name: String,

    /// Element value (e.g. input content).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,

    /// Description text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Geometric bounds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bounds: Option<Bounds>,

    /// State attributes (checked, expanded, disabled, etc.).
    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    pub attributes: HashMap<String, String>,

    /// Child nodes.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub children: Vec<SnapshotNode>,
}

/// iframe mapping information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IframeMapping {
    /// Reference ID.
    pub ref_id: String,
    /// CDP frame ID.
    pub frame_id: String,
    /// iframe name attribute.
    pub name: Option<String>,
    /// iframe src attribute.
    pub src: Option<String>,
}

/// Page snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageSnapshot {
    /// Snapshot ID.
    pub snapshot_id: String,
    /// Page URL.
    pub url: String,
    /// Page title.
    pub title: String,
    /// Root node list.
    pub nodes: Vec<SnapshotNode>,
    /// Timestamp (Unix seconds).
    pub timestamp: i64,
    /// iframe count.
    #[serde(default)]
    pub iframe_count: usize,
    /// iframe mappings (ref_id -> frame_id).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub iframe_mappings: Vec<IframeMapping>,
}

/// Controls how much of a page snapshot is returned to an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotOptions {
    /// Keep only actionable nodes and the ancestors needed to reach them.
    #[serde(default = "default_true")]
    pub interactive_only: bool,
    /// Optional subtree root reference.
    #[serde(default)]
    pub root_ref: Option<String>,
    /// Maximum tree depth to return.
    #[serde(default)]
    pub max_depth: Option<usize>,
    /// Maximum number of nodes to return.
    #[serde(default = "default_max_nodes")]
    pub max_nodes: usize,
}

impl Default for SnapshotOptions {
    fn default() -> Self {
        Self {
            interactive_only: true,
            root_ref: None,
            max_depth: Some(8),
            max_nodes: default_max_nodes(),
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_max_nodes() -> usize {
    250
}

/// Compact identity of a node used by search and snapshot diffs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotNodeSummary {
    pub ref_id: String,
    pub role: String,
    pub name: String,
    pub path: String,
}

/// Search result in the latest page snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotSearchResult {
    pub snapshot_id: String,
    pub matches: Vec<SnapshotNodeSummary>,
}

/// Changes between two consecutive observations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotDiff {
    pub from_snapshot_id: String,
    pub to_snapshot_id: String,
    pub added: Vec<SnapshotNodeSummary>,
    pub removed: Vec<SnapshotNodeSummary>,
    pub changed: Vec<SnapshotNodeSummary>,
}

/// Generate page snapshot using CDP (supports iframes).
///
/// Falls back to JS implementation if CDP fails.
pub async fn generate_snapshot(page: &Page) -> Result<PageSnapshot> {
    // Try CDP first (supports cross-origin iframes)
    match generate_snapshot_cdp_with_frames(page).await {
        Ok(snap) if !snap.nodes.is_empty() => {
            info!(
                "CDP snapshot successful with {} nodes",
                count_nodes(&snap.nodes)
            );
            return Ok(snap);
        }
        Ok(_snap) => {
            warn!("CDP snapshot returned empty, trying JS fallback");
        }
        Err(e) => {
            warn!("CDP snapshot failed ({}), falling back to JS", e);
        }
    }

    // Fall back to JS
    generate_snapshot_js_with_frames(page).await
}

/// Count total nodes.
fn count_nodes(nodes: &[SnapshotNode]) -> usize {
    nodes.iter().map(|n| 1 + count_nodes(&n.children)).sum()
}

/// Generate snapshot using CDP (supports multiple frames).
async fn generate_snapshot_cdp_with_frames(page: &Page) -> Result<PageSnapshot> {
    use chromiumoxide::cdp::browser_protocol::accessibility::{EnableParams, GetFullAxTreeParams};
    use chromiumoxide::cdp::browser_protocol::page::GetFrameTreeParams;

    let snapshot_id = Uuid::new_v4().to_string();
    debug!("CDP: enabling Accessibility domain");
    let _ = page.execute(EnableParams {}).await;

    // Get frame tree
    let frame_tree = page
        .execute(GetFrameTreeParams {})
        .await
        .map_err(|e| Error::Cdp(e.to_string()))?;

    let main_frame_id: String = frame_tree.frame_tree.frame.id.clone().into();
    debug!("CDP: main frame id = {:?}", main_frame_id);

    // Build frame info map: frame_id -> {name, url}
    let frame_info_map = build_frame_info_map(&frame_tree.frame_tree);

    // Get iframe element info from page
    let iframe_elements = get_iframe_elements(page).await.unwrap_or_default();

    // Collect all frame IDs
    let mut frame_ids = vec![main_frame_id.clone()];
    collect_frame_ids(&frame_tree.frame_tree, &mut frame_ids);
    debug!("CDP: found {} frames", frame_ids.len());

    let mut all_nodes: Vec<SnapshotNode> = Vec::new();
    let mut global_counter = 0u32;
    let mut iframe_count = 0;
    let mut iframe_mappings: Vec<IframeMapping> = Vec::new();

    for frame_id in &frame_ids {
        let is_main = *frame_id == main_frame_id;

        // Get AX Tree for this frame
        let ax_result = page
            .execute(GetFullAxTreeParams {
                depth: None,
                frame_id: Some(frame_id.clone().into()),
            })
            .await;

        let ax_nodes = match ax_result {
            Ok(r) => r.nodes.clone(),
            Err(e) => {
                if !is_main {
                    warn!("CDP: failed to get AX tree for frame {:?}: {}", frame_id, e);
                }
                continue;
            }
        };

        if ax_nodes.is_empty() {
            continue;
        }

        debug!("CDP: frame {:?} has {} AX nodes", frame_id, ax_nodes.len());

        // Process nodes for this frame
        let (nodes, counter) =
            process_ax_nodes(page, &ax_nodes, global_counter, &snapshot_id).await?;

        if !nodes.is_empty() {
            if is_main {
                all_nodes = nodes;
            } else {
                // iframe content as special iframe node
                iframe_count += 1;
                let ref_id = format!("iframe{}", iframe_count);

                // Get frame name and URL
                let frame_info = frame_info_map.get(frame_id);
                let frame_name = frame_info.and_then(|i| i.name.clone()).unwrap_or_default();
                let frame_url = frame_info.and_then(|i| i.url.clone()).unwrap_or_default();

                // 尝试从 iframe 元素中匹配 name 和 src
                let matched_element = iframe_elements.iter().find(|el| {
                    // 优先按 name 匹配
                    if !frame_name.is_empty() && el.name.as_deref() == Some(&frame_name) {
                        return true;
                    }
                    // 其次按 src 匹配
                    if !frame_url.is_empty() && el.src.as_deref() == Some(&frame_url) {
                        return true;
                    }
                    false
                });

                let (iframe_name, iframe_src) = if let Some(el) = matched_element {
                    (el.name.clone(), el.src.clone())
                } else {
                    (Some(frame_name.clone()), Some(frame_url.clone()))
                };

                // 记录映射
                iframe_mappings.push(IframeMapping {
                    ref_id: ref_id.clone(),
                    frame_id: frame_id.clone(),
                    name: iframe_name.clone(),
                    src: iframe_src.clone(),
                });

                all_nodes.push(SnapshotNode {
                    ref_id: ref_id.clone(),
                    role: "iframe".to_string(),
                    name: iframe_name
                        .clone()
                        .unwrap_or_else(|| format!("Frame {}", frame_id)),
                    value: None,
                    description: iframe_src.clone(),
                    bounds: None,
                    attributes: {
                        let mut attrs = HashMap::new();
                        attrs.insert("frame_id".to_string(), frame_id.clone());
                        if let Some(src) = iframe_src {
                            attrs.insert("src".to_string(), src);
                        }
                        attrs
                    },
                    children: nodes,
                });
            }
            global_counter = counter;
        }
    }

    let url = page.url().await.ok().flatten().unwrap_or_default();
    let title = page.get_title().await.ok().flatten().unwrap_or_default();

    debug!(
        "CDP snapshot: {} root nodes, {} iframes, url={}",
        all_nodes.len(),
        iframe_count,
        url
    );

    Ok(PageSnapshot {
        snapshot_id,
        url,
        title,
        nodes: all_nodes,
        timestamp: now_secs(),
        iframe_count,
        iframe_mappings,
    })
}

/// Frame 信息
struct FrameInfo {
    name: Option<String>,
    url: Option<String>,
}

/// 构建 frame_id -> FrameInfo 映射
fn build_frame_info_map(
    frame_tree: &chromiumoxide::cdp::browser_protocol::page::FrameTree,
) -> HashMap<String, FrameInfo> {
    let mut map = HashMap::new();

    fn traverse(
        tree: &chromiumoxide::cdp::browser_protocol::page::FrameTree,
        map: &mut HashMap<String, FrameInfo>,
    ) {
        let frame_id: String = tree.frame.id.clone().into();
        map.insert(
            frame_id,
            FrameInfo {
                name: tree.frame.name.clone(),
                url: Some(tree.frame.url.clone()),
            },
        );

        if let Some(children) = &tree.child_frames {
            for child in children {
                traverse(child, map);
            }
        }
    }

    traverse(frame_tree, &mut map);
    map
}

/// iframe 元素信息
#[derive(Debug, Clone)]
struct IframeElement {
    name: Option<String>,
    src: Option<String>,
}

/// 从页面获取 iframe 元素信息
async fn get_iframe_elements(page: &Page) -> Result<Vec<IframeElement>> {
    let js = r#"
    Array.from(document.querySelectorAll('iframe')).map(iframe => ({
        name: iframe.name || null,
        src: iframe.src || null
    }))
    "#;

    let result: serde_json::Value = page
        .evaluate(js)
        .await
        .map_err(|e| Error::JavaScript(e.to_string()))?
        .into_value()
        .map_err(|e| Error::JavaScript(e.to_string()))?;

    let elements: Vec<IframeElement> = result
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|el| IframeElement {
                    name: el["name"].as_str().map(|s| s.to_string()),
                    src: el["src"].as_str().map(|s| s.to_string()),
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(elements)
}

/// 递归收集所有 frame ID
fn collect_frame_ids(
    frame_tree: &chromiumoxide::cdp::browser_protocol::page::FrameTree,
    frame_ids: &mut Vec<String>,
) {
    if let Some(children) = &frame_tree.child_frames {
        for child in children {
            frame_ids.push(child.frame.id.clone().into());
            collect_frame_ids(child, frame_ids);
        }
    }
}

/// 处理 AX 节点并构建树
async fn process_ax_nodes(
    page: &Page,
    ax_nodes: &[chromiumoxide::cdp::browser_protocol::accessibility::AxNode],
    start_counter: u32,
    snapshot_id: &str,
) -> Result<(Vec<SnapshotNode>, u32)> {
    process_ax_nodes_impl(page, ax_nodes, start_counter, true, snapshot_id).await
}

/// 处理 AX 节点并构建树（用于 iframe 内部快照）
///
/// 与 process_ax_nodes 相同，并将 ref_id 注入对应 frame 的 DOM 节点。
pub async fn process_ax_nodes_in_frame(
    page: &Page,
    ax_nodes: &[chromiumoxide::cdp::browser_protocol::accessibility::AxNode],
    start_counter: u32,
    snapshot_id: &str,
) -> Result<(Vec<SnapshotNode>, u32)> {
    process_ax_nodes_impl(page, ax_nodes, start_counter, true, snapshot_id).await
}

/// AX 节点处理的通用实现
///
/// 当 `inject_refs` 为 true 时，会批量注入 `data-agent-ref` 属性到 DOM 元素。
async fn process_ax_nodes_impl(
    page: &Page,
    ax_nodes: &[chromiumoxide::cdp::browser_protocol::accessibility::AxNode],
    start_counter: u32,
    inject_refs: bool,
    snapshot_id: &str,
) -> Result<(Vec<SnapshotNode>, u32)> {
    use chromiumoxide::cdp::browser_protocol::dom::{
        GetDocumentParams, PushNodesByBackendIdsToFrontendParams, SetAttributeValueParams,
    };

    // 为非忽略节点分配 ref_id
    let mut ax_id_to_ref: HashMap<String, String> = HashMap::new();
    let mut backend_ids: Vec<_> = Vec::new();
    let mut ref_ids_for_backend: Vec<String> = Vec::new();
    let mut counter = start_counter;

    for node in ax_nodes {
        if node.ignored {
            continue;
        }
        counter += 1;
        let ref_id = format!("ax{}", counter);
        ax_id_to_ref.insert(String::from(node.node_id.as_ref()), ref_id.clone());

        if inject_refs && let Some(backend_id) = node.backend_dom_node_id {
            backend_ids.push(backend_id);
            ref_ids_for_backend.push(ref_id);
        }
    }

    // 批量注入 data-agent-ref 属性
    if inject_refs && !backend_ids.is_empty() {
        page.execute(GetDocumentParams {
            depth: Some(0),
            pierce: Some(true),
        })
        .await
        .map_err(|e| Error::Cdp(e.to_string()))?;

        match page
            .execute(PushNodesByBackendIdsToFrontendParams {
                backend_node_ids: backend_ids,
            })
            .await
        {
            Ok(push_result) => {
                for (node_id, ref_id) in push_result.node_ids.iter().zip(ref_ids_for_backend.iter())
                {
                    if let Err(error) = page
                        .execute(SetAttributeValueParams {
                            node_id: *node_id,
                            name: "data-agent-ref".to_string(),
                            value: ref_id.clone(),
                        })
                        .await
                    {
                        warn!("SetAttributeValue failed for {ref_id}: {error}");
                        continue;
                    }
                    if let Err(error) = page
                        .execute(SetAttributeValueParams {
                            node_id: *node_id,
                            name: "data-agent-snapshot".to_string(),
                            value: snapshot_id.to_string(),
                        })
                        .await
                    {
                        warn!("Snapshot binding failed for {ref_id}: {error}");
                    }
                }
            }
            Err(e) => {
                warn!("PushNodesByBackendIds failed: {}", e);
            }
        }
    }

    // 构建节点映射
    let mut node_map: HashMap<String, SnapshotNode> = HashMap::new();

    for node in ax_nodes {
        if node.ignored {
            continue;
        }

        let ref_id = ax_id_to_ref
            .get(node.node_id.as_ref())
            .cloned()
            .unwrap_or_else(|| String::from(node.node_id.as_ref()));

        let role = extract_ax_str(&node.role).unwrap_or_else(|| "generic".to_string());
        let name = extract_ax_str(&node.name).unwrap_or_default();
        let description = extract_ax_str(&node.description);
        let value = extract_ax_str(&node.value);

        let mut attributes: HashMap<String, String> = HashMap::new();
        if let Some(props) = &node.properties {
            for prop in props {
                if let Some(v) = prop.value.value.as_ref() {
                    let key = prop.name.as_ref().to_string();
                    let val = match v {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    if !val.is_empty() && val != "null" && val != "false" {
                        attributes.insert(key, val);
                    }
                }
            }
        }

        node_map.insert(
            String::from(node.node_id.as_ref()),
            SnapshotNode {
                ref_id,
                role,
                name,
                value,
                description,
                bounds: None,
                attributes,
                children: Vec::new(),
            },
        );
    }

    // 构建树结构
    let roots = build_ax_tree(ax_nodes, &mut node_map);
    Ok((roots, counter))
}

/// 从 AX 节点构建树结构
fn build_ax_tree(
    ax_nodes: &[chromiumoxide::cdp::browser_protocol::accessibility::AxNode],
    node_map: &mut HashMap<String, SnapshotNode>,
) -> Vec<SnapshotNode> {
    let mut has_parent: std::collections::HashSet<String> = std::collections::HashSet::new();
    for node in ax_nodes {
        if node.ignored {
            continue;
        }
        if let Some(child_ids) = &node.child_ids {
            for cid in child_ids {
                has_parent.insert(String::from(cid.as_ref()));
            }
        }
    }

    let root_ax_ids: Vec<&str> = ax_nodes
        .iter()
        .filter(|n| !n.ignored && !has_parent.contains(n.node_id.as_ref()))
        .map(|n| n.node_id.as_ref())
        .collect();

    let child_ids_map: HashMap<&str, Vec<&str>> = ax_nodes
        .iter()
        .filter(|n| !n.ignored)
        .map(|n| {
            let cids: Vec<&str> = n
                .child_ids
                .as_deref()
                .unwrap_or(&[])
                .iter()
                .map(|id| id.as_ref())
                .collect();
            (n.node_id.as_ref(), cids)
        })
        .collect();

    fn build_subtree(
        ax_id: &str,
        node_map: &mut HashMap<String, SnapshotNode>,
        child_ids_map: &HashMap<&str, Vec<&str>>,
    ) -> Option<SnapshotNode> {
        let mut node = node_map.remove(ax_id)?;
        if let Some(cids) = child_ids_map.get(ax_id) {
            for cid in cids {
                if let Some(child) = build_subtree(cid, node_map, child_ids_map) {
                    node.children.push(child);
                }
            }
        }
        Some(node)
    }

    let mut roots: Vec<SnapshotNode> = Vec::new();
    for ax_id in root_ax_ids {
        if let Some(root) = build_subtree(ax_id, node_map, &child_ids_map) {
            roots.push(root);
        }
    }

    roots
}

/// 从 AxValue 提取字符串
fn extract_ax_str(
    v: &Option<chromiumoxide::cdp::browser_protocol::accessibility::AxValue>,
) -> Option<String> {
    v.as_ref()
        .and_then(|av| av.value.as_ref())
        .and_then(|jv| match jv {
            serde_json::Value::String(s) if !s.is_empty() => Some(s.clone()),
            _ => None,
        })
}

// ---------------------------------------------------------------------------
// JS 回退实现（支持 iframe）
// ---------------------------------------------------------------------------

/// JS 快照代码（支持 iframe）
const GET_AX_TREE_WITH_IFRAMES_JS: &str = r#"
(() => {
    const snapshotId = __AGENT_SNAPSHOT_ID__;
    let refCounter = parseInt(document.body?.getAttribute('data-agent-counter') || '0', 10);
    let iframeCounter = 0;

    function getOrAssignRefId(element) {
        const existing = element.getAttribute('data-agent-ref');
        const refId = existing || ('e' + (++refCounter));
        try {
            element.setAttribute('data-agent-ref', refId);
            element.setAttribute('data-agent-snapshot', snapshotId);
        } catch (e) {}
        return refId;
    }

    function getBounds(el) {
        try {
            const r = el.getBoundingClientRect();
            return { x: r.x, y: r.y, width: r.width, height: r.height };
        } catch (e) {
            return null;
        }
    }

    function getInputRole(el) {
        const typeRoles = {
            button: 'button', submit: 'button', reset: 'button',
            checkbox: 'checkbox', radio: 'radio', range: 'slider',
            number: 'spinbutton', search: 'searchbox', email: 'textbox',
            password: 'textbox', tel: 'textbox', text: 'textbox', url: 'textbox'
        };
        return typeRoles[(el.type || 'text').toLowerCase()] || 'textbox';
    }

    function getAriaRole(el) {
        try {
            const explicit = el.getAttribute('role');
            if (explicit) return explicit;
            const tag = el.tagName.toLowerCase();
            const tagRoles = {
                a: el.hasAttribute('href') ? 'link' : null,
                button: 'button', input: getInputRole(el), select: 'combobox',
                textarea: 'textbox', img: 'img', h1: 'heading', h2: 'heading',
                h3: 'heading', h4: 'heading', h5: 'heading', h6: 'heading',
                nav: 'navigation', main: 'main', header: 'banner',
                footer: 'contentinfo', aside: 'complementary', form: 'form',
                table: 'table', ul: 'list', ol: 'list', li: 'listitem',
                dialog: 'dialog', summary: 'button', details: 'group',
                iframe: 'iframe', frame: 'iframe'
            };
            return tagRoles[tag] || null;
        } catch (e) {
            return null;
        }
    }

    function getAccessibleName(el) {
        try {
            let n = el.getAttribute('aria-label');
            if (n) return n.trim();

            const lby = el.getAttribute('aria-labelledby');
            if (lby) {
                const lEl = document.getElementById(lby);
                if (lEl) return lEl.textContent.trim();
            }

            if (el.id) {
                const label = document.querySelector('label[for="' + el.id + '"]');
                if (label) return label.textContent.trim();
            }

            const tag = el.tagName.toLowerCase();
            if ((tag === 'input' || tag === 'textarea') && !el.getAttribute('for')) {
                const p = el.closest('label');
                if (p) {
                    const t = p.textContent.trim();
                    if (t) return t;
                }
            }

            n = el.getAttribute('title');
            if (n) return n.trim();
            n = el.getAttribute('placeholder');
            if (n) return n.trim();
            n = el.getAttribute('alt');
            if (n) return n.trim();
            n = el.getAttribute('name');
            if (n) return n.trim();

            if (['button', 'a', 'summary'].includes(tag)) {
                return el.textContent.trim().substring(0, 100);
            }

            return '';
        } catch (e) {
            return '';
        }
    }

    function isVisible(el) {
        try {
            const s = window.getComputedStyle(el);
            return s.display !== 'none' && s.visibility !== 'hidden' && s.opacity !== '0';
        } catch (e) {
            return false;
        }
    }

    function isInteractive(role, el) {
        const interactiveRoles = [
            'button', 'link', 'checkbox', 'radio', 'combobox', 'textbox', 'searchbox',
            'slider', 'spinbutton', 'menuitem', 'tab', 'option', 'iframe'
        ];
        if (interactiveRoles.includes(role)) return true;

        try {
            const tag = el.tagName.toLowerCase();
            if (['a', 'button', 'input', 'select', 'textarea', 'iframe'].includes(tag) && !el.disabled) {
                return true;
            }
            if (el.hasAttribute('onclick') || el.getAttribute('tabindex') === '0') {
                return true;
            }
        } catch (e) {}

        return false;
    }

    function processElement(el, depth = 0) {
        if (depth > 50) return null; // 防止无限递归

        if (!isVisible(el)) return null;

        const role = getAriaRole(el);
        const name = getAccessibleName(el);

        // 只保留有角色或名称的元素
        if (!role && !name) return null;

        const refId = getOrAssignRefId(el);
        const node = {
            ref_id: refId,
            role: role || 'generic',
            name: name,
            value: null,
            description: null,
            bounds: getBounds(el),
            attributes: {},
            children: []
        };

        try {
            node.attributes['tag'] = el.tagName.toLowerCase();
            if (el.value !== undefined && el.value !== null) {
                node.value = String(el.value);
            }
            if (isInteractive(role, el)) {
                node.attributes['interactive'] = 'true';
            }
            if (el.disabled) {
                node.attributes['disabled'] = 'true';
            }
            if (el.readOnly) {
                node.attributes['readonly'] = 'true';
            }
        } catch (e) {}

        return node;
    }

    function processChildren(parent, el, depth = 0) {
        if (depth > 50) return;

        try {
            for (const child of el.children) {
                const node = processElement(child, depth);
                if (node) {
                    parent.children.push(node);
                    processChildren(node, child, depth + 1);
                } else {
                    processChildren(parent, child, depth + 1);
                }
            }
        } catch (e) {}
    }

    function processIframe(iframe) {
        iframeCounter++;
        const iframeId = 'iframe' + iframeCounter;

        const node = {
            ref_id: iframeId,
            role: 'iframe',
            name: getAccessibleName(iframe) || iframe.src || 'iframe',
            value: null,
            description: null,
            bounds: getBounds(iframe),
            attributes: {
                tag: 'iframe',
                interactive: 'true'
            },
            children: []
        };

        // 尝试访问 iframe 内容（仅同源）
        try {
            const iframeDoc = iframe.contentDocument || iframe.contentWindow.document;
            if (iframeDoc && iframeDoc.body) {
                const iframeRoot = processElement(iframeDoc.body);
                if (iframeRoot) {
                    processChildren(iframeRoot, iframeDoc.body, 0);
                    node.children.push(iframeRoot);
                }
            }
        } catch (e) {
            // 跨域 iframe，无法访问内容
            node.attributes['cross_origin'] = 'true';
            if (iframe.src) {
                node.attributes['src'] = iframe.src;
            }
        }

        return node;
    }

    // 主处理逻辑
    const body = document.body;
    if (!body) {
        return { nodes: [], url: window.location.href, title: document.title, iframe_count: 0 };
    }

    const result = {
        nodes: [],
        url: window.location.href,
        title: document.title,
        iframe_count: 0
    };

    // 处理主文档
    const root = processElement(body) || {
        ref_id: getOrAssignRefId(body),
        role: 'document',
        name: document.title || '',
        value: null,
        description: null,
        bounds: getBounds(body),
        attributes: { tag: 'body' },
        children: []
    };
    processChildren(root, body, 0);
    result.nodes.push(root);

    // 处理所有 iframe
    const iframes = document.querySelectorAll('iframe');
    for (const iframe of iframes) {
        try {
            const iframeNode = processIframe(iframe);
            result.nodes.push(iframeNode);
        } catch (e) {}
    }

    result.iframe_count = iframeCounter;

    // 保存计数器
    try {
        if (document.body) {
            document.body.setAttribute('data-agent-counter', String(refCounter));
        }
    } catch (e) {}

    return result;
})()
"#;

/// JS 方式生成快照（支持 iframe）
async fn generate_snapshot_js_with_frames(page: &Page) -> Result<PageSnapshot> {
    debug!("JS fallback: generating snapshot with iframe support");
    let snapshot_id = Uuid::new_v4().to_string();
    let script = GET_AX_TREE_WITH_IFRAMES_JS.replace(
        "__AGENT_SNAPSHOT_ID__",
        &serde_json::to_string(&snapshot_id)?,
    );

    let result: serde_json::Value = page
        .evaluate(script)
        .await
        .map_err(|e| Error::JavaScript(e.to_string()))?
        .into_value()
        .map_err(|e| Error::JavaScript(e.to_string()))?;

    let url = result["url"].as_str().unwrap_or("").to_string();
    let title = result["title"].as_str().unwrap_or("").to_string();
    let iframe_count = result["iframe_count"].as_u64().unwrap_or(0) as usize;

    let nodes: Vec<SnapshotNode> = result["nodes"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|n| serde_json::from_value(n.clone()).ok())
                .collect()
        })
        .unwrap_or_default();

    info!(
        "JS snapshot: {} nodes, {} iframes",
        count_nodes(&nodes),
        iframe_count
    );

    Ok(PageSnapshot {
        snapshot_id,
        url,
        title,
        nodes,
        timestamp: now_secs(),
        iframe_count,
        iframe_mappings: Vec::new(), // JS fallback 无法获取 CDP frame_id
    })
}

/// 获取当前 Unix 时间戳（秒）
fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

// ---------------------------------------------------------------------------
// 辅助函数
// ---------------------------------------------------------------------------

/// 按 ref_id 查找节点
pub fn find_node_by_ref<'a>(nodes: &'a [SnapshotNode], ref_id: &str) -> Option<&'a SnapshotNode> {
    for node in nodes {
        if node.ref_id == ref_id {
            return Some(node);
        }
        if let Some(found) = find_node_by_ref(&node.children, ref_id) {
            return Some(found);
        }
    }
    None
}

/// 按角色查找所有节点
pub fn find_nodes_by_role<'a>(nodes: &'a [SnapshotNode], role: &str) -> Vec<&'a SnapshotNode> {
    let mut out = Vec::new();
    collect_by_role(nodes, role, &mut out);
    out
}

fn collect_by_role<'a>(nodes: &'a [SnapshotNode], role: &str, out: &mut Vec<&'a SnapshotNode>) {
    for node in nodes {
        if node.role == role {
            out.push(node);
        }
        collect_by_role(&node.children, role, out);
    }
}

/// Return a bounded snapshot suitable for an agent context window.
pub fn compact_snapshot(snapshot: &PageSnapshot, options: &SnapshotOptions) -> PageSnapshot {
    let source = options
        .root_ref
        .as_deref()
        .and_then(|ref_id| find_node_by_ref(&snapshot.nodes, ref_id))
        .map(|node| vec![node.clone()])
        .unwrap_or_else(|| snapshot.nodes.clone());
    let mut remaining = options.max_nodes.max(1);
    let nodes = compact_nodes(&source, options, 0, &mut remaining);
    PageSnapshot {
        snapshot_id: snapshot.snapshot_id.clone(),
        url: snapshot.url.clone(),
        title: snapshot.title.clone(),
        nodes,
        timestamp: snapshot.timestamp,
        iframe_count: snapshot.iframe_count,
        iframe_mappings: snapshot.iframe_mappings.clone(),
    }
}

fn compact_nodes(
    nodes: &[SnapshotNode],
    options: &SnapshotOptions,
    depth: usize,
    remaining: &mut usize,
) -> Vec<SnapshotNode> {
    if *remaining == 0 || options.max_depth.is_some_and(|max| depth > max) {
        return Vec::new();
    }
    let mut output = Vec::new();
    for node in nodes {
        if *remaining == 0 {
            break;
        }
        if options.interactive_only
            && !is_actionable_node(node)
            && !node.children.iter().any(|child| {
                subtree_has_actionable(child, options.max_depth, depth.saturating_add(1))
            })
        {
            continue;
        }
        // Reserve the ancestor before descending so a tight node budget never
        // returns orphaned descendants or an empty tree.
        *remaining -= 1;
        let mut compact = node.clone();
        compact.children = compact_nodes(&node.children, options, depth + 1, remaining);
        output.push(compact);
    }
    output
}

fn subtree_has_actionable(node: &SnapshotNode, max_depth: Option<usize>, depth: usize) -> bool {
    if max_depth.is_some_and(|max| depth > max) {
        return false;
    }
    is_actionable_node(node)
        || node
            .children
            .iter()
            .any(|child| subtree_has_actionable(child, max_depth, depth.saturating_add(1)))
}

fn is_actionable_node(node: &SnapshotNode) -> bool {
    const ACTIONABLE_ROLES: &[&str] = &[
        "button",
        "link",
        "checkbox",
        "radio",
        "combobox",
        "textbox",
        "searchbox",
        "slider",
        "spinbutton",
        "menuitem",
        "tab",
        "option",
        "switch",
        "iframe",
    ];
    ACTIONABLE_ROLES.contains(&node.role.as_str())
        || node
            .attributes
            .get("interactive")
            .is_some_and(|value| value == "true")
}

/// Search the latest accessibility snapshot without returning the whole tree.
pub fn search_snapshot(
    snapshot: &PageSnapshot,
    query: &str,
    max_results: usize,
) -> SnapshotSearchResult {
    let query = query.to_ascii_lowercase();
    let mut matches = Vec::new();
    collect_search_matches(
        &snapshot.nodes,
        &query,
        "root",
        max_results.max(1),
        &mut matches,
    );
    SnapshotSearchResult {
        snapshot_id: snapshot.snapshot_id.clone(),
        matches,
    }
}

fn collect_search_matches(
    nodes: &[SnapshotNode],
    query: &str,
    parent_path: &str,
    max_results: usize,
    output: &mut Vec<SnapshotNodeSummary>,
) {
    for (index, node) in nodes.iter().enumerate() {
        if output.len() >= max_results {
            return;
        }
        let path = format!("{parent_path}/{index}:{}", node.role);
        if node.name.to_ascii_lowercase().contains(query)
            || node.role.to_ascii_lowercase().contains(query)
            || node
                .value
                .as_deref()
                .is_some_and(|value| value.to_ascii_lowercase().contains(query))
        {
            output.push(SnapshotNodeSummary {
                ref_id: node.ref_id.clone(),
                role: node.role.clone(),
                name: node.name.clone(),
                path: path.clone(),
            });
        }
        collect_search_matches(&node.children, query, &path, max_results, output);
    }
}

/// Compute a structural delta between consecutive snapshots.
pub fn diff_snapshots(previous: &PageSnapshot, current: &PageSnapshot) -> SnapshotDiff {
    let mut old = HashMap::new();
    let mut new = HashMap::new();
    collect_diff_nodes(&previous.nodes, "root", &mut old);
    collect_diff_nodes(&current.nodes, "root", &mut new);

    let added = new
        .iter()
        .filter(|(key, _)| !old.contains_key(*key))
        .map(|(_, (summary, _))| summary.clone())
        .collect();
    let removed = old
        .iter()
        .filter(|(key, _)| !new.contains_key(*key))
        .map(|(_, (summary, _))| summary.clone())
        .collect();
    let changed = new
        .iter()
        .filter_map(|(key, (summary, fingerprint))| {
            old.get(key)
                .filter(|(_, old_fingerprint)| old_fingerprint != fingerprint)
                .map(|_| summary.clone())
        })
        .collect();

    SnapshotDiff {
        from_snapshot_id: previous.snapshot_id.clone(),
        to_snapshot_id: current.snapshot_id.clone(),
        added,
        removed,
        changed,
    }
}

fn collect_diff_nodes(
    nodes: &[SnapshotNode],
    parent_path: &str,
    output: &mut HashMap<String, (SnapshotNodeSummary, String)>,
) {
    for (index, node) in nodes.iter().enumerate() {
        let path = format!("{parent_path}/{index}:{}", node.role);
        let fingerprint =
            serde_json::to_string(&(&node.name, &node.value, &node.description, &node.attributes))
                .unwrap_or_default();
        output.insert(
            path.clone(),
            (
                SnapshotNodeSummary {
                    ref_id: node.ref_id.clone(),
                    role: node.role.clone(),
                    name: node.name.clone(),
                    path: path.clone(),
                },
                fingerprint,
            ),
        );
        collect_diff_nodes(&node.children, &path, output);
    }
}

/// 格式化快照为可读文本
pub fn format_snapshot(snapshot: &PageSnapshot) -> String {
    let mut output = String::new();
    output.push_str(&format!("快照: {}\n", snapshot.snapshot_id));
    output.push_str(&format!("页面: {}\n", snapshot.url));
    output.push_str(&format!("标题: {}\n", snapshot.title));
    if snapshot.iframe_count > 0 {
        output.push_str(&format!("iframe 数量: {}\n", snapshot.iframe_count));
    }
    output.push_str(&format!("元素数量: {}\n\n", count_nodes(&snapshot.nodes)));
    output.push_str("元素树:\n");
    format_node(&snapshot.nodes, 0, &mut output);
    output
}

fn format_node(nodes: &[SnapshotNode], depth: usize, output: &mut String) {
    for node in nodes {
        let indent = "  ".repeat(depth);
        let attrs = if node.attributes.is_empty() {
            String::new()
        } else {
            let pairs: Vec<String> = node
                .attributes
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect();
            format!(" [{}]", pairs.join(", "))
        };

        let children_count = count_nodes(&node.children);
        let children_info = if children_count > 0 {
            format!(" ({} 子元素)", children_count)
        } else {
            String::new()
        };

        output.push_str(&format!(
            "{}[{}] {} \"{}\"{}{}\n",
            indent, node.ref_id, node.role, node.name, attrs, children_info
        ));
        format_node(&node.children, depth + 1, output);
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_node(ref_id: &str, role: &str, name: &str) -> SnapshotNode {
        SnapshotNode {
            ref_id: ref_id.to_string(),
            role: role.to_string(),
            name: name.to_string(),
            value: None,
            description: None,
            bounds: None,
            attributes: std::collections::HashMap::new(),
            children: Vec::new(),
        }
    }

    fn create_test_node_with_children(
        ref_id: &str,
        role: &str,
        name: &str,
        children: Vec<SnapshotNode>,
    ) -> SnapshotNode {
        let mut node = create_test_node(ref_id, role, name);
        node.children = children;
        node
    }

    #[test]
    fn test_count_nodes_empty() {
        let nodes: Vec<SnapshotNode> = Vec::new();
        assert_eq!(count_nodes(&nodes), 0);
    }

    #[test]
    fn test_count_nodes_single() {
        let nodes = vec![create_test_node("ax1", "button", "Click me")];
        assert_eq!(count_nodes(&nodes), 1);
    }

    #[test]
    fn test_count_nodes_with_children() {
        let child1 = create_test_node("ax2", "text", "Hello");
        let child2 = create_test_node("ax3", "text", "World");
        let parent =
            create_test_node_with_children("ax1", "button", "Submit", vec![child1, child2]);

        assert_eq!(count_nodes(&[parent]), 3);
    }

    #[test]
    fn test_count_nodes_nested() {
        // Create a tree: root -> child -> grandchild
        let grandchild = create_test_node("ax3", "span", "Deep");
        let child = create_test_node_with_children("ax2", "div", "Child", vec![grandchild]);
        let root = create_test_node_with_children("ax1", "main", "Root", vec![child]);

        assert_eq!(count_nodes(&[root]), 3);
    }

    #[test]
    fn test_find_node_by_ref_found() {
        let nodes = vec![
            create_test_node("ax1", "button", "Button 1"),
            create_test_node("ax2", "button", "Button 2"),
            create_test_node("ax3", "link", "Link"),
        ];

        let found = find_node_by_ref(&nodes, "ax2");
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "Button 2");
    }

    #[test]
    fn test_find_node_by_ref_not_found() {
        let nodes = vec![create_test_node("ax1", "button", "Button")];
        let found = find_node_by_ref(&nodes, "ax999");
        assert!(found.is_none());
    }

    #[test]
    fn test_find_node_by_ref_nested() {
        let child = create_test_node("ax2", "span", "Child");
        let parent = create_test_node_with_children("ax1", "div", "Parent", vec![child]);
        let nodes = [parent];

        let found = find_node_by_ref(&nodes, "ax2");
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "Child");
    }

    #[test]
    fn test_find_nodes_by_role() {
        let nodes = vec![
            create_test_node("ax1", "button", "Button 1"),
            create_test_node("ax2", "link", "Link 1"),
            create_test_node("ax3", "button", "Button 2"),
            create_test_node("ax4", "textbox", "Input"),
        ];

        let buttons = find_nodes_by_role(&nodes, "button");
        assert_eq!(buttons.len(), 2);

        let links = find_nodes_by_role(&nodes, "link");
        assert_eq!(links.len(), 1);

        let forms = find_nodes_by_role(&nodes, "form");
        assert_eq!(forms.len(), 0);
    }

    #[test]
    fn test_find_nodes_by_role_nested() {
        let child_button = create_test_node("ax3", "button", "Child Button");
        let parent = create_test_node_with_children("ax1", "div", "Parent", vec![child_button]);
        let top_button = create_test_node("ax2", "button", "Top Button");
        let nodes = [parent, top_button];

        let buttons = find_nodes_by_role(&nodes, "button");
        assert_eq!(buttons.len(), 2);
    }

    #[test]
    fn compact_snapshot_reserves_budget_for_ancestors() {
        let button = create_test_node("ax2", "button", "Submit");
        let root = create_test_node_with_children("ax1", "main", "Page", vec![button]);
        let snapshot = PageSnapshot {
            snapshot_id: "snapshot-1".to_string(),
            url: "https://example.com".to_string(),
            title: "Example".to_string(),
            nodes: vec![root],
            timestamp: 0,
            iframe_count: 0,
            iframe_mappings: Vec::new(),
        };
        let compact = compact_snapshot(
            &snapshot,
            &SnapshotOptions {
                interactive_only: true,
                root_ref: None,
                max_depth: Some(8),
                max_nodes: 2,
            },
        );

        assert_eq!(count_nodes(&compact.nodes), 2);
        assert_eq!(compact.nodes[0].children[0].ref_id, "ax2");
    }

    #[test]
    fn test_format_snapshot_basic() {
        let nodes = vec![create_test_node("ax1", "button", "Click me")];
        let snapshot = PageSnapshot {
            snapshot_id: "test-id".to_string(),
            url: "https://example.com".to_string(),
            title: "Example".to_string(),
            nodes,
            timestamp: 1234567890,
            iframe_count: 0,
            iframe_mappings: Vec::new(),
        };

        let formatted = format_snapshot(&snapshot);
        assert!(formatted.contains("https://example.com"));
        assert!(formatted.contains("Example"));
        assert!(formatted.contains("ax1"));
        assert!(formatted.contains("button"));
        assert!(formatted.contains("Click me"));
    }

    #[test]
    fn test_format_snapshot_with_iframes() {
        let nodes = vec![create_test_node("ax1", "main", "Content")];
        let snapshot = PageSnapshot {
            snapshot_id: "test-id".to_string(),
            url: "https://example.com".to_string(),
            title: "Example".to_string(),
            nodes,
            timestamp: 1234567890,
            iframe_count: 2,
            iframe_mappings: vec![IframeMapping {
                ref_id: "iframe1".to_string(),
                frame_id: "frame-123".to_string(),
                name: Some("embed".to_string()),
                src: Some("https://other.com".to_string()),
            }],
        };

        let formatted = format_snapshot(&snapshot);
        assert!(formatted.contains("iframe 数量: 2"));
    }

    #[test]
    fn test_snapshot_node_serialization() {
        let node = create_test_node("ax1", "button", "Submit");

        let json = serde_json::to_string(&node).unwrap();
        assert!(json.contains("\"ref_id\":\"ax1\""));
        assert!(json.contains("\"role\":\"button\""));
        assert!(json.contains("\"name\":\"Submit\""));

        let parsed: SnapshotNode = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.ref_id, "ax1");
        assert_eq!(parsed.role, "button");
        assert_eq!(parsed.name, "Submit");
    }

    #[test]
    fn test_page_snapshot_serialization() {
        let snapshot = PageSnapshot {
            snapshot_id: "test-uuid".to_string(),
            url: "https://example.com".to_string(),
            title: "Example Domain".to_string(),
            nodes: vec![create_test_node("ax1", "main", "Main")],
            timestamp: 1700000000,
            iframe_count: 1,
            iframe_mappings: Vec::new(),
        };

        let json = serde_json::to_string(&snapshot).unwrap();
        let parsed: PageSnapshot = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.snapshot_id, "test-uuid");
        assert_eq!(parsed.url, "https://example.com");
        assert_eq!(parsed.iframe_count, 1);
    }

    #[test]
    fn test_iframe_mapping_serialization() {
        let mapping = IframeMapping {
            ref_id: "iframe1".to_string(),
            frame_id: "frame-abc".to_string(),
            name: Some("myframe".to_string()),
            src: Some("https://embed.com".to_string()),
        };

        let json = serde_json::to_string(&mapping).unwrap();
        let parsed: IframeMapping = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.ref_id, "iframe1");
        assert_eq!(parsed.frame_id, "frame-abc");
        assert_eq!(parsed.name, Some("myframe".to_string()));
    }
}
