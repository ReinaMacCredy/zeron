//! AX tree walker: produces the treeMarkdown string and element cache.
//!
//! Format (matching libs/cua-driver exactly):
//!   `INDENT- [N] AXRole "Title" [value="..." actions=[...]]`
//!   `INDENT- AXStaticText = "value"`  (non-indexed)
//!
//! Rules (from cua-driver reference):
//! - An element is addressable (gets an index) when it has ≥1 action name or
//!   exposes a writable AXValue control surface.
//! - Non-actionable leaf nodes with a value are rendered as `AXRole = "value"`.
//! - AXStaticText with no title/value is omitted.
//! - Tree is walked depth-first; element_index is assigned in DFS order.

use super::bindings::*;
use super::window_scope::{decide_window_scope, TopLevelCandidate, WindowScope};
use core_foundation::base::{CFEqual, CFRelease, CFRetain, CFTypeRef};
use std::sync::mpsc;

/// Default maximum depth for AX tree walks. Deep menus and complex web views
/// can nest deeply; 25 covers realistic app chrome without exploding on
/// pathological trees (mirrors Swift reference implementation).
///
/// Callers can override per-call via `walk_tree`'s `max_depth` parameter to
/// trade fidelity for context-window budget on AX-heavy apps (Electron,
/// Obsidian, large web apps — issue #22865).
pub const DEFAULT_MAX_DEPTH: usize = 25;

/// Default maximum total nodes visited during a single AX walk. Chromium-family
/// apps (Arc, VS Code, Chrome) can expose thousands of nodes; capping at 2 000
/// keeps the walk bounded while still covering realistic app chrome.
/// When the cap is hit the walk stops early and the partial tree is returned
/// with a warning line appended (mirrors Swift reference implementation).
///
/// Callers can override per-call via `walk_tree`'s `max_elements` parameter
/// (issue #22865).
pub const DEFAULT_MAX_ELEMENTS: usize = 2_000;

/// Bound each native AX request. Tokio cannot cancel a blocked
/// `AXUIElementCopyAttributeValue` after `spawn_blocking` starts, so the native
/// messaging timeout is what keeps an unresponsive app from retaining a worker
/// indefinitely.
pub(crate) const AX_MESSAGING_TIMEOUT_SECONDS: f32 = 2.0;

// Chromium clamps every off-screen node to the viewport edge, so how far below the fold a node sits is unknowable.
const BELOW_FOLD_NODES: usize = 100;

unsafe fn set_messaging_timeout(element: AXUIElementRef) {
    let _ = AXUIElementSetMessagingTimeout(element, AX_MESSAGING_TIMEOUT_SECONDS);
}

