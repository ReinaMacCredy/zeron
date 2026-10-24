use zeron_doc::parts::MessagePart;
use zeron_proto::ToolCall;
use zeron_proto::html_render::HtmlRenderRef;

pub(crate) const PATH: &str = "/Users/dev/.zeron/html/chat-visuals/onboarding.html";
const TITLE: &str = "Onboarding card, three treatments";
const HEIGHT: u32 = 520;

pub(crate) fn read(path: &str, large: bool) -> Option<Vec<u8>> {
    if path != PATH {
        return None;
    }
    if !large {
        return Some(PAGE.as_bytes().to_vec());
    }
    let shots: String = (0..2)
        .map(|seed| {
            format!(
                r#"<img style="display:block;width:100%;margin-top:12px;border-radius:var(--radius)" src="data:image/png;base64,{}">"#,
                crate::live::b64::encode(&super::png::gradient(900, 560, seed))
            )
        })
        .collect();
    Some(
        PAGE.replace("</body>", &format!("{shots}\n</body>"))
            .into_bytes(),
    )
}

pub(crate) fn part() -> MessagePart {
    let mut part = super::transcripts::tool(
        "h1",
        ToolCall::Mcp {
            server: "zeron".into(),
            tool: "html_render".into(),
            input: Some(serde_json::json!({ "title": TITLE, "height": HEIGHT })),
        },
        false,
        Some(zeron_proto::html_render::RENDERED_MESSAGE),
    );
    if let MessagePart::Tool { html_render, .. } = &mut part {
        *html_render = Some(HtmlRenderRef {
            path: PATH.into(),
            title: TITLE.into(),
            height: HEIGHT,
        });
    }
    part
}

const PAGE: &str = r#"<!doctype html>
<html><head><title>Onboarding</title>
<style>
.grid{display:flex;flex-wrap:wrap;gap:12px}
.card{flex:1 1 200px;border:1px solid var(--border);border-radius:var(--radius);padding:14px;background:var(--card);color:var(--card-foreground)}
.tag{font-size:var(--font-size-xs);color:var(--muted-foreground);text-transform:uppercase;letter-spacing:.06em}
h3{margin:6px 0 4px;font-size:var(--font-size-lg)}
p{margin:0 0 10px;color:var(--muted-foreground);font-size:var(--font-size-sm)}
.bar{height:6px;border-radius:3px;background:var(--muted);overflow:hidden;margin-bottom:12px}
.bar i{display:block;height:100%;width:40%;background:var(--chart-1);transition:width .3s}
button{font:inherit;font-size:var(--font-size-sm);border-radius:999px;padding:7px 14px;border:0;cursor:pointer}
.solid{background:var(--primary);color:var(--primary-foreground)}
.soft{background:var(--accent-surface);color:var(--accent-surface-foreground)}
.ghost{background:transparent;color:var(--foreground);border:1px solid var(--border-strong)}
.row{display:flex;gap:8px;align-items:center}
.dot{width:8px;height:8px;border-radius:50%;background:var(--success)}
</style></head>
<body>
<div class="grid">
  <div class="card"><div class="tag">A · Progress</div><h3>Connect your Mac</h3><p>Two of five steps done.</p>
    <div class="bar"><i id="bar"></i></div><div class="row"><button class="solid" id="next">Next step</button><button class="ghost">Later</button></div></div>
  <div class="card"><div class="tag">B · Checklist</div><h3>Finish setup</h3>
    <p><span class="row"><span class="dot"></span>Signed in</span></p><p><span class="row"><span class="dot"></span>Host paired</span></p>
    <button class="soft">Pair a second host</button></div>
  <div class="card"><div class="tag">C · Minimal</div><h3>You're almost there</h3><p>Pair a host to run agents from your phone.</p><button class="solid">Pair host</button></div>
</div>
<script>
var w=40;document.getElementById('next').onclick=function(){w=w>=100?20:w+20;document.getElementById('bar').style.width=w+'%';};
</script>
</body></html>"#;
