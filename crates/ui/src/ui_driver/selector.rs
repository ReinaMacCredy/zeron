use gpui::accesskit::Role;
use gpui::{AnyWindowHandle, AsyncApp};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

use super::tree::{Node, Tree, contains, line, read};

const POLL: Duration = Duration::from_millis(100);
const LISTED: usize = 10;
const NEAR: usize = 5;

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Selector {
    Ref(String),
    Match {
        role: Option<String>,
        text: Option<String>,
    },
}

pub(super) fn role_name(word: &str) -> Option<String> {
    (0..=u8::MAX)
        .filter_map(Role::n)
        .map(|role| format!("{role:?}"))
        .find(|name| name.eq_ignore_ascii_case(word))
}

impl Selector {
    pub(super) fn parse(raw: &str) -> Result<Self, String> {
        let bad = |why: &str| {
            format!(
                "bad selector `{raw}`: {why}; expected `#e12`, `#w1-3`, `Role`, `\"text\"` or `Role \"text\"`"
            )
        };
        let raw = raw.trim();
        if let Some(id) = raw.strip_prefix('#') {
            let digits = |n: &str| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit());
            let native = id.strip_prefix('e').is_some_and(digits);
            let web = id
                .strip_prefix('w')
                .and_then(|rest| rest.split_once('-'))
                .is_some_and(|(view, n)| digits(view) && digits(n));
            return if native || web {
                Ok(Selector::Ref(id.to_owned()))
            } else {
                Err(bad(
                    "a ref is `#e` and a number, or a web ref such as `#w1-3`",
                ))
            };
        }
        let (word, rest) = raw.split_at(raw.find('"').unwrap_or(raw.len()));
        let word = word.trim();
        let role = match word {
            "" => None,
            word if word.chars().all(|c| c.is_ascii_alphabetic()) => Some(
                role_name(word)
                    .ok_or_else(|| bad(&format!("`{word}` is not an AccessKit role")))?,
            ),
            _ => return Err(bad("the role is one word, the text goes in double quotes")),
        };
        let text = match rest {
            "" => None,
            rest => {
                let inner = rest
                    .strip_prefix('"')
                    .and_then(|rest| rest.strip_suffix('"'))
                    .filter(|_| rest.len() >= 2)
                    .ok_or_else(|| bad("the text must be one double-quoted string at the end"))?;
                let text = inner.replace("\\\"", "\"").replace("\\\\", "\\");
                if text.is_empty() {
                    return Err(bad("the quoted text is empty"));
                }
                Some(text)
            }
        };
        if role.is_none() && text.is_none() {
            return Err(bad("it is empty"));
        }
        Ok(Selector::Match { role, text })
    }

    pub(super) fn hits(&self, nodes: &[Node]) -> Vec<usize> {
        let (role, text) = match self {
            Selector::Ref(id) => {
                return nodes
                    .iter()
                    .position(|node| &node.id == id)
                    .into_iter()
                    .collect();
            }
            Selector::Match { role, text } => (role, text),
        };
        let candidates: Vec<usize> = (0..nodes.len())
            .filter(|&i| role.as_ref().is_none_or(|role| &nodes[i].role == role))
            .collect();
        let Some(text) = text else {
            return candidates;
        };
        let text = text.to_lowercase();
        let exact = |field: &Option<String>| {
            field
                .as_deref()
                .is_some_and(|field| field.to_lowercase() == text)
        };
        let tiers: [&dyn Fn(&Node) -> bool; 4] = [
            &|node| exact(&node.label),
            &|node| contains(&node.label, &text),
            &|node| contains(&node.value, &text),
            &|node| contains(&node.placeholder, &text) || contains(&node.description, &text),
        ];
        let mut hits = tiers
            .iter()
            .map(|tier| {
                candidates
                    .iter()
                    .copied()
                    .filter(|&i| tier(&nodes[i]))
                    .collect::<Vec<_>>()
            })
            .find(|hits| !hits.is_empty())
            .unwrap_or_default();
        // Page text is the page's to choose, so it never shadows the app's own control.
        if hits.iter().any(|&i| !nodes[i].web) {
            hits.retain(|&i| !nodes[i].web);
        }
        hits
    }

    fn near(&self, nodes: &[Node]) -> Vec<usize> {
        let Selector::Match { role, text } = self else {
            return Vec::new();
        };
        let words: Vec<String> = text
            .iter()
            .flat_map(|text| text.split_whitespace())
            .filter(|word| word.chars().count() >= 2)
            .map(str::to_lowercase)
            .collect();
        (0..nodes.len())
            .filter(|&i| {
                let node = &nodes[i];
                role.as_ref().is_some_and(|role| &node.role == role)
                    || node.label.as_deref().is_some_and(|label| {
                        let label = label.to_lowercase();
                        words.iter().any(|word| label.contains(word))
                    })
            })
            .take(NEAR)
            .collect()
    }
}

