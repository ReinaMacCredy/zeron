//! Agent pages (`html_render`) for ds/html_card: zeron_proto's page host behind `html.*` calls.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use serde_json::json;
use zeron_proto::html_render::{self as shared, PageTheme, Rgba};
use zgx_script::{ScriptHost, Value};

/// Locks what the channel calls and posts null-prototype objects: the page shares this world until zgx gives it an isolated one.
const PICK_SCRIPT: &str = r#"(function(){var s=document.currentScript,W=window,z=W.zgx,H="__HEIGHT__",N="__PICKS__";if(!z)return;var O=Object,DP=O.defineProperty,OC=O.create,J=W.JSON,P=J.parse,post=z.postMessage;function fix(o,k,v){try{var d=OC(null);d.value=v;d.writable=false;d.configurable=false;DP(o,k,d);}catch(e){}}fix(W,"JSON",J);fix(J,"stringify",J.stringify);var h=W.webkit&&W.webkit.messageHandlers&&W.webkit.messageHandlers.zgx;if(h)fix(h,"postMessage",h.postMessage);function rec(a){var o=OC(null);for(var i=0;i+1<a.length;i+=2){var d=OC(null);d.value=a[i+1];d.enumerable=true;DP(o,a[i],d);}return o;}function send(n,k,v){if(n)post(rec(["kind","zeron.page","nonce",n,k,v]));}W.ipc={postMessage:function(t){try{var m=P(t);if(m&&typeof m.height==="number")send(H,"height",m.height);}catch(e){}}};var st={chosen:[],sent:[],locked:[],count:""};function g(b){return b.getAttribute("data-zeron-group")||"";}function label(b){return b.getAttribute("data-zeron-label")||(b.textContent||"").replace(/\s+/g," ").trim().slice(0,80);}function has(l,gr,c){for(var i=0;i<l.length;i++){if(l[i][0]===gr&&l[i][1]===c)return true;}return false;}function groups(){var out=[],seen=OC(null),all=document.querySelectorAll("[data-zeron-pick]");for(var i=0;i<all.length;i++){var b=all[i],gr=g(b);if(gr&&!seen[gr]){seen[gr]=1;var l=b.closest("[data-zeron-group-label]");out[out.length]=rec(["code",gr,"name",(l&&l.getAttribute("data-zeron-group-label"))||gr]);}}return out;}function apply(){var all=document.querySelectorAll("[data-zeron-pick]");for(var i=0;i<all.length;i++){var b=all[i],gr=g(b),c=b.getAttribute("data-zeron-pick"),sent=has(st.sent,gr,gr?c:label(b)),on=sent||has(st.chosen,gr,c),locked=gr?st.locked.indexOf(gr)>=0:sent;if(on)b.setAttribute("data-zeron-chosen","");else b.removeAttribute("data-zeron-chosen");if(locked)b.setAttribute("data-zeron-locked","");else b.removeAttribute("data-zeron-locked");b.setAttribute("aria-pressed",on?"true":"false");}var n=document.querySelectorAll("[data-zeron-count]");for(var j=0;j<n.length;j++)n[j].textContent=st.count;}fix(W,"__zeronPicks",function(next){if(next)st=next;apply();});document.addEventListener("click",function(e){if(!e.isTrusted||!N)return;var b=e.target&&e.target.closest?e.target.closest("[data-zeron-pick]"):null;if(!b)return;e.preventDefault();var gr=g(b),c=b.getAttribute("data-zeron-pick"),r=b.getBoundingClientRect(),at=document.elementFromPoint(r.left+r.width/2,r.top+r.height/2);send(N,"pick",rec(["group",gr,"code",c,"label",label(b),"groups",groups(),"rect",[r.left,r.top,r.width,r.height],"covered",!(at&&(b===at||b.contains(at)))]));},true);document.addEventListener("DOMContentLoaded",function(){apply();send(H,"ready",groups());});var css=document.createElement("style");css.id="zeron-picks";css.textContent="[data-zeron-pick]{cursor:pointer}[data-zeron-pick][data-zeron-chosen]{box-shadow:inset 0 0 0 1.5px var(--zeron-pick,var(--accent))}[data-zeron-pick][data-zeron-locked]{cursor:default}[data-zeron-pick][data-zeron-locked]:not([data-zeron-chosen]){opacity:.45}";if(s&&s.parentNode){s.parentNode.insertBefore(css,s);s.remove();}else{(document.head||document.documentElement).appendChild(css);}})();"#;

