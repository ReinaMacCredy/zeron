use gpui::{AsyncApp, WindowHandle};
use serde_json::{Value, json};
use zeron_proto::computer::CuaReply;
use zeron_ui::shell::Shell;

async fn call(
    window: WindowHandle<Shell>,
    args: Value,
    developer: bool,
    cx: &mut AsyncApp,
) -> anyhow::Result<CuaReply> {
    let reply = window.update(cx, |shell, w, cx| {
        shell.fixture_native_browser_call(args, developer, false, None, w, cx)
    })?;
    Ok(reply.await?)
}
fn value(reply: CuaReply) -> anyhow::Result<Value> {
    let result = reply.result.ok_or_else(|| anyhow::anyhow!("no result"))?;
    anyhow::ensure!(result.is_error != Some(true), "{:?}", result.content);
    Ok(Value::Object(result.structured_content.unwrap_or_default()))
}
fn action(tab: &Value, action: &str) -> Value {
    json!({"action":action,"tab_id":tab["tab_id"],"control_token":tab["control_token"]})
}

pub async fn exercise(
    window: WindowHandle<Shell>,
    origin: &str,
    output: &std::path::Path,
    cx: &mut AsyncApp,
) -> anyhow::Result<()> {
    let tab = value(
        call(
            window,
            json!({"action":"open","url":format!("{origin}/control")}),
            false,
            cx,
        )
        .await?,
    )?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let state = loop {
        let reply = call(window, action(&tab, "state"), false, cx).await?;
        if let Ok(state) = value(reply) {
            if state["title"] == "Browser control fixture" {
                break state;
            }
        }
        anyhow::ensure!(std::time::Instant::now() < deadline, "page failed to load");
        super::pause(cx, 100).await;
    };
    let elements = state["elements"].as_array().unwrap();
    let input = elements.iter().find(|e| e["label"] == "Name").unwrap()["ref"].clone();
    let submit = elements.iter().find(|e| e["label"] == "Submit").unwrap()["ref"].clone();
    let mut typed = action(&tab, "type");
    typed["ref"] = input.clone();
    typed["text"] = json!("Reina");
    value(call(window, typed, false, cx).await?)?;
    let mut click = action(&tab, "click");
    click["ref"] = submit;
    value(call(window, click, false, cx).await?)?;
    let state = value(call(window, action(&tab, "state"), false, cx).await?)?;
    anyhow::ensure!(
        state["text"].as_str().unwrap().contains("Hello Reina"),
        "form outcome missing"
    );
    let mut stale = action(&tab, "type");
    stale["ref"] = input;
    stale["text"] = json!("Wrong");
    anyhow::ensure!(
        call(window, stale, false, cx)
            .await?
            .result
            .unwrap()
            .is_error
            == Some(true),
        "stale ref accepted"
    );
    let mut eval = action(&tab, "evaluate");
    eval["script"] = json!("Promise.resolve(document.querySelector('output').textContent)");
    anyhow::ensure!(
        call(window, eval.clone(), false, cx)
            .await?
            .result
            .unwrap()
            .is_error
            == Some(true),
        "developer gate failed"
    );
    anyhow::ensure!(
        value(call(window, eval.clone(), true, cx).await?)?["value"] == "Hello Reina",
        "JS readback mismatch"
    );
    value(call(window, action(&tab, "diagnostics"), true, cx).await?)?;
    let mut log = action(&tab, "evaluate");
    log["script"] = json!("console.warn('fixture-log'); fetch('/ping'); 'sent'");
    value(call(window, log, true, cx).await?)?;
    super::pause(cx, 500).await;
    let diagnostics = value(call(window, action(&tab, "diagnostics"), true, cx).await?)?;
    anyhow::ensure!(
        diagnostics.to_string().contains("fixture-log")
            && diagnostics.to_string().contains("fetch"),
        "diagnostics missing"
    );
    let image = call(window, action(&tab, "screenshot"), false, cx)
        .await?
        .result
        .unwrap()
        .content
        .unwrap()
        .into_iter()
        .find(|part| part.kind == "image")
        .unwrap();
    use base64::Engine;
    std::fs::write(
        output.join("native-control-webkit.png"),
        base64::engine::general_purpose::STANDARD.decode(image.data.unwrap())?,
    )?;
    window.update(cx, |shell, _, cx| shell.fixture_browser_takeover(cx))?;
    anyhow::ensure!(
        call(window, eval.clone(), true, cx)
            .await?
            .result
            .unwrap()
            .is_error
            == Some(true),
        "takeover failed"
    );
    window.update(cx, |shell, _, cx| {
        let (_, browser) = shell.fixture_active_browser(cx).unwrap();
        browser
            .read(cx)
            .fixture_eval("console.warn('manual-private-interval')");
    })?;
    window.update(cx, |shell, _, cx| shell.fixture_browser_resume(cx))?;
    anyhow::ensure!(
        call(window, eval, true, cx).await?.result.unwrap().is_error == Some(true),
        "old token survived resume"
    );
    let tabs = value(call(window, json!({"action":"list"}), false, cx).await?)?;
    let fresh = tabs["tabs"][0].clone();
    let after_takeover = value(call(window, action(&fresh, "diagnostics"), true, cx).await?)?;
    anyhow::ensure!(
        !after_takeover
            .to_string()
            .contains("manual-private-interval"),
        "takeover kept collecting diagnostics"
    );
    let before_route = value(call(window, action(&fresh, "state"), false, cx).await?)?;
    let mut route = action(&fresh, "evaluate");
    route["script"] = json!("var oldUrl = location.href; history.pushState({}, '', '#new-route'); history.replaceState({}, '', oldUrl); null");
    value(call(window, route, true, cx).await?)?;
    let mut old_route = action(&fresh, "type");
    old_route["ref"] = before_route["elements"][0]["ref"].clone();
    old_route["text"] = json!("Wrong route");
    anyhow::ensure!(
        call(window, old_route, false, cx)
            .await?
            .result
            .unwrap()
            .is_error
            == Some(true),
        "route change preserved old refs"
    );

    let foreign = window
        .update(cx, |shell, w, cx| {
            shell.fixture_native_browser_call(
                action(&fresh, "state"),
                false,
                false,
                Some("other-task".into()),
                w,
                cx,
            )
        })?
        .await?;
    anyhow::ensure!(
        foreign.result.unwrap().is_error == Some(true),
        "cross task access allowed"
    );
    let cancelled = window
        .update(cx, |shell, w, cx| {
            shell.fixture_native_browser_call(action(&fresh, "state"), false, true, None, w, cx)
        })?
        .await?;
    anyhow::ensure!(
        cancelled.result.unwrap().is_error == Some(true),
        "cancelled command accepted"
    );
    let other = value(call(window, json!({"action":"open","url":origin}), false, cx).await?)?;
    anyhow::ensure!(other["tab_id"] != fresh["tab_id"], "tab identity reused");
    let still = value(call(window, action(&fresh, "state"), false, cx).await?)?;
    anyhow::ensure!(
        still["text"].as_str().unwrap().contains("Hello Reina"),
        "selected tab retargeted action"
    );
    std::fs::write(
        output.join("native-control-report.json"),
        serde_json::to_vec_pretty(
            &json!({"form":state,"diagnostics":diagnostics,"takeover":true,"stale_ref":true,"cross_task_refused":true,"cancelled_refused":true,"selection_keeps_target":true}),
        )?,
    )?;
    Ok(())
}
