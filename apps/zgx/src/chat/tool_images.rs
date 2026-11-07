//! The pictures a tool read, wrote or printed, for its row opened (gpui's `tool_images::image_paths`).

use serde_json::Value as Json;

const MAX_IMAGES_PER_TOOL: usize = 4;
const IMAGE_EXTENSIONS: [&str; 9] = ["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "tif", "tiff"];

fn is_image_path(path: &str) -> bool {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    name.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && IMAGE_EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
}

// Opening a UNC or device path can reach another host.
fn is_remote_or_device_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    let separator = |b: Option<&u8>| matches!(b, Some(b'/' | b'\\'));
    (separator(bytes.first()) && separator(bytes.get(1))) || path.starts_with(r"\??\") || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && !separator(bytes.get(2)))
}

fn push_path(out: &mut Vec<String>, path: &str) {
    if out.len() < MAX_IMAGES_PER_TOOL && is_image_path(path) && !is_remote_or_device_path(path) && !out.iter().any(|known| known == path) {
        out.push(path.to_owned());
    }
}

fn push_text_paths(out: &mut Vec<String>, text: &str) {
    let separators = |c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '<' | '>' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' | '|' | '=');
    for token in text.split(separators) {
        if out.len() >= MAX_IMAGES_PER_TOOL {
            return;
        }
        let token = token.trim_end_matches(['.', ':', '!', '?']);
        let token = token.strip_prefix("file://").unwrap_or(token);
        if token.contains("://") || token.starts_with("data:") || token.contains(['*', '?']) {
            continue;
        }
        push_path(out, token);
    }
}

fn push_json_paths(out: &mut Vec<String>, value: &Json) {
    match value {
        Json::String(text) => push_text_paths(out, text),
        Json::Array(items) => items.iter().for_each(|v| push_json_paths(out, v)),
        Json::Object(map) => map.values().for_each(|v| push_json_paths(out, v)),
        _ => {}
    }
}

fn joined(path: &str, cwd: Option<&str>) -> String {
    let absolute = path.starts_with(['/', '\\']) || (path.len() >= 3 && path.as_bytes()[0].is_ascii_alphabetic() && path.as_bytes()[1] == b':');
    match cwd.filter(|_| !absolute) {
        Some(cwd) if cwd.contains('\\') => format!("{}\\{}", cwd.trim_end_matches(['/', '\\']), path.trim_start_matches(".\\")),
        Some(cwd) => format!("{}/{}", cwd.trim_end_matches('/'), path.trim_start_matches("./")),
        None => path.to_owned(),
    }
}

/// A tool part's pictures, each on its chat's host (relative ones under `cwd`).
pub(crate) fn image_paths(part: &Json, cwd: Option<&str>) -> Vec<String> {
    let call = &part["call"];
    let mut out = Vec::new();
    match call["kind"].as_str().unwrap_or_default() {
        "readFile" | "writeFile" | "editFile" | "applyPatch" => {
            if let Some(path) = call["path"].as_str() {
                push_path(&mut out, path);
            }
        }
        "exec" => push_text_paths(&mut out, call["command"].as_str().unwrap_or_default()),
        "mcp" | "unknown" => push_json_paths(&mut out, &call["input"]),
        _ => {}
    }
    if let Some(output) = part["output"].as_str() {
        push_text_paths(&mut out, output);
    }
    out.iter().map(|p| joined(p, cwd)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_tools_pictures_are_the_image_files_it_touched_at_most_four() {
        let read = json!({ "call": { "kind": "readFile", "path": "shots/a.png" } });
        assert_eq!(image_paths(&read, Some("/w")), ["/w/shots/a.png"]);
        let exec = json!({ "call": { "kind": "exec", "command": "convert in.jpg out.webp && open https://x.dev/y.png" }, "output": "wrote /tmp/z.PNG. and *.png, \\\\host\\s\\h.png" });
        assert_eq!(image_paths(&exec, Some("/w")), ["/w/in.jpg", "/w/out.webp", "/tmp/z.PNG"]);
        let mcp = json!({ "call": { "kind": "mcp", "input": { "paths": ["a.gif", "b.gif", "c.gif", "d.gif", "e.gif"] } } });
        assert_eq!(image_paths(&mcp, None).len(), 4);
        assert!(image_paths(&json!({ "call": { "kind": "readFile", "path": "notes.md" } }), None).is_empty());
    }
}