pub fn pick_script(nonce: &str, picks: &str) -> String {
    PICK_SCRIPT.replace("__HEIGHT__", nonce).replace("__PICKS__", picks)
}

/// A page script before the bootstrap could hook what it captures, so picks stay off then.
pub fn bootstrap_first(html: &str, themed: &str) -> bool {
    let same = html.bytes().zip(themed.bytes()).take_while(|(a, b)| a == b).count();
    let shared = &html[..html.floor_char_boundary(same)];
    let at = shared.rfind('>').map_or(0, |i| i + 1);
    let tag = |lower: &str, name: &str| {
        lower.strip_prefix(name).is_some_and(|rest| rest.chars().next().is_none_or(|c| c.is_whitespace() || c == '>' || c == '/'))
    };
    let mut rest = &html[..at];
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
        if rest.is_empty() {
            return true;
        }
        let lower = rest.chars().take(10).collect::<String>().to_ascii_lowercase();
        let end = if rest.starts_with("<!--") {
            rest.find("-->").map(|i| i + 3)
        } else if lower.starts_with("<!doctype") || tag(&lower, "<html") || tag(&lower, "<head") {
            rest.find('>').map(|i| i + 1)
        } else {
            None
        };
        match end {
            Some(end) => rest = &rest[end..],
            None => return false,
        }
    }
}

type Pages = Rc<RefCell<HashMap<String, Rc<str>>>>;

thread_local! {
    static PAGES: Pages = Rc::default();
}

fn text(value: Option<&Value>) -> String {
    value.map(crate::auth::text).unwrap_or_default()
}

fn num(value: Option<&Value>, or: f64) -> f64 {
    value.and_then(Value::as_f64).unwrap_or(or)
}

fn key_of(arg: &Value) -> (String, Option<String>) {
    let path = text(arg.field("path"));
    let device = Some(text(arg.field("device"))).filter(|d| !d.is_empty());
    (path, device)
}

fn page_key(path: &str, device: Option<&str>) -> String {
    format!("{}\0{path}", device.unwrap_or_default())
}

pub fn page_theme(theme: Option<&Value>) -> PageTheme {
    let color = |name: &str| {
        theme
            .and_then(|t| t.field(name))
            .map(crate::auth::text)
            .and_then(|hex| Rgba::hex(&hex))
            .unwrap_or(Rgba { r: 0.5, g: 0.5, b: 0.5, a: 1.0 })
    };
    let flag = |name: &str| theme.and_then(|t| t.field(name)).is_some_and(|v| matches!(v, Value::Bool(true)));
    let words = |name: &str| theme.and_then(|t| t.field(name)).map(crate::auth::text).unwrap_or_default();
    let size = |name: &str, or: f32| theme.and_then(|t| t.field(name)).and_then(Value::as_f64).map_or(or, |v| v as f32);
    PageTheme {
        dark: flag("dark"),
        glass: flag("glass"),
        bg: color("bg"),
        text: color("text"),
        text_faint: color("text_faint"),
        text_muted: color("text_muted"),
        element_hover: color("element_hover"),
        hover: color("hover"),
        selection: color("selection"),
        surface_card: color("surface_card"),
        surface_overlay: color("surface_overlay"),
        surface_raised: color("surface_raised"),
        border: color("border"),
        border_strong: color("border_strong"),
        input_bg: color("input_bg"),
        accent: color("accent"),
        solid: color("solid"),
        on_solid: color("on_solid"),
        on_accent: color("on_accent"),
        accent_wash: color("accent_wash"),
        accent_strong: color("accent_strong"),
        danger: color("danger"),
        danger_strong: color("danger_strong"),
        danger_muted: color("danger_muted"),
        warning: color("warning"),
        warning_muted: color("warning_muted"),
        success: color("success"),
        success_muted: color("success_muted"),
        code_wash: color("code_wash"),
        code_text: color("code_text"),
        font_sans: words("font_sans"),
        font_mono: words("font_mono"),
        code_font_size: size("code_font_size", 12.5),
        text_size: size("text_size", 14.0),
        line_height: size("line_height", 22.0),
    }
}