fn listing(nodes: &[Node], hits: &[usize]) -> String {
    let mut out: String = hits
        .iter()
        .take(LISTED)
        .map(|&i| format!("\n  {}", line(&nodes[i])))
        .collect();
    if hits.len() > LISTED {
        out += &format!("\n  … and {} more", hits.len() - LISTED);
    }
    out
}

pub(super) fn resolve(
    nodes: &[Node],
    raw: &str,
    selector: &Selector,
    nth: Option<usize>,
) -> Result<usize, String> {
    let hits = selector.hits(nodes);
    match (hits.len(), nth) {
        (0, _) if matches!(selector, Selector::Ref(_)) => Err(format!(
            "`{raw}` is not in the current tree (gone); read `tree` again"
        )),
        (0, _) => {
            let near = selector.near(nodes);
            let tail = if near.is_empty() {
                String::new()
            } else {
                format!("; near:{}", listing(nodes, &near))
            };
            Err(format!("no element matches `{raw}`{tail}"))
        }
        (_, Some(n)) => hits.get(n).copied().ok_or_else(|| {
            format!(
                "`{raw}` nth {n} is out of range, {} matches:{}",
                hits.len(),
                listing(nodes, &hits)
            )
        }),
        (1, None) => Ok(hits[0]),
        (count, None) => Err(format!(
            "`{raw}` matches {count} elements; pass `nth` (0-based) or a narrower selector:{}",
            listing(nodes, &hits)
        )),
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(
    untagged,
    expecting = "an element: \"ref\" (e12) or \"select\" (Role \"text\") with optional \"nth\""
)]
pub(super) enum Element {
    Ref {
        #[serde(rename = "ref")]
        id: String,
    },
    Select {
        select: String,
        #[serde(default)]
        nth: Option<usize>,
    },
}

impl Element {
    pub(super) fn selector(&self) -> Result<(Selector, Option<usize>, String), String> {
        match self {
            Element::Ref { id } => {
                let raw = format!("#{}", id.strip_prefix('#').unwrap_or(id));
                Ok((Selector::parse(&raw)?, None, raw))
            }
            Element::Select { select, nth } => Ok((Selector::parse(select)?, *nth, select.clone())),
        }
    }

    pub(super) fn find<'a>(&self, tree: &'a Tree) -> Result<&'a Node, String> {
        let (selector, nth, raw) = self.selector()?;
        resolve(&tree.nodes, &raw, &selector, nth).map(|i| &tree.nodes[i])
    }

    pub(super) fn find_or_absent<'a>(&self, tree: &'a Tree) -> Result<Option<&'a Node>, String> {
        let (selector, nth, raw) = self.selector()?;
        if !matches!(selector, Selector::Ref(_))
            && selector.hits(&tree.nodes).len() <= nth.unwrap_or(0)
        {
            return Ok(None);
        }
        resolve(&tree.nodes, &raw, &selector, nth).map(|i| Some(&tree.nodes[i]))
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub(super) enum Keys {
    One(String),
    Many(Vec<String>),
}

impl Keys {
    pub(super) fn strokes(&self) -> Vec<&str> {
        match self {
            Keys::One(keys) => keys.split_whitespace().collect(),
            Keys::Many(keys) => keys.iter().flat_map(|k| k.split_whitespace()).collect(),
        }
    }
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(tag = "for", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub(super) enum Seen {
    Select {
        #[serde(flatten)]
        element: Element,
        #[serde(default = "super::wait_timeout")]
        timeout_ms: u64,
    },
    Gone {
        #[serde(flatten)]
        absent: Absent,
        #[serde(default = "super::wait_timeout")]
        timeout_ms: u64,
    },
    Text {
        text: String,
        #[serde(default = "super::wait_timeout")]
        timeout_ms: u64,
    },
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(
    untagged,
    expecting = "what must be gone: \"ref\", \"select\" (with optional \"nth\") or \"text\""
)]
pub(super) enum Absent {
    Element(Element),
    Text { text: String },
}

impl Seen {
    pub(super) fn timeout_ms(&self) -> u64 {
        match self {
            Seen::Select { timeout_ms, .. }
            | Seen::Gone { timeout_ms, .. }
            | Seen::Text { timeout_ms, .. } => *timeout_ms,
        }
    }
}

pub(super) fn check(seen: &Seen, tree: &Tree) -> Result<Value, String> {
    match seen {
        Seen::Select { element, .. } => element
            .find(tree)
            .map(|node| json!({ "ref": node.id, "node": line(node) })),
        Seen::Gone { absent, .. } => {
            let (hits, raw) = match absent {
                Absent::Element(element) => {
                    let (selector, _, raw) = element.selector()?;
                    (selector.hits(&tree.nodes), raw)
                }
                Absent::Text { text } => (holding(&tree.nodes, text).collect(), text.clone()),
            };
            if hits.is_empty() {
                Ok(json!({ "gone": raw }))
            } else {
                Err(format!(
                    "`{raw}` still matches {}:{}",
                    hits.len(),
                    listing(&tree.nodes, &hits)
                ))
            }
        }
        Seen::Text { text, .. } => holding(&tree.nodes, text)
            .next()
            .map(|i| json!({ "ref": tree.nodes[i].id, "node": line(&tree.nodes[i]) }))
            .ok_or_else(|| {
                format!(
                    "no label, value or description among {} nodes contains {text:?}",
                    tree.nodes.len()
                )
            }),
    }
}

fn holding<'a>(nodes: &'a [Node], text: &str) -> impl Iterator<Item = usize> + 'a {
    let needle = text.to_lowercase();
    (0..nodes.len()).filter(move |&i| {
        let node = &nodes[i];
        [&node.label, &node.value, &node.description]
            .into_iter()
            .any(|field| contains(field, &needle))
    })
}

