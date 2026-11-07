//! Agent pages (`html_render`) for ds/html_card: the shipping app's page host
//! (crates/proto html_render.rs, shared with crates/ui), here for zgx's
//! `ui.webview`. A page is stored on the chat's host (the engine's uploads),
//! read once through the engine (`ReadAttachmentChunk`, from that device when
//! it isn't this one), and handed to the view with the theme bootstrap at the
//! start of its head: the theme's CSS variables, its height reported, and
//! the options it offers (`data-zeron-pick`) heard when clicked.
//!
//! The calls:
//!   html.page { path, device?, theme, font_size }        (`ui.call`)
//!       { html, nonce }: the page with its bootstrap; `nonce` signs what its
//!       bootstrap posts (a page's own script can't read it).
//!   html.retheme { theme, font_size }                     (`ui.call_sync`)
//!       the script that swaps a shown page's theme.
//!   html.frame { agent, content? }                        (`ui.call_sync`)
//!       { height, overflows, scrolls }: the frame for a page, by the
//!       agent's height and the height it reports.
//!   html.live url                                         (`ui.call_sync`)
//!       { label, server } of a local dev server's URL, or nil.
//!   html.reach url                                        (`ui.call`)
//!       whether that server answers.
//!   html.source { path, device? }                         (`ui.call_sync`)
//!       the page as the sheet's Source shows it (nil before it was read).
//!   html.save { path, device?, to }                       (`ui.call`)
//!       the page as the agent wrote it, saved at `to`.
//!   html.file_name title                                  (`ui.call_sync`)
//!   html.title path                                       (`ui.call`)
//!       { title? } of a page file here, or { missing = true }.
//!   html.picks_state { chosen, sent, locked, count }       (`ui.call_sync`)
//!       the script that shows a page's picks on its buttons.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use serde_json::json;
use zeron_proto::html_render::{self as shared, PageTheme, Rgba};
use zgx_script::{ScriptHost, Value};

/// What a page's bootstrap adds in zgx: the height its size script reports,
/// passed on to the view, and its options, clicked (`isTrusted` only),
/// with the button's place and what is in view there. The script removes
/// itself as it runs, so the page's own scripts never read its nonce.
const PICK_SCRIPT: &str = r#"(function(){var s=document.currentScript,z=window.zgx,N="__NONCE__";if(!z)return;var post=z.postMessage.bind(z);function send(m){m.kind="zeron.page";m.nonce=N;post(m);}window.ipc={postMessage:function(t){try{var m=JSON.parse(t);if(m&&typeof m.height==="number")send({height:m.height});}catch(e){}}};var st={chosen:[],sent:[],locked:[],count:""};function g(b){return b.getAttribute("data-zeron-group")||"";}function label(b){return b.getAttribute("data-zeron-label")||(b.textContent||"").replace(/\s+/g," ").trim().slice(0,80);}function has(l,gr,c){for(var i=0;i<l.length;i++){if(l[i][0]===gr&&l[i][1]===c)return true;}return false;}function groups(){var out=[],seen={},all=document.querySelectorAll("[data-zeron-pick]");for(var i=0;i<all.length;i++){var b=all[i],gr=g(b);if(gr&&!seen[gr]){seen[gr]=1;var l=b.closest("[data-zeron-group-label]");out.push({code:gr,name:(l&&l.getAttribute("data-zeron-group-label"))||gr});}}return out;}function apply(){var all=document.querySelectorAll("[data-zeron-pick]");for(var i=0;i<all.length;i++){var b=all[i],gr=g(b),c=b.getAttribute("data-zeron-pick"),sent=has(st.sent,gr,gr?c:label(b)),on=sent||has(st.chosen,gr,c),locked=gr?st.locked.indexOf(gr)>=0:sent;if(on)b.setAttribute("data-zeron-chosen","");else b.removeAttribute("data-zeron-chosen");if(locked)b.setAttribute("data-zeron-locked","");else b.removeAttribute("data-zeron-locked");b.setAttribute("aria-pressed",on?"true":"false");}var n=document.querySelectorAll("[data-zeron-count]");for(var j=0;j<n.length;j++)n[j].textContent=st.count;}window.__zeronPicks=function(next){if(next)st=next;apply();};document.addEventListener("click",function(e){if(!e.isTrusted)return;var b=e.target&&e.target.closest?e.target.closest("[data-zeron-pick]"):null;if(!b)return;e.preventDefault();var gr=g(b),c=b.getAttribute("data-zeron-pick"),r=b.getBoundingClientRect(),at=document.elementFromPoint(r.left+r.width/2,r.top+r.height/2);send({pick:{group:gr,code:c,label:label(b),groups:groups(),rect:[r.left,r.top,r.width,r.height],covered:!(at&&(b===at||b.contains(at)))}});},true);document.addEventListener("DOMContentLoaded",function(){apply();send({ready:groups()});});var css=document.createElement("style");css.id="zeron-picks";css.textContent="[data-zeron-pick]{cursor:pointer}[data-zeron-pick][data-zeron-chosen]{box-shadow:inset 0 0 0 1.5px var(--zeron-pick,var(--accent))}[data-zeron-pick][data-zeron-locked]{cursor:default}[data-zeron-pick][data-zeron-locked]:not([data-zeron-chosen]){opacity:.45}";if(s&&s.parentNode){s.parentNode.insertBefore(css,s);s.remove();}else{(document.head||document.documentElement).appendChild(css);}})();"#;