pub fn themed(html: &str, theme: &PageTheme, font_size: f32, pick: Option<&str>, nonce: &str, picks: &str) -> (String, bool) {
    let first = bootstrap_first(html, &shared::themed_page(html, theme, font_size, true, ""));
    let mut extra = pick_script(nonce, if first { picks } else { "" });
    if let Some(pick) = pick.and_then(Rgba::hex) {
        extra.push_str(&format!(
            "document.documentElement.style.setProperty(\"--zeron-pick\",\"{}\");",
            shared::css_color(pick)
        ));
    }
    (shared::themed_page(html, theme, font_size, true, &extra), first)
}

fn nonce() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
    format!("{:016x}", h.finish())
}

pub fn picks_script(arg: &Value) -> String {
    let pairs = |name: &str| -> Vec<(String, String)> {
        arg.field(name)
            .map(Value::list)
            .unwrap_or_default()
            .iter()
            .map(|pair| (text(pair.field("group")), text(pair.field("code"))))
            .collect()
    };
    let locked: Vec<String> = arg.field("locked").map(Value::list).unwrap_or_default().iter().map(crate::auth::text).collect();
    let state = json!({
        "chosen": pairs("chosen").into_iter().map(|(g, c)| json!([g, c])).collect::<Vec<_>>(),
        "sent": pairs("sent").into_iter().map(|(g, c)| json!([g, c])).collect::<Vec<_>>(),
        "locked": locked,
        "count": text(arg.field("count")),
    });
    format!("window.__zeronPicks&&window.__zeronPicks({state});")
}

/// The shipping app's `server_answers`.
fn server_answers(url: &str) -> bool {
    use std::net::ToSocketAddrs;
    let Ok(live) = shared::live_url(url) else {
        return false;
    };
    (live.host, live.port).to_socket_addrs().is_ok_and(|mut addresses| {
        addresses.any(|address| {
            address.ip().is_loopback()
                && std::net::TcpStream::connect_timeout(&address, std::time::Duration::from_secs(1)).is_ok()
        })
    })
}

async fn off_thread<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (tx, mut rx) = zgx::task::latest();
    std::thread::spawn(move || {
        tx.send(work());
    });
    rx.recv().await
}

async fn read_page(path: &str, device: Option<&str>) -> Result<String, String> {
    use base64::Engine as _;
    let mut bytes = Vec::new();
    let mut offset = 0u64;
    loop {
        let mut params = json!({ "path": path, "offset": offset });
        if let Some(device) = device {
            params["targetDeviceId"] = json!(device);
        }
        let chunk = crate::engine::get().call("ReadAttachmentChunk", params).await?;
        let data = base64::engine::general_purpose::STANDARD
            .decode(chunk["data"].as_str().unwrap_or(""))
            .map_err(|_| "The page came back garbled.".to_string())?;
        bytes.extend_from_slice(&data);
        if bytes.len() > shared::MAX_PAGE_BYTES {
            return Err("The page is too large.".into());
        }
        if chunk["done"].as_bool().unwrap_or(false) {
            return String::from_utf8(bytes).map_err(|_| "The page isn't text.".to_string());
        }
        let next = chunk["nextOffset"].as_u64().unwrap_or(0);
        if next <= offset {
            return Err("The page changed while it was read.".into());
        }
        offset = next;
    }
}

