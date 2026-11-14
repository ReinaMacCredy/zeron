mod build;
mod preview;
mod synctex;

use std::path::{Path, PathBuf};

use zgx_script::{ScriptHost, Value};

pub fn install(host: &ScriptHost) {
    host.async_action("tex.compile", |arg| async move {
        let path = path_of(&arg, "path")?;
        let cache = crate::pdf::app_cache().join("tex");
        let done = crate::favicons::off_thread(move || preview::compile(&path, &cache, tectonic())).await.ok_or("The compile stopped")?;
        Ok(value(&done))
    });
    host.async_action("tex.sync", |arg| async move {
        let pdf = path_of(&arg, "pdf")?;
        let at = |key: &str| arg.field(key).and_then(Value::as_f64).filter(|n| n.is_finite());
        let page = at("page").filter(|p| *p >= 1.).ok_or("no page")? as u32;
        let from = (at("x").ok_or("no x")?, at("y").ok_or("no y")?);
        let to = at("x2").zip(at("y2"));
        let found = crate::favicons::off_thread(move || preview::sync(&pdf, page, from, to)).await.ok_or("The lookup stopped")??;
        Ok(Value::record([
            ("file", Value::from(found.file.to_string_lossy().as_ref())),
            ("line", Value::from(found.line as f64)),
            ("end_line", Value::from(found.end as f64)),
            ("text", Value::from(found.text.as_str())),
        ]))
    });
}

fn path_of(arg: &Value, key: &str) -> Result<PathBuf, String> {
    let path = arg.field(key).map(crate::auth::text).unwrap_or_default();
    if path.is_empty() || !Path::new(&path).is_absolute() {
        return Err("That file isn't on this computer".into());
    }
    Ok(synctex::normalized(Path::new(&path)))
}

fn tectonic() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("ZERON_TECTONIC") {
        return Some(PathBuf::from(path));
    }
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.parent()?.join("Resources").join("tectonic").join("tectonic"))
}

fn value(done: &preview::Done) -> Value {
    let path = |p: &Option<PathBuf>| p.as_ref().map_or(Value::Nil, |p| Value::from(p.to_string_lossy().as_ref()));
    let problems: Vec<Value> = done
        .problems
        .iter()
        .map(|p| {
            Value::record([
                ("message", Value::from(p.message.as_str())),
                ("line", p.line.map_or(Value::Nil, |n| Value::from(n as f64))),
                ("file", path(&p.file)),
            ])
        })
        .collect();
    Value::record([
        ("state", Value::from(done.state)),
        ("pdf", path(&done.pdf)),
        ("tool", Value::from(done.tool)),
        ("root", path(&done.root)),
        ("problems", Value::List(problems.into())),
    ])
}