/// The page script for a nonce.
pub fn pick_script(nonce: &str) -> String {
    PICK_SCRIPT.replace("__NONCE__", nonce)
}

/// Pages read, by where they are (`device\0path`), as the agent wrote them.
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

/// The page theme from the roles ds/html_card reads from the theme
/// (`#rrggbbaa` each), its flags and its fonts.
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

/// The page with its bootstrap, signed with `nonce`; the pick colour as a
/// variable the page may use (`--zeron-pick`).
pub fn themed(html: &str, theme: &PageTheme, font_size: f32, pick: Option<&str>, nonce: &str) -> String {
    let mut extra = pick_script(nonce);
    if let Some(pick) = pick.and_then(Rgba::hex) {
        extra.push_str(&format!(
            "document.documentElement.style.setProperty(\"--zeron-pick\",\"{}\");",
            shared::css_color(pick)
        ));
    }
    shared::themed_page(html, theme, font_size, true, &extra)
}

fn nonce() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
    format!("{:016x}", h.finish())
}

/// The picks a page shows, as the script that puts them on its buttons.
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

/// Whether something accepts connections at a live URL's loopback address
/// (the shipping app's `server_answers`).
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

/// A stored page, through the engine, a chunk at a time.
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

/// A page by its key: kept, else read (`source` stands in for the file: a
/// fixture's).
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
            let nonce = nonce();
            let out = themed(&html, &theme, font, pick.as_deref(), &nonce);
            Ok(Value::record([("html", Value::from(out.as_str())), ("nonce", Value::from(nonce.as_str()))]))
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
                Value::record([("kind", Value::from("zeron.page")), ("nonce", Value::from(format!("n:{path}").as_str())), ("pick", pick)]),
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
        // (Past the landed slot's 700 ms light.)
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
        let out = themed("<p>x</p>", &theme, 16., Some("#c0347a"), "n0");
        let shared_page = shared::themed_page("<p>x</p>", &theme, 16., true, "");
        let (head, _) = shared_page.split_once("</script>").unwrap();
        assert!(out.starts_with(head), "the shared bootstrap comes first");
        assert!(out.contains("N=\"n0\"") && out.contains("e.isTrusted"));
        assert!(out.contains("--zeron-pick\",\"rgba(192,52,122,1)\""));
        assert!(out.ends_with("</head><p>x</p>"));
    }

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