/// `source` stands in for the stored file in fixtures.
async fn page(pages: Pages, arg: &Value) -> Result<Rc<str>, String> {
    let (path, device) = key_of(arg);
    let key = page_key(&path, device.as_deref());
    if let Some(page) = pages.borrow().get(&key) {
        return Ok(page.clone());
    }
    let html: Rc<str> = match arg.field("source") {
        Some(Value::Str(source)) => Rc::from(source.as_ref()),
        _ => read_page(&path, device.as_deref()).await?.into(),
    };
    pages.borrow_mut().insert(key, html.clone());
    Ok(html)
}

pub fn install(host: &ScriptHost) {
    let pages = PAGES.with(Rc::clone);
    let p = pages.clone();
    host.async_action("html.page", move |arg| {
        let p = p.clone();
        async move {
            let html = page(p, &arg).await?;
            let theme = page_theme(arg.field("theme"));
            let font = num(arg.field("font_size"), 16.) as f32;
            let pick = arg.field("theme").and_then(|t| t.field("pick")).map(crate::auth::text);
            let (nonce, picks) = (nonce(), nonce());
            let (out, heard) = themed(&html, &theme, font, pick.as_deref(), &nonce, &picks);
            Ok(Value::record([
                ("html", Value::from(out.as_str())),
                ("nonce", Value::from(nonce.as_str())),
                ("picks", if heard { Value::from(picks.as_str()) } else { Value::Nil }),
            ]))
        }
    });
    host.function("html.retheme", |arg| {
        let theme = page_theme(arg.field("theme"));
        let mut js = shared::retheme_script(&theme, num(arg.field("font_size"), 16.) as f32, true);
        if let Some(pick) = arg.field("theme").and_then(|t| t.field("pick")).map(crate::auth::text).and_then(|h| Rgba::hex(&h)) {
            js.push_str(&format!(
                "document.documentElement.style.setProperty(\"--zeron-pick\",\"{}\");",
                shared::css_color(pick)
            ));
        }
        Value::from(js.as_str())
    });
    host.function("html.frame", |arg| {
        let agent = shared::clamp_height(num(arg.field("agent"), 600.)) as f32;
        let content = arg.field("content").and_then(Value::as_f64).map(|v| v as f32);
        let height = shared::frame_height(agent, content);
        Value::record([
            ("height", Value::Num(f64::from(height))),
            ("agent", Value::Num(f64::from(agent))),
            ("overflows", Value::Bool(content.is_some_and(|c| shared::overflows(agent, c)))),
            ("scrolls", Value::Bool(shared::scrolls_inside(height, content))),
        ])
    });
    host.function("html.live", |arg| {
        let url = crate::auth::text(&arg);
        match shared::live_url(&url) {
            Ok(live) => Value::record([("label", Value::from(live.label)), ("server", Value::from(live.server)), ("url", Value::from(live.url))]),
            Err(_) => Value::Nil,
        }
    });
    host.async_action("html.reach", |arg| async move {
        let url = crate::auth::text(&arg);
        Ok(Value::Bool(off_thread(move || server_answers(&url)).await.unwrap_or(false)))
    });
    let p = pages.clone();
    host.function("html.source", move |arg| {
        let (path, device) = key_of(&arg);
        p.borrow()
            .get(&page_key(&path, device.as_deref()))
            .map_or(Value::Nil, |html| Value::from(shared::source_text(html).as_str()))
    });
    let p = pages.clone();
    host.async_action("html.save", move |arg| {
        let p = p.clone();
        async move {
            let html = page(p, &arg).await?;
            let to = text(arg.field("to"));
            let bytes = html.as_bytes().to_vec();
            off_thread(move || std::fs::write(to, bytes).map_err(|e| e.to_string())).await.unwrap_or(Err("failed".into()))?;
            Ok(Value::Bool(true))
        }
    });
    host.function("html.file_name", |arg| Value::from(shared::page_file_name(&crate::auth::text(&arg)).as_str()));
    host.async_action("html.title", |arg| async move {
        let path = crate::auth::text(&arg);
        let read = off_thread(move || shared::read_title(std::path::Path::new(&path))).await.unwrap_or(Err(()));
        Ok(match read {
            Ok(title) => Value::record([("title", title.as_deref().map_or(Value::Nil, Value::from))]),
            Err(()) => Value::record([("missing", Value::Bool(true))]),
        })
    });
    host.function("html.picks_state", |arg| Value::from(picks_script(&arg).as_str()));
    let p = pages.clone();
    host.function("html.forget", move |arg| {
        let (path, device) = key_of(&arg);
        p.borrow_mut().remove(&page_key(&path, device.as_deref()));
        Value::Nil
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use zgx_script::testing::TestApp;

    fn mount() -> TestApp {
        let ui = std::env::var_os("ZERON_ZGX_TEST_UI")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui"));
        let app = TestApp::new(|runtime| ScriptHost::new(runtime, ui.join("chat_fixture.luau")));
        app.ui.scene.borrow_mut().resize(960., 900.);
        app.frames(30);
        assert_eq!(app.error(), None, "the fixture failed");
        assert!(app.host.run_command("go_s2"));
        app.frames(30);
        assert!(app.host.run_command("pages_seed"));
        app.frames(30);
        app
    }

    fn post(app: &mut TestApp, card: &str, path: &str, pick: Value) {
        let message = Value::record([
            ("id", Value::from(format!("html-page-{card}").as_str())),
            (
                "value",
                Value::record([("kind", Value::from("zeron.page")), ("nonce", Value::from(format!("p:{path}").as_str())), ("pick", pick)]),
            ),
        ]);
        app.host.set_state("page_message", message).unwrap();
        assert!(app.host.run_command("page_post"));
        app.frames(20);
    }

    fn sent(app: &TestApp) -> String {
        app.host
            .states()
            .into_iter()
            .find(|(name, _)| name == "calls" || name.ends_with("::calls"))
            .map(|(_, v)| crate::auth::text(&v))
            .unwrap_or_default()
    }

    #[test]
    fn settled_pages_and_the_tray_at_rest_run_no_script() {
        let mut app = mount();
        let groups = Value::List(vec![Value::record([("code", Value::from("A")), ("name", Value::from("Góc trang"))])].into());
        let pick = Value::record([("group", Value::from("A")), ("code", Value::from("A2")), ("label", Value::from("Nổi sát đỉnh")), ("groups", groups)]);
        post(&mut app, "p1-tc1-html", "/u/html/s2/frame.html", pick);
        assert!(app.find("#composer-picks").is_some(), "the tray shows");
        app.frames(120);
        let before = app.calls();
        app.frames(180);
        assert_eq!(app.calls() - before, 0, "pages, cards and the tray at rest run no script");
    }

    #[test]
    fn a_prompt_carries_its_picks_as_zeron_proto_writes_them() {
        use shared::{Pick, PickGroup, with_picks};
        let mut app = mount();
        let group = |code: &str, name: &str| Value::record([("code", Value::from(code)), ("name", Value::from(name))]);
        let groups = Value::List(vec![group("A", "Góc trang"), group("B", "Thanh tab"), group("C", "Rail")].into());
        let pick = |group: &str, code: &str, label: &str, groups: &Value| {
            Value::record([("group", Value::from(group)), ("code", Value::from(code)), ("label", Value::from(label)), ("groups", groups.clone())])
        };
        post(&mut app, "p1-tc1-html", "/u/html/s2/frame.html", pick("B", "B2", "Không viền", &groups));
        post(&mut app, "p1-tc1-html", "/u/html/s2/frame.html", pick("A", "A2", "Nổi sát đỉnh", &groups));
        let none = Value::List(Vec::new().into());
        post(&mut app, "p2-tc2-html", "/u/html/s2/pictures.html", pick("", "p1", "Sea", &none));
        app.type_text("role:textbox \"Message\"", "+ header của C2");
        app.keys("Enter");
        app.frames(60);
        let p = |group: Option<&str>, code: &str, label: &str| Pick { group: group.map(str::to_owned), code: code.into(), label: label.into() };
        let zeron = with_picks(
            "+ header của C2",
            &[
                (
                    "Khung app".into(),
                    vec![p(Some("A"), "A2", "Nổi sát đỉnh"), p(Some("B"), "B2", "Không viền")],
                    vec![PickGroup { code: "C".into(), name: "Rail".into() }],
                ),
                ("Ảnh".into(), vec![p(None, "p1", "Sea")], vec![]),
            ],
        );
        let calls = sent(&app);
        assert!(calls.contains(&format!("send:s2:{zeron}")), "sent as zeron_proto writes it:\n{zeron}\n---\n{calls}");
        let (words, cards) = shared::extract_picks(&zeron);
        assert_eq!(words, "+ header của C2");
        assert_eq!(cards.len(), 2, "and read back by the host as two cards' picks");
    }

    #[test]
    fn a_page_is_handed_the_shared_bootstrap_and_its_pick_script() {
        let theme = page_theme(None);
        let (out, heard) = themed("<p>x</p>", &theme, 16., Some("#c0347a"), "n0", "p0");
        let shared_page = shared::themed_page("<p>x</p>", &theme, 16., true, "");
        let (head, _) = shared_page.split_once("</script>").unwrap();
        assert!(out.starts_with(head), "the shared bootstrap comes first");
        assert!(heard && out.contains("H=\"n0\",N=\"p0\"") && out.contains("e.isTrusted"));
        assert!(out.contains("--zeron-pick\",\"rgba(192,52,122,1)\""));
        assert!(out.ends_with("</head><p>x</p>"));
        let (early, heard) = themed("<script>hook()</script><head></head><p>x</p>", &theme, 16., None, "n0", "p0");
        assert!(!heard && early.contains("H=\"n0\",N=\"\""), "a script before the bootstrap: no pick nonce in the page");
    }

    #[test]
    fn only_a_doctype_comments_and_the_html_and_head_tags_may_come_before_the_bootstrap() {
        let theme = page_theme(None);
        let first = |html: &str| bootstrap_first(html, &shared::themed_page(html, &theme, 16., true, ""));
        for html in [
            "<p>x</p>",
            "<!doctype html><title>x</title><script>a()</script>",
            "\u{feff}<!-- made by -->\n<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\"><script>a()</script>",
            "<html><body><p>no head</p></body></html>",
            "<!doctype html><html><head><title>x</title></head><body><button data-zeron-pick=\"A1\">A1</button></body></html>",
        ] {
            assert!(first(html), "{html}");
        }
        for html in [
            "<script>hook()</script><head></head>",
            "<!doctype html><script>hook()</script><html><head></head>",
            "<html><body><img src=x onerror=hook()><head></head>",
            "<script>hook()</script><html><body></body></html>",
            "<html data-x=\"a>b\"><script>hook()</script><head></head>",
            "<svg><script>hook()</script></svg><head></head>",
        ] {
            assert!(!first(html), "{html}");
        }
    }

    /// The two leak routes the U23 review reproduced, and the ones next to them, run under node
    /// against both zgx channels (macOS's and WebKitGTK's, as at the pinned rev).
    #[test]
    fn a_pages_own_script_never_reads_the_nonces_nor_forges_a_signed_message() {
        let dir = std::env::temp_dir().join(format!("zeron-pick-script-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (script, harness) = (dir.join("pick.js"), dir.join("harness.js"));
        std::fs::write(&script, pick_script("HEIGHTNONCE1", "PICKSNONCE22")).unwrap();
        std::fs::write(&harness, LEAK_HARNESS).unwrap();
        let run = std::process::Command::new("node").arg(&harness).arg(&script).arg("HEIGHTNONCE1").arg("PICKSNONCE22").output();
        let _ = std::fs::remove_dir_all(&dir);
        let out = match run {
            Ok(out) => out,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("node is not on PATH: the page-script leak test did not run");
                return;
            }
            Err(e) => panic!("node: {e}"),
        };
        let said = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(out.status.success(), "{said}");
        assert!(said.contains("checked 26"), "every attack ran on both channels:\n{said}");
    }

    const LEAK_HARNESS: &str = r#"
const vm = require('vm'), fs = require('fs');
const [, , file, HN, PN] = process.argv;
const pick = fs.readFileSync(file, 'utf8');
const MAC = "(() => { const handler = window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.zgx; if (!handler || window.zgx) return; Object.defineProperty(window, 'zgx', { value: Object.freeze({ postMessage(value) { handler.postMessage(JSON.stringify(value === undefined ? null : value)); } }) }); Object.defineProperty(window, 'zgui', { value: window.zgx }); })();";
const GTK = "(() => { const handler = window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.zgx; if (!handler || window.zgx) return; const post = handler.postMessage.bind(handler); const stringify = JSON.stringify; const origin = location.origin; const main = window === window.top; Object.defineProperty(window, 'zgx', { value: Object.freeze({ postMessage(value) { post({ n: 'zgx', o: origin, m: main, b: stringify(value === undefined ? null : value) }); } }) }); Object.defineProperty(window, 'zgui', { value: window.zgx }); })();";
const SETUP = "var handlers={};function Btn(c,g,l){this.a={'data-zeron-pick':c,'data-zeron-group':g,'data-zeron-label':l};this.textContent=l;}Btn.prototype.getAttribute=function(k){return Object.prototype.hasOwnProperty.call(this.a,k)?this.a[k]:null;};Btn.prototype.setAttribute=function(k,v){this.a[k]=v;};Btn.prototype.removeAttribute=function(k){delete this.a[k];};Btn.prototype.closest=function(s){return s==='[data-zeron-pick]'?this:null;};Btn.prototype.getBoundingClientRect=function(){return {left:1,top:2,width:3,height:4};};Btn.prototype.contains=function(x){return x===this;};var buttons=[new Btn('A1','A','One'),new Btn('A2','A','Two')];var document={currentScript:{parentNode:{insertBefore:function(){}},remove:function(){}},addEventListener:function(k,f){(handlers[k]=handlers[k]||[]).push(f);},querySelectorAll:function(s){return s==='[data-zeron-pick]'?buttons:[];},createElement:function(){return {};},elementFromPoint:function(){return buttons[1];},head:{appendChild:function(){}},documentElement:{appendChild:function(){},style:{setProperty:function(){}}}};function MH(){}MH.prototype.postMessage=function(t){__native(t);};var webkit={messageHandlers:{zgx:new MH()}};var location={origin:'null'};var window=this;var top=this;function steal(v){__steal(v);}";
const RUN = "var i,l=handlers.DOMContentLoaded||[];for(i=0;i<l.length;i++)l[i]();if(window.ipc)window.ipc.postMessage('{\"height\":7}');l=handlers.click||[];for(i=0;i<l.length;i++)l[i]({isTrusted:true,target:buttons[1],preventDefault:function(){}});";
const ATTACKS = {
  prototype_setter: "Object.defineProperty(Object.prototype,'nonce',{configurable:true,set:function(v){steal(v);}});",
  stringify_wrapped: "var s0=JSON.stringify;JSON.stringify=function(v){steal(s0(v));return s0.apply(this,arguments);};",
  json_replaced: "var s1=JSON.stringify;window.JSON={stringify:function(v){steal(s1(v));return s1(v);},parse:JSON.parse};",
  json_shadowed: "let JSON={stringify:function(v){steal(v);return '{}';},parse:function(){return {};}};",
  to_json: "function forged(n){var o=Object.create(null),p=Object.create(null);p.group='A';p.code='A1';p.label='forged';o.kind='zeron.page';o.nonce=n;o.pick=p;return o;}Object.prototype.toJSON=function(){steal(this);return forged(this.nonce);};",
  array_to_json: "Array.prototype.toJSON=function(){steal(this);var o=Object.create(null);o.pick='forged';return o;};",
  descriptor_getters: "['get','set','value','writable','enumerable','configurable'].forEach(function(k){try{Object.defineProperty(Object.prototype,k,{configurable:true,get:function(){steal(this);return undefined;}});}catch(e){}});",
  handler_wrapped: "var hp=webkit.messageHandlers.zgx,o0=hp.postMessage;try{hp.postMessage=function(t){steal(t);return o0.call(hp,t);};}catch(e){}var pr=Object.getPrototypeOf(hp),p0=pr.postMessage;pr.postMessage=function(t){steal(t);return p0.call(this,t);};",
  channel_replaced: "try{window.zgx={postMessage:function(v){steal(v);}};}catch(e){}try{Object.defineProperty(window,'zgx',{value:{postMessage:function(v){steal(v);}}});}catch(e){}",
  define_property_wrapped: "var d0=Object.defineProperty;Object.defineProperty=function(o,k,d){steal(d);return d0(o,k,d);};var c0=Object.create;Object.create=function(p){var o=c0(p);steal(o);return o;};",
  array_push_hooked: "var u0=Array.prototype.push;Array.prototype.push=function(){steal([].slice.call(arguments));return u0.apply(this,arguments);};Object.defineProperty(Array.prototype,'0',{configurable:true,set:function(v){steal(v);}});",
  string_hooked: "var r0=String.prototype.replace;String.prototype.replace=function(){steal(String(this));return r0.apply(this,arguments);};",
  picks_replaced: "try{window.__zeronPicks=function(v){steal(v);};}catch(e){}window.ipc={postMessage:function(t){steal(t);}};",
};
const seen = new Set();
function holds(v, depth) {
  if (typeof v === 'string') return v.includes(HN) || v.includes(PN);
  if (v === null || (typeof v !== 'object' && typeof v !== 'function') || depth > 6 || seen.has(v)) return false;
  seen.add(v);
  for (const k of Reflect.ownKeys(v)) {
    const d = Reflect.getOwnPropertyDescriptor(v, k);
    if (d && 'value' in d && holds(d.value, depth + 1)) return true;
  }
  return false;
}
let checked = 0, failed = [];
for (const [channel, CHANNEL] of [['mac', MAC], ['gtk', GTK]]) {
  for (const [name, attack] of Object.entries(ATTACKS)) {
    const stolen = [], native = [];
    const ctx = vm.createContext({ __steal: (v) => stolen.push(v), __native: (t) => native.push(t) });
    for (const code of [SETUP, CHANNEL, pick]) vm.runInContext(code, ctx);
    try { vm.runInContext(attack, ctx); } catch (e) {}
    try { vm.runInContext(RUN, ctx); } catch (e) { failed.push(`${channel}/${name}: the page broke the run: ${e}`); }
    checked++;
    seen.clear();
    if (stolen.some((v) => holds(v, 0))) failed.push(`${channel}/${name}: the page read a nonce`);
    const bodies = native.map((t) => JSON.parse(channel === 'gtk' ? t.b : t));
    const picks = bodies.filter((b) => b && b.nonce === PN);
    const ok = picks.length === 1 && picks[0].pick && picks[0].pick.code === 'A2' && picks[0].pick.label === 'Two' && Object.keys(picks[0]).sort().join() === 'kind,nonce,pick';
    if (!ok) failed.push(`${channel}/${name}: signed picks ${JSON.stringify(picks)}`);
    if (bodies.some((b) => b && b.nonce === HN && (b.pick !== undefined || !(typeof b.height === 'number' || b.ready !== undefined)))) failed.push(`${channel}/${name}: a height message carried more`);
  }
}
if (failed.length) { console.error(failed.join('\n')); process.exit(1); }
console.log(`checked ${checked}`);
"#;

    #[test]
    fn theme_roles_read_from_hex_and_fall_back_to_grey() {
        let theme = Value::record([("dark", Value::Bool(true)), ("accent", Value::from("#ec83b7ff")), ("font_sans", Value::from("Geist"))]);
        let t = page_theme(Some(&theme));
        assert!(t.dark && !t.glass);
        assert_eq!(shared::css_color(t.accent), "rgba(236,131,183,1)");
        assert_eq!(shared::css_color(t.bg), "rgba(128,128,128,1)");
        assert!(shared::theme_css(&t, 16., true).contains("--font-sans:\"Geist\", "));
    }
}