/// A single node in the AX tree.
#[derive(Debug, Clone)]
pub struct AXNode {
    /// 0-based index (Some = actionable, None = non-actionable display-only node)
    pub element_index: Option<usize>,
    pub role: String,
    /// AXTitle — shown as `"title"` in the tree line.
    pub title: Option<String>,
    /// AXValue — shown as `= "value"` in the tree line.
    pub value: Option<String>,
    /// AXDescription — shown as `(description)` in the tree line.
    /// Kept separate from `title` so `_find_calc_button("2")` can find
    /// Calculator buttons where AXTitle="" but AXDescription="2".
    pub description: Option<String>,
    pub identifier: Option<String>,
    pub help: Option<String>,
    pub actions: Vec<String>,
    /// The raw AXUIElementRef pointer value, for caching.
    pub element_ptr: usize,
    /// Depth in the rendered markdown tree (matches the indent level used in
    /// `tree_markdown`). Layout containers AXScrollArea/AXGroup collapse so
    /// children share the parent's depth.
    pub depth: usize,
    /// `element_index` of the nearest actionable ancestor, if any. Walks the
    /// rendered tree (so it skips collapsed layout containers).
    pub parent_element_index: Option<usize>,
    /// Screen-coordinate bounding rect `[x, y, width, height]` captured at
    /// walk time. `None` when AX didn't report a usable position+size.
    pub frame: Option<[f64; 4]>,
    /// AXValue coerced to a string for ALL CF types (CFNumber → "8",
    /// CFBoolean → "1"/"0", CFString as-is). Kept separate from `value`
    /// (string-only) so tree_markdown and the has_content gate — both of
    /// which read `value` — stay byte-identical; only the structured
    /// `elements` array consumes this.
    pub value_state: Option<String>,
    /// AXValueDescription — human-readable value form (e.g. "8 dB").
    pub value_description: Option<String>,
    /// AXMinValue / AXMaxValue for range controls (sliders, steppers).
    pub min_value: Option<f64>,
    pub max_value: Option<f64>,
    /// AXEnabled. `None` when the app doesn't report the attribute.
    pub enabled: Option<bool>,
    /// AXSelected. `None` when the app doesn't report the attribute.
    pub selected: Option<bool>,
    /// True when this node is an AX web-document root or descends from one.
    /// This trust marker is independent of actionable ancestry because
    /// AXWebArea is commonly non-actionable and therefore has no element index.
    pub in_web_content: bool,
    /// AXPlaceholderValue, reported separately so no consumer mistakes it for content.
    pub placeholder: Option<String>,
    pub settable: bool,
    pub expanded: Option<bool>,
    pub subrole: Option<String>,
    pub role_description: Option<String>,
    pub visible_rows: Option<VisibleRows>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisibleRows {
    pub first: usize,
    pub last: usize,
    pub total: usize,
}

#[derive(Default)]
struct ControlState {
    value_state: Option<String>,
    value_description: Option<String>,
    min_value: Option<f64>,
    max_value: Option<f64>,
    enabled: Option<bool>,
    selected: Option<bool>,
}

fn read_control_state_if_actionable<F>(is_actionable: bool, read: F) -> ControlState
where
    F: FnOnce() -> ControlState,
{
    if is_actionable {
        read()
    } else {
        ControlState::default()
    }
}

fn role_supports_value_addressing(role: &str) -> bool {
    matches!(
        role,
        "AXTextField"
            | "AXTextArea"
            | "AXComboBox"
            | "AXSlider"
            | "AXStepper"
            | "AXCheckBox"
            | "AXRadioButton"
    )
}

fn is_addressable(actions_present: bool, value_settable: bool, enabled: Option<bool>) -> bool {
    (actions_present || value_settable) && enabled != Some(false)
}

fn outside_clip([x, y, width, height]: [f64; 4], viewport: Option<[f64; 4]>) -> bool {
    let Some([vx, vy, vw, vh]) = viewport else {
        return false;
    };
    // Chromium clamps an off-screen node to the viewport edge with zero size.
    if width <= 0.0 || height <= 0.0 {
        return y >= vy + vh || y + height <= vy || x >= vx + vw || x + width <= vx;
    }
    let [cx, cy, cw, ch] = [vx - vw, vy - vh, vw * 3.0, vh * 3.0];
    x + width <= cx || y + height <= cy || x >= cx + cw || y >= cy + ch
}

fn below_viewport([x, y, _, _]: [f64; 4], viewport: Option<[f64; 4]>) -> bool {
    viewport.is_some_and(|[vx, vy, vw, vh]| y >= vy + vh && x >= vx && x < vx + vw)
}

pub struct TreeWalkResult {
    pub tree_markdown: String,
    pub nodes: Vec<AXNode>,
    /// True when the walk was cut short by the MAX_ELEMENTS cap.
    pub truncated: bool,
    /// Whether the requested `window_id` actually resolved to an AX surface,
    /// and if not, why. `None` when no `window_id` was requested.
    ///
    /// Issue #2237: without this, an unresolvable id was indistinguishable
    /// from a clean snapshot — callers had no way to tell that the tree they
    /// were handed belonged to a different surface. Any variant other than
    /// [`WindowScope::Matched`] comes with an EMPTY walk, so `nodes` never
    /// describes a window other than the requested one.
    pub window_scope: Option<WindowScope>,
}

/// Walk the AX tree of `pid`, optionally filtered to a specific window.
///
/// `window_id` — when Some, only the AXWindow matching that CGWindowID is
/// walked (plus non-window children like the menu bar). When None, all
/// top-level children are walked.
///
/// Key background-app fix: at the application root we union `AXChildren`
/// and `AXWindows`. macOS only puts windows in `AXChildren` when the app
/// is frontmost; `AXWindows` returns the window list regardless of focus
/// state. Without this union, Safari / any backgrounded app returns an
/// empty tree.
///
/// # Safety
/// Calls macOS AX API. Must be called on a thread that has a CF run loop.
pub fn walk_tree(pid: i32, window_id: Option<u32>, query: Option<&str>) -> TreeWalkResult {
    walk_tree_bounded(
        pid,
        window_id,
        query,
        DEFAULT_MAX_ELEMENTS,
        DEFAULT_MAX_DEPTH,
    )
}

/// Walk the AX tree with caller-supplied caps. See [`walk_tree`] for the
/// common case (defaults apply). `max_elements`/`max_depth` clamp the
/// rendered tree breadth-wise (DFS truncated when the element counter hits
/// the cap) and depth-wise (nodes whose markdown indent would exceed the cap
/// are omitted). Markdown and the `nodes` vec are truncated identically.
///
/// Issue #22865: caps protect against Electron / Obsidian / large web apps
/// that produce 10k+ element trees and blow context windows.
pub fn walk_tree_bounded(
    pid: i32,
    window_id: Option<u32>,
    query: Option<&str>,
    max_elements: usize,
    max_depth: usize,
) -> TreeWalkResult {
    let mut nodes: Vec<AXNode> = Vec::new();
    let mut lines: Vec<(usize, String)> = Vec::new(); // (depth, line)
    let mut index_counter = 0usize;
    // Shared visited-node counter passed into walk_element to enforce the cap.
    let mut visited_count = 0usize;
    let mut below_fold = BELOW_FOLD_NODES;
    // Set to true only when walk_element actually stops early due to the cap —
    // avoids a false-positive when the tree naturally ends on exactly the cap.
    let mut truncated = false;
    let mut window_scope: Option<WindowScope> = None;

    unsafe {
        let app_elem = AXUIElementCreateApplication(pid);
        if app_elem.is_null() {
            return TreeWalkResult {
                tree_markdown: String::new(),
                nodes,
                truncated: false,
                // No application AX element at all, so a requested window
                // certainly did not resolve.
                window_scope: window_id.map(|_| WindowScope::AxUnresolved { ax_window_count: 0 }),
            };
        }
        set_messaging_timeout(app_elem);

        // Chromium/Electron apps (Arc, VS Code, Electron shells) ship their
        // web-content AX tree OFF and only build it once an assistive client
        // asks for it. Without this, the first walk of such an app returns an
        // empty/title-bar-only tree (#1616). Flip the enablement attribute,
        // then — only when the flip actually took and only the first time we
        // see this process lifetime — let the asynchronously-built tree settle
        // before we read it. Native Cocoa apps reject the attribute, so they
        // pay no settle cost. This relies on the MAX_ELEMENTS node cap to keep
        // the now-materialized (potentially large) tree bounded.
        super::enablement::ensure_chromium_ax_enabled(pid, app_elem);

        // Union every application-level route that can retain a window proxy.
        // AXChildren omits windows when the app isn't frontmost. AXWindows may
        // omit an off-Space surface, while AXFocusedWindow or AXMainWindow can
        // still retain the exact proxy without activating or switching Spaces.
        let from_children = copy_children(app_elem);
        let from_windows = copy_ax_windows(app_elem);
        let retained_windows = [
            copy_element_attr(app_elem, "AXFocusedWindow"),
            copy_element_attr(app_elem, "AXMainWindow"),
        ];

        let mut top_level = from_children;
        for w in from_windows
            .into_iter()
            .chain(retained_windows.into_iter().flatten())
        {
            // AXChildren and AXWindows can return different proxy pointers for
            // the same native window. CFEqual compares their AX identity;
            // pointer equality alone duplicates the whole subtree and can turn
            // one exact dialog action into a false ambiguity.
            if !top_level
                .iter()
                .any(|&e| CFEqual(e as CFTypeRef, w as CFTypeRef) != 0)
            {
                top_level.push(w);
            } else {
                // Already present — release the extra application-level retain.
                CFRelease(w as CFTypeRef);
            }
        }

        // Scope: keep non-window children (menu bar) + the target window —
        // but ONLY once the target window has actually been identified. When
        // nothing claims the requested id, `decide_window_scope` reports why
        // and walks nothing; it must never fall back to "everything that isn't
        // a window", which is how issue #2237 returned menu bars as panels.
        let walk_these: Vec<AXUIElementRef> = if let Some(wid) = window_id {
            let candidates: Vec<TopLevelCandidate> = top_level
                .iter()
                .map(|&child| {
                    set_messaging_timeout(child);
                    let role = copy_string_attr(child, "AXRole").unwrap_or_default();
                    let subrole = copy_string_attr(child, "AXSubrole");
                    let identifier = copy_string_attr(child, "AXIdentifier");
                    // Match AX window element → CGWindowID via private SPI.
                    // Only windows carry one, so skip the round-trip elsewhere.
                    let ax_window_id = if role == "AXWindow" {
                        ax_get_window_id(child)
                    } else {
                        None
                    };
                    TopLevelCandidate {
                        role,
                        subrole,
                        identifier,
                        ax_window_id,
                    }
                })
                .collect();
            let decision = decide_window_scope(&candidates, wid, || {
                crate::windows::resolve_window_owner(pid, wid)
            });
            let walk: Vec<AXUIElementRef> = decision
                .walk
                .iter()
                .map(|&index| top_level[index])
                .collect();
            // A sheet can be both a top-level child and a child of the matched
            // window; walking it twice lists every dialog control twice.
            let window_children: Vec<AXUIElementRef> = walk
                .iter()
                .filter(|&&element| {
                    copy_string_attr(element, "AXRole").as_deref() == Some("AXWindow")
                })
                .flat_map(|&window| copy_children(window))
                .collect();
            let walk = walk
                .into_iter()
                .filter(|&element| {
                    copy_string_attr(element, "AXRole").as_deref() == Some("AXWindow")
                        || !window_children
                            .iter()
                            .any(|&child| CFEqual(child as CFTypeRef, element as CFTypeRef) != 0)
                })
                .collect();
            for child in window_children {
                CFRelease(child as CFTypeRef);
            }
            window_scope = Some(decision.scope);
            walk
        } else {
            top_level.to_vec()
        };

        // The fetch pool overlaps sibling elements' AX round trips. Every
        // ordering and budget decision still runs on this thread (see
        // walk_children), so the rendered tree is identical to a serial walk.
        let env = WalkEnv {
            pool: FetchPool::new(),
            focused: copy_element_attr(app_elem, "AXFocusedUIElement"),
        };

        // Walk each top-level child at depth 0. walk_children never releases
        // the refs it is handed — `walk_these` borrows `top_level`, whose
        // release loop below owns each element exactly once.
        walk_children(
            &walk_these,
            0,
            None,
            false,
            None,
            None,
            &mut nodes,
            &mut lines,
            &mut index_counter,
            &mut visited_count,
            &mut below_fold,
            &mut truncated,
            max_elements,
            max_depth,
            &env,
        );
        if let Some(focused) = env.focused {
            CFRelease(focused as CFTypeRef);
        }

        // Release all top-level elements; every application-level read retains.
        for child in top_level {
            CFRelease(child as CFTypeRef);
        }

        CFRelease(app_elem as CFTypeRef);
    }

    let truncated_flag = truncated;
    let raw_markdown = render_lines(&lines);
    let mut tree_markdown = if let Some(q) = query {
        filter_tree(&raw_markdown, q)
    } else {
        raw_markdown
    };

    if truncated_flag {
        tree_markdown.push_str(&format!(
            "\n⚠️  AX tree truncated at {max_elements} nodes \
             (app has a very large accessibility tree — Arc, Electron, or similar). \
             Element indices above are still valid. Use pixel clicks for elements \
             not visible in this partial tree."
        ));
    }

    TreeWalkResult {
        tree_markdown,
        nodes,
        truncated: truncated_flag,
        window_scope,
    }
}

// ── Parallel attribute fetch ─────────────────────────────────────────────
//
// Every AX read is one IPC round trip into the target process (~1.5 ms each).
// The AX server serializes requests per element but serves requests against
// DIFFERENT elements concurrently, so siblings' attribute fetches can overlap.
// The pool below keeps that overlap bounded; all tree ordering and budget
// decisions stay on the walk thread, which is what keeps the rendered output
// identical to the serial walk.

/// Number of concurrent AX fetch workers. Four overlaps the ~1.5 ms IPC
/// stalls without flooding the target app's AX server thread.
const FETCH_WORKERS: usize = 4;

/// Everything the assembly phase needs from one element, resolved on a pool
/// worker. `attrs.children` holds retained `AXUIElementRef`s under the same
/// ownership contract as [`copy_children`]: the caller must `CFRelease` each
/// one after use — including children whose subtree is never entered because
/// a truncation gate fires first.
struct FetchedElement {
    attrs: ElementAttrs,
    /// Resolved action names — the batch `AXActionNames` slot, or the
    /// `AXUIElementCopyActionNames` fallback for AppKit elements that don't
    /// serve it through the attribute API.
    actions: Vec<String>,
    /// `AXValue` writability probe result. Only meaningful for the
    /// `role_supports_value_addressing` family when `actions` is empty —
    /// resolved on the worker so its round trip overlaps too.
    value_settable: bool,
}

// FetchedElement is plain fetched data — decoded Strings/values plus retained
// element refs that are moved (never shared) between threads. CF reference
// counting is thread-safe and each job's element is a distinct object, so the
// same Send assertion as `RetainedElement` in ax/cache.rs applies.
unsafe impl Send for FetchedElement {}

/// One fetch request: the slot the element occupies in its siblings array
/// (results come back out of order and are re-placed by slot) plus the
/// element as a usize — the same convention the element cache uses for Send.
struct FetchJob {
    slot: usize,
    element: usize,
}

struct FetchResult {
    slot: usize,
    fetched: Option<FetchedElement>,
}

/// Perform every AX read the walk needs for one element, in the same order
/// the serial walker issued them: bound the messaging timeout, one batched
/// attribute fetch, the `AXActionNames` fallback, then the `AXValue`
/// writability probe. Runs on a pool worker, or inline on the walk thread
/// for an element that wasn't prefetched.
unsafe fn fetch_element(element: AXUIElementRef) -> FetchedElement {
    // Messaging timeouts are per AX object, not inherited from the application
    // element, so every element must be bounded before any attribute read.
    set_messaging_timeout(element);
    let mut attrs = copy_element_attrs(element);
    // Most AppKit elements do not serve AXActionNames through the attribute
    // API (the batch slot is then an error marker); fall back to the
    // dedicated AXUIElementCopyActionNames call for those.
    let actions = attrs
        .action_names
        .take()
        .unwrap_or_else(|| copy_action_names(element));
    // Some native controls expose no AX action names but do expose a writable
    // AXValue. Probe writability only for the small family of value controls —
    // the same gate the serial walk used — so display nodes pay no extra IPC.
    let role = attrs.role.as_deref().unwrap_or("AXUnknown");
    // A combobox advertises actions whether or not it accepts text.
    let value_settable = (actions.is_empty() || role == "AXComboBox")
        && role_supports_value_addressing(role)
        && is_attribute_settable(element, "AXValue");
    FetchedElement {
        attrs,
        actions,
        value_settable,
    }
}

/// Bounded pool of worker threads executing [`fetch_element`] for sibling
/// element arrays. Workers share no elements: each job is a distinct,
/// independently-retained `AXUIElementRef` owned by its slot.
struct FetchPool {
    job_txs: Vec<mpsc::Sender<FetchJob>>,
    result_rx: mpsc::Receiver<FetchResult>,
    workers: Vec<std::thread::JoinHandle<()>>,
}

impl FetchPool {
    fn new() -> Self {
        let (result_tx, result_rx) = mpsc::channel::<FetchResult>();
        let mut job_txs = Vec::new();
        let mut workers = Vec::new();
        // CUA_AX_SERIAL_FETCH disables the pool: every element falls back to
        // an inline fetch on the walk thread — the serial IPC pattern — for
        // A/B timing and as a kill switch if pooled reads misbehave.
        if std::env::var_os("CUA_AX_SERIAL_FETCH").is_none() {
            for _ in 0..FETCH_WORKERS {
                let (job_tx, job_rx) = mpsc::channel::<FetchJob>();
                let result_tx = result_tx.clone();
                workers.push(std::thread::spawn(move || {
                    while let Ok(job) = job_rx.recv() {
                        let element = job.element as AXUIElementRef;
                        // A panic in the decode path must not kill the worker:
                        // a dead worker leaves its queued jobs unanswered and
                        // the collector would wait on results that never come.
                        let fetched =
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
                                fetch_element(element)
                            }))
                            .ok();
                        if result_tx
                            .send(FetchResult {
                                slot: job.slot,
                                fetched,
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                }));
                job_txs.push(job_tx);
            }
        }
        FetchPool {
            job_txs,
            result_rx,
            workers,
        }
    }