pub(super) async fn wait(
    handle: AnyWindowHandle,
    seen: Seen,
    cx: &mut AsyncApp,
) -> Result<Value, String> {
    let timeout_ms = seen.timeout_ms();
    if let Seen::Select { element, .. }
    | Seen::Gone {
        absent: Absent::Element(element),
        ..
    } = &seen
    {
        element.selector()?;
    }
    let started = Instant::now();
    loop {
        let tree = read(handle, cx).await?;
        let last = match check(&seen, &tree) {
            Ok(mut found) => {
                found["waitedMs"] = json!(started.elapsed().as_millis() as u64);
                return Ok(found);
            }
            Err(last) => last,
        };
        if started.elapsed() >= Duration::from_millis(timeout_ms) {
            return Err(format!(
                "timed out after {timeout_ms} ms; last seen: {last}"
            ));
        }
        cx.background_executor().timer(POLL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::super::tree::tests::{node, tree_of};
    use super::*;

    fn sel(raw: &str) -> Selector {
        Selector::parse(raw).unwrap()
    }

    #[test]
    fn selector_parse() {
        assert_eq!(
            sel("button"),
            Selector::Match {
                role: Some("Button".into()),
                text: None
            }
        );
        assert_eq!(
            sel("\"New chat\""),
            Selector::Match {
                role: None,
                text: Some("New chat".into())
            }
        );
        assert_eq!(
            sel(" textinput  \"Say \\\"hi\\\"\" "),
            Selector::Match {
                role: Some("TextInput".into()),
                text: Some("Say \"hi\"".into())
            }
        );
        assert_eq!(sel("#e12"), Selector::Ref("e12".into()));
        assert_eq!(sel("#w1-3"), Selector::Ref("w1-3".into()));
        for bad in [
            "",
            "#12",
            "#e",
            "#e1x",
            "#w1",
            "#w-3",
            "#w1-",
            "Buton",
            "Button New",
            "Button \"open",
            "\"\"",
            "Button \"a\" extra",
            "\"",
        ] {
            let error = Selector::parse(bad).unwrap_err();
            assert!(error.starts_with("bad selector"), "{bad}: {error}");
        }
    }

    #[test]
    fn label_matches_before_value_and_placeholder() {
        let mut input = node("e2", "TextInput", None);
        input.value = Some("send this".into());
        let mut search = node("e3", "SearchInput", None);
        search.placeholder = Some("Search".into());
        let nodes = vec![node("e1", "Button", Some("Send")), input, search];
        assert_eq!(sel("\"send\"").hits(&nodes), vec![0]);
        assert_eq!(sel("TextInput \"send\"").hits(&nodes), vec![1]);
        assert_eq!(sel("\"this\"").hits(&nodes), vec![1]);
        assert_eq!(sel("\"search\"").hits(&nodes), vec![2]);
        assert_eq!(sel("Button").hits(&nodes), vec![0]);
        assert_eq!(sel("#e3").hits(&nodes), vec![2]);
        let switches = vec![
            node("e4", "Switch", Some("Compact model picker")),
            node("e5", "Switch", Some("Compact mode")),
        ];
        assert_eq!(sel("Switch \"compact MODE\"").hits(&switches), vec![1]);
        assert_eq!(sel("Switch \"compact\"").hits(&switches), vec![0, 1]);
    }

    #[test]
    fn a_web_node_never_shadows_a_native_one_in_the_same_tier() {
        let web = |id: &str, label: &str| {
            let mut node = node(id, "Button", Some(label));
            node.web = true;
            node
        };
        let tree = tree_of(&[
            (0, node("e1", "Window", None)),
            (1, web("w1-0", "page")),
            (2, web("w1-3", "Save")),
            (1, node("e2", "Button", Some("Save"))),
            (1, node("e3", "Button", Some("Save draft"))),
            (2, web("w1-4", "Option B")),
        ]);
        let found = |select: &str| {
            Element::Select {
                select: select.into(),
                nth: None,
            }
            .find(&tree)
            .map(|node| node.id.clone())
        };
        assert_eq!(found("Button \"Save\""), Ok("e2".into()));
        assert_eq!(found("Button \"Option B\""), Ok("w1-4".into()));
        assert_eq!(
            found("Button \"save d\""),
            Ok("e3".into()),
            "a native substring hit beats nothing"
        );
        assert_eq!(sel("\"save\"").hits(&tree.nodes), vec![3]);
        assert_eq!(sel("#w1-3").hits(&tree.nodes), vec![2]);
        let mut only_web = tree.clone();
        only_web
            .nodes
            .retain(|node| node.id != "e2" && node.id != "e3");
        assert_eq!(
            sel("Button \"Save\"").hits(&only_web.nodes),
            vec![2],
            "with no native match the page's node resolves"
        );
    }

    #[test]
    fn text_and_gone_text_read_label_value_and_description() {
        let mut button = node("e1", "Button", Some("Send"));
        button.placeholder = Some("Ask anything".into());
        let mut status = node("e2", "StaticText", None);
        status.value = Some("Picked: Blue".into());
        let tree = tree_of(&[(0, button), (0, status)]);
        let gone = |text: &str| {
            let absent = Absent::Text { text: text.into() };
            check(
                &Seen::Gone {
                    absent,
                    timeout_ms: 0,
                },
                &tree,
            )
        };
        let seen = |text: &str| {
            let text = text.into();
            check(
                &Seen::Text {
                    text,
                    timeout_ms: 0,
                },
                &tree,
            )
        };
        for present in ["picked: blue", "send"] {
            let error = gone(present).unwrap_err();
            assert!(
                error.starts_with(&format!("`{present}` still matches 1:")),
                "{error}"
            );
            assert!(seen(present).is_ok());
        }
        for absent in ["ask anything", "Button"] {
            assert_eq!(gone(absent), Ok(json!({ "gone": absent })));
            assert!(seen(absent).is_err());
        }
    }

    #[test]
    fn nth_ambiguity_and_misses() {
        let nodes = vec![
            node("e1", "Button", Some("Open file")),
            node("e2", "Link", Some("Open")),
            node("e3", "Button", Some("Open folder")),
            node("e4", "Button", Some("Close")),
        ];
        let resolve = |raw: &str, nth| resolve(&nodes, raw, &sel(raw), nth);
        assert_eq!(resolve("Button \"open\"", Some(1)), Ok(2));
        assert_eq!(resolve("Link", None), Ok(1));
        let ambiguous = resolve("Button \"open\"", None).unwrap_err();
        assert!(ambiguous.contains("matches 2 elements"), "{ambiguous}");
        assert!(ambiguous.contains("e1 Button \"Open file\" @10,20 100x30"));
        assert!(ambiguous.contains("e3 Button \"Open folder\""));
        assert!(!ambiguous.contains("e2"), "{ambiguous}");
        let range = resolve("Button \"open\"", Some(2)).unwrap_err();
        assert!(
            range.contains("nth 2 is out of range, 2 matches"),
            "{range}"
        );
        let miss = resolve("Button \"open filez\"", None).unwrap_err();
        assert!(miss.starts_with("no element matches `Button \"open filez\"`; near:"));
        for id in ["e1", "e3", "e4"] {
            assert!(miss.contains(&format!("{id} Button")), "{miss}");
        }
        let gone = resolve("#e9", None).unwrap_err();
        assert!(gone.contains("gone"), "{gone}");
        let tree = Tree {
            nodes,
            ..Tree::default()
        };
        let select = |select: &str, nth| Element::Select {
            select: select.into(),
            nth,
        };
        let absent = |element: Element| element.find_or_absent(&tree).map(|n| n.map(|n| &n.id));
        assert_eq!(absent(select("Tab", None)), Ok(None));
        assert_eq!(absent(select("Link", Some(1))), Ok(None));
        assert_eq!(absent(select("Link", None)), Ok(Some(&"e2".to_owned())));
        assert!(absent(select("Button \"open\"", None)).is_err());
        assert!(absent(select("Buton", None)).is_err());
        assert!(absent(Element::Ref { id: "e9".into() }).is_err());
    }
}