    /// Fetch `elements` concurrently across the pool. Returns one slot per
    /// element in the same order; a `None` slot means the job never ran (its
    /// worker was never spawned, or it panicked mid-fetch) and the caller
    /// fetches that element inline instead.
    unsafe fn fetch_batch(&self, elements: &[AXUIElementRef]) -> Vec<Option<FetchedElement>> {
        let mut results: Vec<Option<FetchedElement>> = Vec::new();
        results.resize_with(elements.len(), || None);
        if self.job_txs.is_empty() {
            return results;
        }
        let worker_count = self.job_txs.len();
        let mut sent = 0usize;
        // Same-element concurrency is the one AX hazard: if a pathological
        // app returns a duplicate child pointer, its later slots stay None
        // and fetch inline on the walk thread instead.
        let mut seen = std::collections::HashSet::new();
        for (slot, &element) in elements.iter().enumerate() {
            if !seen.insert(element as usize) {
                continue;
            }
            // Round-robin keeps each worker's queue within one job of the
            // others; jobs cost the same (one element's reads), so this stays
            // balanced without a shared queue.
            if self.job_txs[slot % worker_count]
                .send(FetchJob {
                    slot,
                    element: element as usize,
                })
                .is_ok()
            {
                sent += 1;
            }
        }
        // Every sent job yields exactly one result — the worker cannot die
        // silently (panics are caught above) — so this always drains. The
        // result channel empties before the next batch is submitted.
        for _ in 0..sent {
            match self.result_rx.recv() {
                Ok(result) if result.slot < results.len() => {
                    results[result.slot] = result.fetched;
                }
                // A slot outside this batch cannot happen, but release the
                // orphaned children refs rather than leak them.
                Ok(result) => {
                    if let Some(fetched) = result.fetched {
                        for child in fetched.attrs.children {
                            CFRelease(child as CFTypeRef);
                        }
                    }
                }
                Err(_) => break,
            }
        }
        results
    }
}

impl Drop for FetchPool {
    fn drop(&mut self) {
        // Closing every job channel ends each worker's recv loop.
        self.job_txs.clear();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

// Focus is read before the walk: display-only pointers are released as it goes.
struct WalkEnv {
    pool: FetchPool,
    focused: Option<AXUIElementRef>,
}

/// Release the retained children of a prefetched element whose subtree was
/// never entered — the fetch already ran, so its `AXChildren` refs exist and
/// must not leak. Children that ARE walked release through the caller's loop
/// exactly as the serial walk did.
unsafe fn release_prefetched_children(fetched: Option<FetchedElement>) {
    if let Some(fetched) = fetched {
        for child in fetched.attrs.children {
            CFRelease(child as CFTypeRef);
        }
    }
}

/// Walk `children` serially in document order, prefetching their attribute
/// batches on the pool so the per-element AX round trips overlap.
///
/// Prefetching never changes the rendered output: every ordering and budget
/// decision still happens on the walk thread in the same sequence the serial
/// walker used. The prefetch bound mirrors the truncation gates — children
/// at `child_depth > max_depth` are all depth-blocked (the gate is uniform
/// across a siblings array), and children beyond the remaining `max_elements`
/// budget are cap-blocked because each older sibling's walk consumes at least
/// one visit. Those elements reach `walk_element` unfetched and take the same
/// early-return path the serial code takes.
#[allow(clippy::too_many_arguments)]
unsafe fn walk_children(
    children: &[AXUIElementRef],
    child_depth: usize,
    parent_index: Option<usize>,
    in_web_content: bool,
    clip: Option<[f64; 4]>,
    parent_frame: Option<[f64; 4]>,
    nodes: &mut Vec<AXNode>,
    lines: &mut Vec<(usize, String)>,
    counter: &mut usize,
    visited_count: &mut usize,
    below_fold: &mut usize,
    truncated: &mut bool,
    max_elements: usize,
    max_depth: usize,
    env: &WalkEnv,
) {
    let prefetch_count = if child_depth > max_depth {
        0
    } else {
        children
            .len()
            .min(max_elements.saturating_sub(*visited_count))
    };
    let mut prefetched: Vec<Option<FetchedElement>> = if prefetch_count == 0 {
        Vec::new()
    } else if prefetch_count == 1 {
        // A single element has nothing to overlap with — skip the pool round
        // trip and fetch on this thread.
        vec![Some(fetch_element(children[0]))]
    } else {
        env.pool.fetch_batch(&children[..prefetch_count])
    };
    for (index, &child) in children.iter().enumerate() {
        let fetched = if index < prefetch_count {
            prefetched[index].take()
        } else {
            None
        };
        walk_element(
            child,
            fetched,
            child_depth,
            parent_index,
            in_web_content,
            clip,
            parent_frame,
            nodes,
            lines,
            counter,
            visited_count,
            below_fold,
            truncated,
            max_elements,
            max_depth,
            env,
        );
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn walk_element(
    element: AXUIElementRef,
    prefetched: Option<FetchedElement>,
    depth: usize,
    parent_index: Option<usize>,
    in_web_content: bool,
    clip: Option<[f64; 4]>,
    parent_frame: Option<[f64; 4]>,
    nodes: &mut Vec<AXNode>,
    lines: &mut Vec<(usize, String)>,
    counter: &mut usize,
    visited_count: &mut usize,
    below_fold: &mut usize,
    truncated: &mut bool,
    max_elements: usize,
    max_depth: usize,
    env: &WalkEnv,
) {
    if depth > max_depth {
        release_prefetched_children(prefetched);
        return;
    }
    // Enforce total-node cap — mirrors Swift's maxElements guard.
    // Set the truncated flag only when we actually stop early.
    if *visited_count >= max_elements {
        *truncated = true;
        release_prefetched_children(prefetched);
        return;
    }

    // Elements the prefetch bound skipped — and any whose pooled fetch
    // failed — fetch inline here. When the pool is disabled this is the
    // serial path for every element.
    let FetchedElement {
        attrs,
        actions,
        value_settable,
    } = prefetched.unwrap_or_else(|| fetch_element(element));

    // One IPC call fetched every attribute this walk needs. Per-attribute
    // failures arrive as error markers in the batch slots and decode to
    // None/empty — the same results the individual copy_* helpers produce
    // on a failed fetch.
    let ElementAttrs {
        role,
        children,
        title,
        value: copied_value,
        placeholder_value,
        description,
        identifier,
        help,
        enabled: attr_enabled,
        value_description: attr_value_description,
        min_value: attr_min_value,
        max_value: attr_max_value,
        selected: attr_selected,
        frame,
        bounds,
        expanded,
        subrole,
        role_description,
        ..
    } = attrs;

    let role = role.unwrap_or_else(|| "AXUnknown".into());
    let mut children = children;
    if role == "AXMenuBarItem" && attr_selected != Some(true) {
        for child in children.drain(..) {
            CFRelease(child as CFTypeRef);
        }
    }
    let visible_rows = if !in_web_content && matches!(role.as_str(), "AXTable" | "AXOutline") {
        walk_visible_rows(element, &mut children)
    } else {
        None
    };

    if in_web_content && bounds.is_some_and(|bounds| outside_clip(bounds, clip)) {
        if *below_fold > 0 && bounds.is_some_and(|bounds| below_viewport(bounds, clip)) {
            *below_fold -= 1;
        } else {
            for child in children {
                CFRelease(child as CFTypeRef);
            }
            return;
        }
    }
    *visited_count += 1;
    // react-select draws its read-only input at scale(.01), leaving no usable frame.
    let frame = if in_web_content {
        frame.or(parent_frame)
    } else {
        frame
    };

    let web_root = !in_web_content && is_web_content_role(&role);
    let in_web_content = in_web_content || web_root;
    let clip = if web_root { clip.or(frame) } else { clip };

    // Skip pure layout containers that have no interesting content.
    if role == "AXScrollArea" || role == "AXGroup" {
        // Still recurse — children may be interesting. Layout containers
        // collapse, so children inherit the parent's depth AND the same
        // parent_index (no actionable node was emitted here).
        walk_children(
            &children,
            depth,
            parent_index,
            in_web_content,
            clip,
            frame,
            nodes,
            lines,
            counter,
            visited_count,
            below_fold,
            truncated,
            max_elements,
            max_depth,
            env,
        );
        for child in children {
            CFRelease(child as CFTypeRef);
        }
        return;
    }

    // Keep AXTitle and AXDescription SEPARATE so that the tree format matches
    // the Swift reference: title → "title", description → (description).
    // This is critical for Calculator where AXTitle="" but AXDescription="2"
    // (digit buttons). Merging them would produce "2" (quoted) instead of (2)
    // (parens), breaking _find_calc_button which searches for "(2)".
    //
    // `copied_value` carries enough type information to preserve the
    // string-only markdown while also exposing numeric/boolean control state.
    let value = copied_value
        .as_ref()
        .and_then(|copied| copied.string_value.clone());
    // AXPlaceholderValue as fallback for empty text fields.
    let placeholder = placeholder_value
        .as_ref()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty());
    let value = value.filter(|v| !v.trim().is_empty()).or(placeholder_value);
    let help = help.filter(|h| !h.trim().is_empty());
    // `actions` and `value_settable` were resolved alongside the batch fetch
    // (on a pool worker for prefetched elements) — including the
    // AXUIElementCopyActionNames fallback and the AXValue writability probe
    // for the value-control family (Finder's transient inline-rename field is
    // the case that needs an index without any action names).

    let visible_title = title.as_deref().unwrap_or("").trim().to_owned();
    let visible_description = description.as_deref().unwrap_or("").trim().to_owned();
    let visible_value = value.as_deref().unwrap_or("").trim().to_owned();

    let has_content =
        !visible_title.is_empty() || !visible_description.is_empty() || !visible_value.is_empty();
    // A closed submenu can keep its descendants in AXChildren while reporting
    // those controls disabled. Never assign such a row a live element index:
    // the same native state also causes dispatch to refuse it, and exposing an
    // index for it invites agents to retain an unusable menu target.
    let enabled = if !actions.is_empty() || value_settable {
        attr_enabled
    } else {
        None
    };
    // ARIA options and custom selects act on pointer events their AX node does not advertise.
    let focused = env
        .focused
        .is_some_and(|focused| CFEqual(focused as CFTypeRef, element as CFTypeRef) != 0);
    let is_actionable = is_addressable(!actions.is_empty(), value_settable, enabled)
        || (in_web_content && has_content && attr_enabled != Some(false))
        || focused;

    if !is_actionable && !has_content && role != "AXWindow" && role != "AXSheet" {
        walk_children(
            &children,
            depth + 1,
            parent_index,
            in_web_content,
            clip,
            frame,
            nodes,
            lines,
            counter,
            visited_count,
            below_fold,
            truncated,
            max_elements,
            max_depth,
            env,
        );
        for child in children {
            CFRelease(child as CFTypeRef);
        }
        return;
    }

    let element_ptr = element as usize;
    // Structured `elements` only contains actionable nodes. Keep all new AX
    // round-trips behind that same gate so display-only rows pay no cost.
    let control_state = read_control_state_if_actionable(is_actionable, || ControlState {
        value_state: copied_value.map(|copied| copied.state_value),
        value_description: attr_value_description
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty()),
        min_value: attr_min_value,
        max_value: attr_max_value,
        enabled,
        selected: attr_selected,
    });
    let node = if is_actionable {
        let idx = *counter;
        *counter += 1;
        // Retain so the element stays alive in the cache after `copy_children`
        // releases the per-child ref at the end of the caller's loop.
        CFRetain(element as CFTypeRef);
        AXNode {
            element_index: Some(idx),
            role: role.clone(),
            title: if visible_title.is_empty() {
                None
            } else {
                Some(visible_title.clone())
            },
            value: if visible_value.is_empty() {
                None
            } else {
                Some(visible_value.clone())
            },
            description: if visible_description.is_empty() {
                None
            } else {
                Some(visible_description.clone())
            },
            identifier: identifier.clone(),
            help: help.clone(),
            actions: actions.clone(),
            element_ptr,
            depth,
            parent_element_index: parent_index,
            frame,
            value_state: control_state.value_state.clone(),
            value_description: control_state.value_description.clone(),
            min_value: control_state.min_value,
            max_value: control_state.max_value,
            enabled: control_state.enabled,
            selected: control_state.selected,
            in_web_content,
            placeholder: placeholder.clone(),
            settable: value_settable,
            expanded,
            subrole,
            role_description,
            visible_rows,
        }
    } else {
        AXNode {
            element_index: None,
            role: role.clone(),
            title: if visible_title.is_empty() {
                None
            } else {
                Some(visible_title.clone())
            },
            value: if visible_value.is_empty() {
                None
            } else {
                Some(visible_value.clone())
            },
            description: if visible_description.is_empty() {
                None
            } else {
                Some(visible_description.clone())
            },
            identifier: identifier.clone(),
            help: help.clone(),
            actions: vec![],
            element_ptr,
            depth,
            parent_element_index: parent_index,
            frame,
            value_state: control_state.value_state.clone(),
            value_description: control_state.value_description.clone(),
            min_value: control_state.min_value,
            max_value: control_state.max_value,
            enabled: attr_enabled,
            selected: attr_selected,
            in_web_content,
            placeholder: placeholder.clone(),
            settable: value_settable,
            expanded,
            subrole,
            role_description,
            visible_rows,
        }
    };

    // Track this node as the parent for its descendants only when it was
    // assigned an element_index (mirrors what the markdown shows: only
    // indexed rows are addressable in click(element_index=N)).
    let next_parent = node.element_index.or(parent_index);

    let line = format_node_line(&node);
    lines.push((depth, line));
    nodes.push(node);

    walk_children(
        &children,
        depth + 1,
        next_parent,
        in_web_content,
        clip,
        frame,
        nodes,
        lines,
        counter,
        visited_count,
        below_fold,
        truncated,
        max_elements,
        max_depth,
        env,
    );
    for child in children {
        CFRelease(child as CFTypeRef);
    }
}

// AppKit builds a cell view for every row whose cells are read, so off-screen rows are never walked.
unsafe fn walk_visible_rows(
    table: AXUIElementRef,
    children: &mut Vec<AXUIElementRef>,
) -> Option<VisibleRows> {
    attribute_count(table, "AXVisibleRows")?;
    let total = attribute_count(table, "AXRows").unwrap_or(0);
    let rows = copy_element_array(table, "AXVisibleRows");
    for child in children.drain(..) {
        CFRelease(child as CFTypeRef);
    }
    children.extend(copy_element_attr(table, "AXHeader"));
    let index = |row: AXUIElementRef| {
        set_messaging_timeout(row);
        copy_number_attr(row, "AXIndex").map(|index| index.max(0.0) as usize)
    };
    let range = match (rows.first(), rows.last()) {
        (Some(&first), Some(&last)) => index(first)
            .zip(index(last))
            .map(|(first, last)| VisibleRows { first, last, total }),
        _ => None,
    };
    children.extend(rows);
    range
}

#[derive(Debug, Clone, Default)]
pub struct FocusedElement {
    pub role: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub value: Option<String>,
    pub identifier: Option<String>,
    pub settable: bool,
    pub element_index: Option<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct WindowFocus {
    pub focused: Option<FocusedElement>,
    pub selected_text: Option<String>,
    pub url: Option<String>,
}

// Only indexed nodes are compared: display-only element pointers are released after the walk.
pub fn read_window_focus(pid: i32, window_id: u32, nodes: &mut Vec<AXNode>) -> WindowFocus {
    let mut focus = WindowFocus::default();
    unsafe {
        let app = AXUIElementCreateApplication(pid);
        if app.is_null() {
            return focus;
        }
        set_messaging_timeout(app);
        let present = |text: Option<String>| {
            text.map(|text| text.trim().to_owned())
                .filter(|text| !text.is_empty())
        };
        if let Some(element) = copy_element_attr(app, "AXFocusedUIElement") {
            set_messaging_timeout(element);
            let mut retained = false;
            if super::exact_target::element_window_id(element) == Some(window_id) {
                let mut element_index = nodes
                    .iter()
                    .filter(|node| node.element_index.is_some())
                    .find(|node| CFEqual(node.element_ptr as CFTypeRef, element as CFTypeRef) != 0)
                    .and_then(|node| node.element_index);
                let settable = is_attribute_settable(element, "AXValue");
                if element_index.is_none() {
                    let index = nodes
                        .iter()
                        .filter_map(|node| node.element_index)
                        .max()
                        .map_or(0, |index| index + 1);
                    nodes.push(focused_node(element, index, settable));
                    element_index = Some(index);
                    retained = true;
                }
                focus.focused = Some(FocusedElement {
                    role: copy_string_attr(element, "AXRole").unwrap_or_else(|| "AXUnknown".into()),
                    title: present(copy_string_attr(element, "AXTitle")),
                    description: present(copy_string_attr(element, "AXDescription")),
                    value: present(copy_string_attr(element, "AXValue")),
                    identifier: present(copy_string_attr(element, "AXIdentifier")),
                    settable,
                    element_index,
                });
                focus.selected_text =
                    copy_string_attr(element, "AXSelectedText").filter(|text| !text.is_empty());
            }
            if !retained {
                CFRelease(element as CFTypeRef);
            }
        }
        for window in copy_ax_windows(app) {
            if focus.url.is_none() && ax_get_window_id(window) == Some(window_id) {
                set_messaging_timeout(window);
                focus.url = present(copy_string_attr(window, "AXDocument"));
            }
            CFRelease(window as CFTypeRef);
        }
        CFRelease(app as CFTypeRef);
    }
    focus
}

/// Takes ownership of the caller's +1 retain on `element`.
unsafe fn focused_node(element: AXUIElementRef, index: usize, settable: bool) -> AXNode {
    let FetchedElement { attrs, actions, .. } = fetch_element(element);
    for child in attrs.children {
        CFRelease(child as CFTypeRef);
    }
    let text = |value: Option<String>| {
        value
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    };
    let placeholder = text(attrs.placeholder_value);
    let value = text(attrs.value.and_then(|copied| copied.string_value)).or(placeholder.clone());
    AXNode {
        element_index: Some(index),
        role: attrs.role.unwrap_or_else(|| "AXUnknown".into()),
        title: text(attrs.title),
        value,
        description: text(attrs.description),
        identifier: attrs.identifier,
        help: text(attrs.help),
        actions,
        element_ptr: element as usize,
        depth: 0,
        parent_element_index: None,
        frame: attrs.frame,
        value_state: None,
        value_description: None,
        min_value: None,
        max_value: None,
        enabled: attrs.enabled,
        selected: attrs.selected,
        in_web_content: has_web_area_ancestor(element),
        placeholder,
        settable,
        expanded: attrs.expanded,
        subrole: attrs.subrole,
        role_description: attrs.role_description,
        visible_rows: None,
    }
}

/// A contenteditable editor in a web page. Chromium writes AXValue and
/// AXSelectedText into one without firing beforeinput or input, so the page's
/// editor never sees the text; only key events reach it.
pub(crate) unsafe fn is_web_rich_editor(element: AXUIElementRef) -> bool {
    let role = copy_string_attr(element, "AXRole").unwrap_or_default();
    if !matches!(
        role.as_str(),
        "AXTextField" | "AXTextArea" | "AXSearchField"
    ) || !has_web_area_ancestor(element)
    {
        return false;
    }
    let children = copy_children(element);
    let has_children = !children.is_empty();
    for child in children {
        CFRelease(child as CFTypeRef);
    }
    // Chromium lists AXInvalid for every <input> and <textarea>, for a contenteditable only with
    // aria-invalid; reading it answers on both.
    let names = copy_attribute_names(element);
    has_children || (!names.is_empty() && !names.iter().any(|name| name == "AXInvalid"))
}

pub(crate) unsafe fn has_web_area_ancestor(element: AXUIElementRef) -> bool {
    let mut current = copy_element_attr(element, "AXParent");
    let mut found = false;
    for _ in 0..64 {
        let Some(node) = current else {
            break;
        };
        set_messaging_timeout(node);
        found = copy_string_attr(node, "AXRole").is_some_and(|role| is_web_content_role(&role));
        current = if found {
            None
        } else {
            copy_element_attr(node, "AXParent")
        };
        CFRelease(node as CFTypeRef);
    }
    if let Some(node) = current {
        CFRelease(node as CFTypeRef);
    }
    found
}

fn is_web_content_role(role: &str) -> bool {
    let normalized = role
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    normalized.contains("webarea") || normalized.contains("documentweb") || normalized == "document"
}

#[cfg(test)]
mod web_content_role_tests {
    use super::is_web_content_role;

    #[test]
    fn recognizes_native_web_document_roles_without_marking_app_chrome() {
        for role in ["AXWebArea", "AXDocumentWeb", "document"] {
            assert!(is_web_content_role(role), "{role} must start web trust");
        }
        for role in ["AXWindow", "AXButton", "AXToolbar"] {
            assert!(!is_web_content_role(role), "{role} stays native");
        }
    }
}

fn format_node_line(node: &AXNode) -> String {
    let mut parts = String::new();

    // Common prefix (with or without index).
    if let Some(idx) = node.element_index {
        parts.push_str(&format!("- [{}] {}", idx, node.role));
    } else {
        parts.push_str(&format!("- {}", node.role));
    }

    // AXTitle → "title"
    if let Some(t) = &node.title {
        parts.push_str(&format!(" \"{}\"", t));
    }
    // AXValue → = "value"
    if let Some(v) = &node.value {
        parts.push_str(&format!(" = \"{}\"", v));
    }
    // AXDescription → (description) — critical for Calculator digit buttons
    // where AXTitle="" but AXDescription="2".
    if let Some(d) = &node.description {
        parts.push_str(&format!(" ({})", d));
    }

    // Bracketed metadata block (identifier, help, actions).
    if node.element_index.is_some() {
        let mut attrs: Vec<String> = Vec::new();
        if let Some(id) = &node.identifier {
            attrs.push(format!("id={}", id));
        }
        if let Some(h) = &node.help {
            attrs.push(format!("help=\"{}\"", h));
        }
        if !node.actions.is_empty() {
            let action_str = node
                .actions
                .iter()
                .map(|a| a.strip_prefix("AX").unwrap_or(a).to_lowercase())
                .collect::<Vec<_>>()
                .join(",");
            attrs.push(format!("actions=[{}]", action_str));
        }
        if !attrs.is_empty() {
            parts.push_str(" [");
            parts.push_str(&attrs.join(" "));
            parts.push(']');
        }
    }

    parts
}

fn render_lines(lines: &[(usize, String)]) -> String {
    let mut out = String::new();
    for (depth, line) in lines {
        for _ in 0..*depth {
            out.push_str("  ");
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Filter the tree markdown to lines matching `query` plus their ancestor chain.
fn filter_tree(markdown: &str, query: &str) -> String {
    let needle = query.to_lowercase();
    let lines: Vec<&str> = markdown.lines().collect();

    let mut current_ancestor: Vec<&str> = Vec::new();
    let mut last_emitted_at: Vec<Option<&str>> = Vec::new();
    let mut output: Vec<&str> = Vec::new();

    for line in &lines {
        let depth = leading_indent_depth(line);

        while current_ancestor.len() <= depth {
            current_ancestor.push("");
            last_emitted_at.push(None);
        }
        for emitted_at in last_emitted_at.iter_mut().skip(depth + 1) {
            *emitted_at = None;
        }
        current_ancestor[depth] = line;

        if line.to_lowercase().contains(&needle) {
            for ancestor_depth in 0..depth {
                let ancestor = current_ancestor[ancestor_depth];
                if ancestor.is_empty() {
                    continue;
                }
                if last_emitted_at[ancestor_depth] == Some(ancestor) {
                    continue;
                }
                last_emitted_at[ancestor_depth] = Some(ancestor);
                output.push(ancestor);
            }
            last_emitted_at[depth] = Some(line);
            output.push(line);
        }
    }

    if output.is_empty() {
        return String::new();
    }
    let mut result = output.join("\n");
    result.push('\n');
    result
}

fn leading_indent_depth(line: &str) -> usize {
    let mut count = 0;
    for ch in line.chars() {
        if ch == ' ' {
            count += 1;
        } else {
            break;
        }
    }
    count / 2
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn writable_value_controls_are_addressable_without_actions() {
        assert!(is_addressable(false, true, Some(true)));
        assert!(is_addressable(true, false, None));
        assert!(!is_addressable(false, false, Some(true)));
        assert!(!is_addressable(true, false, Some(false)));

        for role in [
            "AXTextField",
            "AXTextArea",
            "AXComboBox",
            "AXSlider",
            "AXStepper",
            "AXCheckBox",
            "AXRadioButton",
        ] {
            assert!(role_supports_value_addressing(role), "{role}");
        }
        for role in ["AXStaticText", "AXImage", "AXWindow", "AXGroup"] {
            assert!(!role_supports_value_addressing(role), "{role}");
        }
    }

    #[test]
    fn control_state_reads_are_gated_by_actionability() {
        let reads = Cell::new(0);
        let display_only = read_control_state_if_actionable(false, || {
            reads.set(reads.get() + 1);
            ControlState {
                enabled: Some(true),
                ..ControlState::default()
            }
        });
        assert_eq!(reads.get(), 0, "display-only nodes must not read state");
        assert_eq!(display_only.enabled, None);

        let actionable = read_control_state_if_actionable(true, || {
            reads.set(reads.get() + 1);
            ControlState {
                enabled: Some(true),
                ..ControlState::default()
            }
        });
        assert_eq!(reads.get(), 1, "actionable nodes must read state once");
        assert_eq!(actionable.enabled, Some(true));
    }

    /// Every AXNode field that must agree between serial and parallel walks.
    /// `element_ptr` is excluded on purpose: the AX server may hand back
    /// different proxy pointers across calls, so even two serial walks can
    /// legitimately differ there. `frame` is compared separately — live apps
    /// legitimately drift a few pixels between two sequential walks.
    fn node_fingerprint(node: &AXNode) -> String {
        format!(
            "{:?}|{}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{}",
            node.element_index,
            node.role,
            node.title,
            node.value,
            node.description,
            node.identifier,
            node.help,
            node.actions,
            node.depth,
            node.parent_element_index,
            node.value_state,
            node.value_description,
            node.min_value,
            node.max_value,
            node.enabled,
            node.selected,
            node.in_web_content,
        )
    }

    /// Frames agree when both sides report the same rect within a small
    /// pixel tolerance (or both report nothing). 20 px covers the settle
    /// drift observed on live apps without masking a real layout change.
    fn frames_agree(a: Option<[f64; 4]>, b: Option<[f64; 4]>) -> bool {
        match (a, b) {
            (None, None) => true,
            (Some(a), Some(b)) => a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() <= 20.0),
            _ => false,
        }
    }

    /// A/B harness: walks the same app serially (pool disabled via
    /// CUA_AX_SERIAL_FETCH) and in parallel, asserts byte-identical output,
    /// and prints both wall-clock times. Needs a live target — pass its pid
    /// in CUA_AX_TEST_PID — and an AX-trusted test process, so it is ignored
    /// by default:
    ///
    ///   CUA_AX_TEST_PID=$(pgrep -x Calculator | head -1) \
    ///     cargo test -p platform-macos --lib ax::tree::tests -- --ignored --nocapture
    ///
    /// Run it alone: the env toggle is process-global.
    #[test]
    #[ignore]
    fn parallel_prefetch_matches_serial_output_and_times_it() {
        let pid: i32 = std::env::var("CUA_AX_TEST_PID")
            .ok()
            .and_then(|v| v.parse().ok())
            .expect("CUA_AX_TEST_PID must name a live pid");

        std::env::set_var("CUA_AX_SERIAL_FETCH", "1");
        let start = std::time::Instant::now();
        let serial = walk_tree(pid, None, None);
        let serial_ms = start.elapsed().as_secs_f64() * 1e3;

        std::env::remove_var("CUA_AX_SERIAL_FETCH");
        let start = std::time::Instant::now();
        let parallel = walk_tree(pid, None, None);
        let parallel_ms = start.elapsed().as_secs_f64() * 1e3;

        eprintln!(
            "pid={pid} serial={serial_ms:.1}ms parallel={parallel_ms:.1}ms \
             nodes serial={} parallel={}",
            serial.nodes.len(),
            parallel.nodes.len()
        );

        assert_eq!(
            serial.tree_markdown, parallel.tree_markdown,
            "tree_markdown differs between serial and parallel walks"
        );
        assert_eq!(serial.truncated, parallel.truncated);
        assert_eq!(serial.nodes.len(), parallel.nodes.len());
        for (i, (s, p)) in serial.nodes.iter().zip(&parallel.nodes).enumerate() {
            assert_eq!(
                node_fingerprint(s),
                node_fingerprint(p),
                "node {i} differs between serial and parallel walks"
            );
            assert!(
                frames_agree(s.frame, p.frame),
                "node {i} frame differs: {:?} vs {:?}",
                s.frame,
                p.frame
            );
        }

        // Truncation gates must cut identically under prefetch: a tight
        // element cap exercises the budget bound in walk_children, a tight
        // depth cap exercises the depth bound.
        for (max_elements, max_depth) in [(5usize, 25usize), (2_000usize, 1usize)] {
            std::env::set_var("CUA_AX_SERIAL_FETCH", "1");
            let s = walk_tree_bounded(pid, None, None, max_elements, max_depth);
            std::env::remove_var("CUA_AX_SERIAL_FETCH");
            let p = walk_tree_bounded(pid, None, None, max_elements, max_depth);
            assert_eq!(
                s.tree_markdown, p.tree_markdown,
                "bounded walk (max_elements={max_elements}, max_depth={max_depth}) differs"
            );
            assert_eq!(s.truncated, p.truncated);
            assert_eq!(s.nodes.len(), p.nodes.len());
        }
    }
}
