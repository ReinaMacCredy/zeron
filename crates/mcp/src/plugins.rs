use std::collections::{BTreeSet, HashSet};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc;
use zeron_proto::tools::{ToolGroup, ZERON_TOOL_PREFIX, group_live};
use zeron_rpc::{RpcError, methods};

use crate::ThreadTools;

pub(crate) const CREATE_PLUGIN: &str = "create_plugin";
pub(crate) const LIST_PLUGINS: &str = "list_plugins";
pub(crate) const LIST_TEMPLATES: &str = "list_templates";
pub(crate) const CREATE_TEMPLATE: &str = "create_template";
pub(crate) const USE_TEMPLATE: &str = "use_template";
pub(crate) const TEMPLATE_STATUS: &str = "template_status";
pub(crate) const DELETE_TEMPLATE: &str = "delete_template";
pub(crate) const DESCRIBE_APP: &str = "describe_app";
pub(crate) const EXPORT_TEMPLATE: &str = "export_template";
pub(crate) const IMPORT_TEMPLATE: &str = "import_template";
pub(crate) const DESCRIBE_ELEMENTS: &str = "describe_elements";
pub(crate) const PLUGIN_STATUS: &str = "plugin_status";
pub(crate) const INSTALL_PLUGIN: &str = "install_plugin";
pub(crate) const UPDATE_PLUGIN: &str = "update_plugin";

const LIST_TIMEOUT: Duration = Duration::from_secs(5);
const FIRST_RETRY: Duration = Duration::from_millis(300);
const MAX_RETRY: Duration = Duration::from_secs(30);

pub(crate) fn list_changed_notification() -> Value {
    json!({ "jsonrpc": "2.0", "method": "notifications/tools/list_changed" })
}

fn plugin_tool_names(reply: &Value) -> BTreeSet<String> {
    reply["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|tool| tool["mcp_name"].as_str().map(str::to_owned))
        .collect()
}

fn listed_plugin_tools(reply: &Value) -> Vec<Value> {
    let device = match reply["device"]["name"].as_str().unwrap_or("").trim() {
        "" => "this device",
        name => name,
    };
    reply["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|tool| tool["mcp_name"].is_string())
        .map(|tool| {
            let plugin = match tool["plugin_name"].as_str().unwrap_or("").trim() {
                "" => tool["plugin"].as_str().unwrap_or(""),
                name => name,
            };
            let origin =
                format!("(From the {plugin} plugin. Works while Zeron is open on {device}.)");
            let description = match tool["description"].as_str().unwrap_or("").trim() {
                "" => origin,
                text => format!("{text} {origin}"),
            };
            json!({
                "name": tool["mcp_name"],
                "description": description,
                "inputSchema": input_schema(&tool["input"]),
            })
        })
        .collect()
}

fn input_schema(input: &Value) -> Value {
    match input {
        Value::Object(schema) => {
            let mut schema = schema.clone();
            schema
                .entry("type")
                .or_insert_with(|| Value::String("object".into()));
            Value::Object(schema)
        }
        _ => json!({ "type": "object", "properties": {} }),
    }
}

pub(crate) fn is_plugin_tool_name(name: &str) -> bool {
    let word = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    };
    !name.starts_with(ZERON_TOOL_PREFIX)
        && name
            .split_once("__")
            .is_some_and(|(plugin, tool)| word(plugin) && word(tool))
}

/// A plugin's own tools have no switch of their own: they follow the group's.
pub(crate) fn switched_off(name: &str, disabled: &HashSet<String>) -> bool {
    is_plugin_tool_name(name)
        && !group_live(
            ToolGroup::Plugins,
            &disabled.iter().map(String::as_str).collect(),
        )
}

fn plain(error: RpcError) -> String {
    match error {
        RpcError::Failed(message) => message,
        RpcError::UnknownMethod(_) => {
            "This Zeron doesn't run plugins yet; update Zeron to use them.".into()
        }
        other => other.to_string(),
    }
}

fn given<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args[key].as_str().filter(|value| !value.trim().is_empty())
}

fn required<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args[key]
        .as_str()
        .ok_or_else(|| format!("invalid arguments: missing field `{key}`"))
}

fn template_id(args: &Value) -> Result<&str, String> {
    given(args, "id").ok_or_else(|| "Say which template: `id`, from list_templates.".to_owned())
}

pub(crate) fn tool(name: &str, description: &str, mut input_schema: Value) -> Value {
    input_schema["additionalProperties"] = json!(false);
    json!({ "name": name, "description": description, "inputSchema": input_schema })
}

pub(crate) fn tools() -> Vec<Value> {
    vec![
        tool(
            CREATE_PLUGIN,
            "Create a Zeron plugin on this device: a folder with a manifest (zeron-plugin.json), init.luau to edit, and zeron.luau, the SDK. Zeron signs it as yours and loads it, and re-signs and reloads it as you save; the person allows once, in Zeron, any permissions it asks for. `id` is lowercase letters, digits and dashes (2-40). `permissions` are what the plugin may do (ui.add, ui.change, ui.replace, profile.read, sessions.read, messages.read, account.email, files.read, panel.tabs, pages.add, github, github.write, navigate, compose, draft, network, clipboard, files, web, windows, tools, devices, native, commands: with a manifest `commands` list of programs it runs, as you). The reply links the authoring guide (`guide`): read it before writing.",
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "pattern": "^[a-z0-9][a-z0-9-]{1,39}$", "description": "Folder and manifest id, e.g. focus." },
                    "name": { "type": "string", "description": "Display name, e.g. Focus." },
                    "description": { "type": "string" },
                    "permissions": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["id", "name"]
            }),
        ),
        tool(
            LIST_PLUGINS,
            "Zeron plugins on this device: id, name, status (on, off, pending review, tampered, broken), who made it, and its folder.",
            json!({ "type": "object", "properties": {} }),
        ),
        tool(
            PLUGIN_STATUS,
            "Where a Zeron plugin's UI landed, to check your work without a person: its status (on, off, pending review, tampered, broken, and why), what it was allowed, and, from Zeron's window on this device, each zeron.place it made (its selector, how, how many elements it matched and their boxes in the window), what its row in Settings says (a node that needs what its place doesn't give, places this template doesn't have, an error that stopped it) and its pages. With snapshot: true, also a PNG of the window cut to around its nodes (the whole window when none shows): read the file at `snapshot` to look. Edit the plugin, save, and ask again.",
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "The plugin's id, from list_plugins." },
                    "snapshot": { "type": "boolean", "description": "Also save a picture of where it shows." }
                },
                "required": ["id"]
            }),
        ),
        tool(
            INSTALL_PLUGIN,
            "Install someone's Zeron plugin from a .zeron-plugin file (`path`) or a link (`url`): a link to a .zeron-plugin file, a GitHub release (…/releases/latest or …/releases/tag/<tag>, its .zeron-plugin asset), or a git repository (owner/repo, its GitHub page, …/tree/<branch>/<folder>, or any https git URL; `ref` a branch or tag, `folder` where the plugin is in it). Zeron checks the signature over every file first: an unsigned or changed plugin is refused. Whatever you install waits for the person to allow it in Zeron's window, which shows who signed it and what it asks: you can't allow it for them (`waiting` says so). Installing over a plugin already installed is an update, and only when the same key signed both.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "A .zeron-plugin file's full path." },
                    "url": { "type": "string", "description": "A package link, a GitHub release, or a git repository (https)." },
                    "ref": { "type": "string", "description": "With a git repository: the branch or tag (default its default branch)." },
                    "folder": { "type": "string", "description": "With a git repository: the plugin's folder in it (default the top)." }
                }
            }),
        ),
        tool(
            UPDATE_PLUGIN,
            "Check whether a plugin installed from someone else has a newer version at the source it came from (its file, link, release or repository): its version and what it changes in what the plugin asks (`added`, `removed`); without an id, check every installed plugin. Installing an update is the person's, in Zeron (Settings → Plugins): you can't update it for them, so tell them what you found.",
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "The plugin's id, from list_plugins; without one, every installed plugin." },
                    "check": { "type": "boolean", "description": "Kept for older callers: this tool only ever checks." }
                }
            }),
        ),
        tool(
            DESCRIBE_ELEMENTS,
            "Where a Zeron plugin can place its nodes: Zeron's named elements, each with its selector (#id or .class), where it is in words, what it is, what it provides to what's placed in it (session, message, file, project, panel; everything has profile, appearance and route), whether the window running now has it (a template may rename or leave out elements), and whether it's stable (kept across Zeron's updates and by every template: place by those where you can). Place with zeron.place { at = \"<selector>\", before | after | replace = function() return <node> end }; selectors also take `#a >> .b` (inside) and role:button \"Name\". In the built node read what's provided with zeron.context(name) (nil where absent) or zeron.need(name) (it draws nothing there, and Settings says why). Records hold only what the plugin was allowed: sessions.read, messages.read, profile.read, account.email, files.read (without it a file is an id of the plugin's own, never its path). The reply links the plugin authoring guide (`guide`).",
            json!({ "type": "object", "properties": {} }),
        ),
        tool(
            LIST_TEMPLATES,
            "Zeron's templates on this device. A template is a whole copy of Zeron's interface (its Luau app: app.luau, screens/, ds/, assets/) that you can rewrite; it inherits Zeron's own for any file it lacks. Each one's id, name, folder, whether it's the one in use, and its status: ok, errors (some files fall back to Zeron's own; template_status says why), out_of_date (made for another version of Zeron: Zeron has updated since; template_status names the files whose Zeron original changed) or broken. Zeron's own comes first. One installed from someone's package (import_template) has `imported` (who signed it); packages waiting for the person to allow them are under `pending`.",
            json!({ "type": "object", "properties": {} }),
        ),
        tool(
            CREATE_TEMPLATE,
            "Make a new template on this device: a whole copy of Zeron's interface (or of another template, with `from`) in its own folder, to rewrite. Returns its folder, its files and a short map of the app. Delete any file to inherit Zeron's own. While Zeron is open and the template is in use (use_template), each edit shows live as you save it. A file with an error falls back to Zeron's own for that file, and template_status says why. `id` is lowercase letters, digits and dashes (2-40). The reply links the authoring guide (`guide`): read it before writing.",
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "pattern": "^[a-z0-9][a-z0-9-]{1,39}$", "description": "The template's id and folder name, e.g. midnight." },
                    "name": { "type": "string", "description": "Its display name, e.g. Midnight." },
                    "from": { "type": "string", "description": "A template to start from, from list_templates (default zeron, Zeron's own)." }
                },
                "required": ["id", "name"]
            }),
        ),
        tool(
            USE_TEMPLATE,
            "Run Zeron's window with a template: it switches live while Zeron is open. `zeron` is Zeron's own interface. Only templates installed on this device can be used. Unless the person has kept this template in Zeron, it runs as a preview (`preview`): Zeron shows them a bar to Keep or Revert it, and holds back what only a kept template may do until they keep it. You can't keep it for them; ask them to look.",
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "A template's id, from list_templates, or zeron." }
                },
                "required": ["id"]
            }),
        ),
        tool(
            TEMPLATE_STATUS,
            "How a template is doing: whether it's in use, the error that broke it (root_error), the files with errors (each falls back to Zeron's own, with the error), the files out of date (Zeron's own has changed since they were copied), how many files it has, and how many it inherits from Zeron.",
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "A template's id, from list_templates." }
                },
                "required": ["id"]
            }),
        ),
        tool(
            DELETE_TEMPLATE,
            "Delete a template: its folder is set aside in the templates folder's .removed. Deleting the one in use switches the window back to Zeron's own. Zeron's own can't be deleted.",
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "A template's id, from list_templates." }
                },
                "required": ["id"]
            }),
        ),
        tool(
            DESCRIBE_APP,
            "What Zeron's interface is made of, to find your way in a template: its screens (each one's module, file, what it draws, and the elements it names: #ids and .classes, which plugins place by; keep them to keep plugins working) and a short map of the app's folders. For where plugins can place in the window running now, use describe_elements. The reply links the template authoring guide (`guide`).",
            json!({ "type": "object", "properties": {} }),
        ),
        tool(
            EXPORT_TEMPLATE,
            "Share a template: save it as one signed .zeron-template file, signed with this device's key (the person's Zeron account vouches for it, as for their plugins). It holds the template's own files and its preview; the files no template may replace (screens/plugins, screens/templates, account/, .luaurc) are left out. Zeron's own can't be exported. It's saved in Zeron's exports folder (an earlier export of it there is replaced). Returns the file's path, how many files it holds and the key that signed it.",
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "A template's id, from list_templates." }
                },
                "required": ["id"]
            }),
        ),
        tool(
            IMPORT_TEMPLATE,
            "Install a template from a .zeron-template file (`path`) or a link (`url`: to a .zeron-template file, or a GitHub repository or release with one; a template from a link is checked there for updates). Zeron checks its signature over every file first: an unsigned or changed package is refused. One signed on this device installs at once (using it still runs as a preview until the person keeps it). Anyone else's waits for the person: a template runs as Zeron's own code with all its powers, so they must allow it in Zeron's window (Templates shows it waiting, with who signed it); you can't allow it for them. The reply says which: `installed` (its id), or `pending` with `needs_consent`. Files no template may replace are never installed (`ignored` names them). list_templates shows it once installed.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "The .zeron-template file's full path." },
                    "url": { "type": "string", "description": "Or a link: a .zeron-template file's, or a GitHub repository or release (e.g. owner/repo) with one." }
                }
            }),
        ),
    ]
}

impl ThreadTools {
    async fn plugin_rpc(&self, method: &str, params: Value) -> Result<Value, String> {
        self.client.call(method, params).await.map_err(plain)
    }

    pub(crate) async fn add_plugin_tools(&self, list: &mut Vec<Value>) {
        let reply = tokio::time::timeout(
            LIST_TIMEOUT,
            self.client.call(methods::LIST_PLUGIN_TOOLS, json!({})),
        )
        .await;
        let Ok(Ok(reply)) = reply else {
            return;
        };
        self.advertise_plugin_tools(plugin_tool_names(&reply));
        for tool in listed_plugin_tools(&reply) {
            if !list.iter().any(|known| known["name"] == tool["name"]) {
                list.push(tool);
            }
        }
    }

    fn advertise_plugin_tools(&self, names: BTreeSet<String>) -> bool {
        let mut advertised = self.advertised_plugin_tools.lock().unwrap();
        advertised
            .replace(names.clone())
            .is_some_and(|before| before != names)
    }

    pub(crate) async fn watch_plugin_tools(&self, changed: mpsc::Sender<()>) {
        let mut retry = FIRST_RETRY;
        loop {
            match self
                .client
                .subscribe_checked(methods::WATCH_PLUGIN_TOOLS, json!({}))
                .await
            {
                Ok(mut stream) => {
                    retry = FIRST_RETRY;
                    while let Some(item) = stream.recv().await {
                        if self.advertise_plugin_tools(plugin_tool_names(&item))
                            && changed.send(()).await.is_err()
                        {
                            return;
                        }
                    }
                }
                Err(RpcError::UnknownMethod(_)) => return,
                Err(_) => {}
            }
            if changed.is_closed() {
                return;
            }
            tokio::time::sleep(retry).await;
            retry = (retry * 2).min(MAX_RETRY);
        }
    }

    pub(crate) async fn call_plugin_tool(&self, name: &str, args: &Value) -> Result<Value, String> {
        let reply = self
            .plugin_rpc(
                methods::CALL_PLUGIN_TOOL,
                json!({ "mcp_name": name, "args": args, "chatId": self.chat_id }),
            )
            .await?;
        if reply["ok"] == false {
            return Err(reply["error"]
                .as_str()
                .unwrap_or("The plugin tool failed")
                .to_owned());
        }
        Ok(reply.get("value").cloned().unwrap_or(Value::Null))
    }

    pub(crate) async fn create_plugin(&self, args: &Value) -> Result<Value, String> {
        let mut params = json!({ "id": required(args, "id")?, "name": required(args, "name")? });
        if let Some(description) = args["description"].as_str() {
            params["description"] = json!(description);
        }
        if args["permissions"].is_array() {
            params["permissions"] = args["permissions"].clone();
        }
        let item = self.plugin_rpc(methods::NEW_PLUGIN, params).await?;
        let id = item["id"].as_str().unwrap_or_default();
        let dir = item["dir"].as_str().unwrap_or_default();
        let mut reply = json!({
            "id": id,
            "dir": dir,
            "files": scaffold_files(dir).await,
            "status": item.get("status"),
            "hint": format!(
                "Edit {dir}/init.luau; Zeron re-signs and reloads it as you save. Use {dir}/zeron.luau's helpers."
            ),
        });
        if let Some(guide) = item["guide"].as_str() {
            reply["guide"] = json!(guide);
            reply["guide_summary"] = item["guide_summary"].clone();
            reply["hint"] = json!(format!(
                "Read {guide} first (how to write a plugin, with examples). Then edit {dir}/init.luau; Zeron re-signs and reloads it as you save. Check it with plugin_status (snapshot: true)."
            ));
        }
        Ok(reply)
    }

    pub(crate) async fn list_plugins(&self) -> Result<Value, String> {
        let reply = self.plugin_rpc(methods::LIST_PLUGINS, json!({})).await?;
        let plugins = reply["plugins"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .map(|p| {
                let mut out = json!({
                    "id": p.get("id"),
                    "name": p.get("name"),
                    "status": p.get("status"),
                    "author": p.get("author"),
                    "dir": p.get("dir"),
                });
                for key in ["source", "update"] {
                    if let Some(value) = p.get(key).filter(|v| !v.is_null()) {
                        out[key] = value.clone();
                    }
                }
                out
            })
            .collect::<Vec<_>>();
        Ok(json!({ "plugins": plugins }))
    }

    pub(crate) async fn plugin_status(&self, args: &Value) -> Result<Value, String> {
        let params = json!({
            "id": required(args, "id")?,
            "snapshot": args["snapshot"].as_bool().unwrap_or(false),
        });
        self.plugin_rpc(methods::PLUGIN_STATUS, params).await
    }

    pub(crate) async fn install_plugin(&self, args: &Value) -> Result<Value, String> {
        let params = match (given(args, "path"), given(args, "url")) {
            (Some(path), None) => json!({ "path": path }),
            (None, Some(url)) => {
                json!({ "url": url, "ref": given(args, "ref"), "folder": given(args, "folder") })
            }
            _ => {
                return Err(
                    "Give one of `path` (a .zeron-plugin file) or `url` (a link or repository)."
                        .into(),
                );
            }
        };
        let reply = self.plugin_rpc(methods::INSTALL_PLUGIN, params).await?;
        Ok(installed(reply))
    }

    /// Only ever checks: installing an update is the person's, and an agent's token can't.
    pub(crate) async fn update_plugin(&self, args: &Value) -> Result<Value, String> {
        let mut reply = self
            .plugin_rpc(
                methods::CHECK_PLUGIN_UPDATES,
                json!({ "id": given(args, "id") }),
            )
            .await?;
        if reply["updates"].as_array().is_some_and(|u| !u.is_empty()) {
            reply["next"] = json!(
                "To install an update, the person updates the plugin in Zeron (Settings → Plugins); you can't update it for them. Tell them what you found."
            );
        }
        Ok(reply)
    }

    pub(crate) async fn describe_elements(&self) -> Result<Value, String> {
        self.plugin_rpc(methods::DESCRIBE_ELEMENTS, json!({})).await
    }

    pub(crate) async fn list_templates(&self) -> Result<Value, String> {
        self.plugin_rpc(methods::LIST_TEMPLATES, json!({})).await
    }

    pub(crate) async fn describe_app(&self) -> Result<Value, String> {
        self.plugin_rpc(methods::DESCRIBE_APP, json!({})).await
    }

    pub(crate) async fn create_template(&self, args: &Value) -> Result<Value, String> {
        let id = template_id(args)?;
        let name = given(args, "name").ok_or("Give the template a `name`.")?;
        let mut params = json!({ "id": id, "name": name });
        if let Some(from) = given(args, "from") {
            params["from"] = json!(from);
        }
        self.plugin_rpc(methods::CREATE_TEMPLATE, params).await
    }

    pub(crate) async fn use_template(&self, args: &Value) -> Result<Value, String> {
        let params = json!({ "id": template_id(args)? });
        let mut reply = self.plugin_rpc(methods::USE_TEMPLATE, params).await?;
        if reply["preview"] == true {
            reply["next"] = json!(
                "Unless the person has kept this template, Zeron runs it as a preview with a bar to Keep or Revert it. Ask them to look; you can't keep it for them."
            );
        }
        Ok(reply)
    }

    pub(crate) async fn template_status(&self, args: &Value) -> Result<Value, String> {
        let params = json!({ "id": template_id(args)? });
        self.plugin_rpc(methods::TEMPLATE_STATUS, params).await
    }

    pub(crate) async fn delete_template(&self, args: &Value) -> Result<Value, String> {
        let params = json!({ "id": template_id(args)? });
        self.plugin_rpc(methods::DELETE_TEMPLATE, params).await
    }

    /// Never forwards a path: where an export lands is the person's choice, and the engine refuses one from an agent.
    pub(crate) async fn export_template(&self, args: &Value) -> Result<Value, String> {
        let params = json!({ "id": template_id(args)? });
        self.plugin_rpc(methods::EXPORT_TEMPLATE, params).await
    }

    pub(crate) async fn import_template(&self, args: &Value) -> Result<Value, String> {
        let params = match (given(args, "path"), given(args, "url")) {
            (Some(path), None) => json!({ "path": path }),
            (None, Some(url)) => json!({ "url": url }),
            _ => {
                return Err(
                    "Say what to import: `path`, a .zeron-template file, or `url`, a link to one."
                        .into(),
                );
            }
        };
        let mut reply = self.plugin_rpc(methods::IMPORT_TEMPLATE, params).await?;
        if reply["needs_consent"] == true {
            reply["next"] = json!(
                "It's waiting for the person to allow it in Zeron (Templates shows it, with who signed it). Ask them to look; you can't allow it for them."
            );
        }
        Ok(reply)
    }
}

fn installed(reply: Value) -> Value {
    let plugin = &reply["plugin"];
    let mut out = json!({
        "id": plugin["id"],
        "name": plugin["name"],
        "version": plugin["version"],
        "status": plugin["status"],
        "author": plugin["author"],
        "update": reply["update"],
        "added": reply["added"],
        "removed": reply["removed"],
        "waiting": reply["waiting"],
    });
    if let Some(from) = reply.get("from") {
        out["from"] = from.clone();
    }
    if reply["waiting"] == true {
        out["next"] = json!(
            "It's waiting for the person to allow it in Zeron (a notice asks them, and Settings → Plugins shows who signed it and what it asks). Ask them to look; you can't allow it for them."
        );
    }
    out
}

async fn scaffold_files(dir: &str) -> Vec<String> {
    let mut files = Vec::new();
    if !dir.is_empty()
        && let Ok(mut entries) = tokio::fs::read_dir(dir).await
    {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_file = entry.file_type().await.is_ok_and(|t| t.is_file());
            if is_file && !name.starts_with('.') {
                files.push(name);
            }
        }
    }
    if files.is_empty() {
        files = ["init.luau", "zeron-plugin.json", "zeron.luau"]
            .map(str::to_owned)
            .to_vec();
    }
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use futures::StreamExt;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    use zeron_rpc::{RpcReply, RpcService, memory_client};

    fn tools_reply(names: &[&str]) -> Value {
        json!({
            "device": { "id": "dev-local", "name": "Studio" },
            "tools": names.iter().map(|name| json!({
                "plugin": "word-count", "plugin_name": "Word Count", "tool": "count_words",
                "mcp_name": name, "description": "Count words in a text",
                "input": { "type": "object", "properties": { "text": { "type": "string" } }, "required": ["text"] },
                "open": true,
            })).collect::<Vec<_>>(),
        })
    }
    #[derive(Default)]
    struct Engine {
        disabled: Vec<String>,
        calls: Mutex<Vec<(String, Value)>>,
        watch: Vec<Value>,
        call_error: Option<&'static str>,
        no_plugins: bool,
    }

    #[async_trait]
    impl RpcService for Engine {
        async fn handle(&self, method: &str, params: Value) -> Result<RpcReply, RpcError> {
            if method == methods::GET_TOOL_SETTINGS {
                return Ok(RpcReply::Value(
                    json!({ "settings": {}, "disabled": self.disabled }),
                ));
            }
            self.calls
                .lock()
                .unwrap()
                .push((method.to_owned(), params.clone()));
            if self.no_plugins {
                return Err(RpcError::UnknownMethod(method.into()));
            }
            Ok(match method {
                methods::LIST_PLUGIN_TOOLS => {
                    RpcReply::Value(tools_reply(&["word_count__count_words"]))
                }
                methods::WATCH_PLUGIN_TOOLS => RpcReply::Stream(
                    futures::stream::iter(self.watch.clone())
                        .chain(futures::stream::pending())
                        .boxed(),
                ),
                methods::CALL_PLUGIN_TOOL => match self.call_error {
                    Some(message) => return Err(RpcError::Failed(message.into())),
                    None => RpcReply::Value(json!({ "ok": true, "value": { "count": 3 } })),
                },
                methods::NEW_PLUGIN => RpcReply::Value(json!({
                    "id": params["id"], "name": params["name"], "dir": "/nowhere/plugins/focus",
                    "status": "on", "own": true,
                    "guide": "/nowhere/docs/plugins.md", "guide_summary": "How to write a Zeron plugin.",
                })),
                methods::LIST_PLUGINS => RpcReply::Value(json!({
                    "dirs": { "plugins": "/d/plugins", "templates": "/d/templates" },
                    "plugins": [{
                        "id": "focus", "name": "Focus", "version": "1.2.0", "dir": "/d/plugins/focus",
                        "status": "pending", "author": { "kind": "verified", "username": "ada" },
                        "hash": "abc", "permissions": ["ui.add"],
                    }],
                })),
                methods::LIST_TEMPLATES => RpcReply::Value(json!({
                    "dir": "/d/templates", "active": "zeron",
                    "templates": [
                        { "id": "zeron", "name": "Zeron", "builtin": true, "active": true, "status": "ok", "dir": "/d/templates/.zeron" },
                        { "id": "midnight", "name": "Midnight", "builtin": false, "active": false, "status": "errors",
                          "error": "ds/tokens.luau: boom", "dir": "/d/templates/midnight" },
                    ],
                })),
                methods::CREATE_TEMPLATE => RpcReply::Value(json!({
                    "id": params["id"], "dir": "/d/templates/midnight", "files": ["app.luau"], "map": "app.luau is the root",
                })),
                methods::USE_TEMPLATE => match params["id"].as_str() {
                    Some("nope") => {
                        return Err(RpcError::Failed(
                            "There's no template nope. list_templates names them.".into(),
                        ));
                    }
                    _ => RpcReply::Value(
                        json!({ "active": params["id"], "preview": params["id"] != "zeron" }),
                    ),
                },
                methods::TEMPLATE_STATUS => RpcReply::Value(json!({
                    "id": params["id"], "active": false, "file_errors": { "ds/tokens.luau": "boom" },
                    "out_of_date": [], "files": 40, "inherits": 2,
                })),
                methods::DELETE_TEMPLATE => {
                    RpcReply::Value(json!({ "deleted": params["id"], "active": "zeron" }))
                }
                methods::EXPORT_TEMPLATE => RpcReply::Value(json!({
                    "id": params["id"], "path": params["path"], "files": 12, "key": "k1",
                })),
                methods::IMPORT_TEMPLATE => match params["path"].as_str() {
                    Some("/x/mine.zeron-template") => RpcReply::Value(json!({
                        "installed": "midnight", "name": "Midnight", "author": { "kind": "you" }, "ignored": [],
                    })),
                    Some("/x/bad.zeron-template") => {
                        return Err(RpcError::Failed(
                            "This template was changed after it was signed, so Zeron won't install it.".into(),
                        ));
                    }
                    _ => RpcReply::Value(json!({
                        "pending": "midnight-1", "id": "midnight", "name": "Midnight", "key": "k2",
                        "author": { "kind": "verified", "username": "ada" }, "needs_consent": true,
                    })),
                },
                methods::INSTALL_PLUGIN => RpcReply::Value(json!({
                    "plugin": { "id": "focus", "name": "Focus", "version": "1.0.0", "status": "pending",
                                "author": { "kind": "verified", "username": "ada" } },
                    "update": false, "added": [], "removed": [], "waiting": true,
                })),
                methods::UPDATE_PLUGIN => RpcReply::Value(json!({
                    "plugin": { "id": params["id"], "name": "Focus", "version": "2.0.0", "status": "on",
                                "author": { "kind": "verified", "username": "ada" } },
                    "update": true, "from": "1.0.0", "added": [], "removed": ["links.open"], "waiting": false,
                })),
                methods::CHECK_PLUGIN_UPDATES => RpcReply::Value(json!({
                    "updates": [{ "id": "focus", "name": "Focus", "version": "2.0.0", "added": ["github"], "removed": [] }],
                    "errors": [],
                })),
                methods::PLUGIN_STATUS => RpcReply::Value(json!({
                    "id": params["id"], "status": "on", "open": true,
                    "placements": [{ "at": ".session-row", "how": "after", "count": 3 }],
                })),
                methods::DESCRIBE_ELEMENTS => RpcReply::Value(json!({
                    "elements": [{ "selector": ".session-row", "place": "a sidebar row", "provides": ["session"], "present": true }],
                })),
                methods::DESCRIBE_APP => RpcReply::Value(json!({
                    "screens": [{ "module": "@app/screens/shell/sidebar", "elements": ["#sidebar"] }], "map": "app.luau is the root",
                })),
                other => return Err(RpcError::UnknownMethod(other.into())),
            })
        }
    }

    fn last_call(engine: &Engine) -> (String, Value) {
        engine.calls.lock().unwrap().last().unwrap().clone()
    }

    #[tokio::test]
    async fn template_tools_forward_and_report_errors() {
        let engine = Arc::new(Engine::default());
        let tools = tools_for(engine.clone(), None);

        let listed = tools.call(LIST_TEMPLATES, &json!({})).await.unwrap();
        assert_eq!(listed["templates"][1]["error"], "ds/tokens.luau: boom");
        assert_eq!(
            last_call(&engine),
            (methods::LIST_TEMPLATES.to_owned(), json!({}))
        );

        let made = tools
            .call(
                CREATE_TEMPLATE,
                &json!({ "id": "midnight", "name": "Midnight", "from": "dusk" }),
            )
            .await
            .unwrap();
        assert_eq!(made["dir"], "/d/templates/midnight");
        assert_eq!(
            last_call(&engine),
            (
                methods::CREATE_TEMPLATE.to_owned(),
                json!({ "id": "midnight", "name": "Midnight", "from": "dusk" })
            )
        );
        tools
            .call(
                CREATE_TEMPLATE,
                &json!({ "id": "midnight", "name": "Midnight" }),
            )
            .await
            .unwrap();
        assert_eq!(
            last_call(&engine).1,
            json!({ "id": "midnight", "name": "Midnight" })
        );
        let used = tools
            .call(USE_TEMPLATE, &json!({ "id": "midnight" }))
            .await
            .unwrap();
        assert_eq!(
            (used["active"].as_str(), used["preview"].as_bool()),
            (Some("midnight"), Some(true))
        );
        assert!(
            used["next"]
                .as_str()
                .unwrap()
                .contains("can't keep it for them")
        );
        assert_eq!(
            last_call(&engine),
            (
                methods::USE_TEMPLATE.to_owned(),
                json!({ "id": "midnight" })
            )
        );
        assert_eq!(
            tools
                .call(USE_TEMPLATE, &json!({ "id": "nope" }))
                .await
                .unwrap_err(),
            "There's no template nope. list_templates names them."
        );

        let status = tools
            .call(TEMPLATE_STATUS, &json!({ "id": "midnight" }))
            .await
            .unwrap();
        assert_eq!(status["file_errors"]["ds/tokens.luau"], "boom");
        assert_eq!(last_call(&engine).0, methods::TEMPLATE_STATUS);

        let deleted = tools
            .call(DELETE_TEMPLATE, &json!({ "id": "midnight" }))
            .await
            .unwrap();
        assert_eq!(deleted, json!({ "deleted": "midnight", "active": "zeron" }));
        assert_eq!(
            last_call(&engine),
            (
                methods::DELETE_TEMPLATE.to_owned(),
                json!({ "id": "midnight" })
            )
        );

        let described = tools.call(DESCRIBE_APP, &json!({})).await.unwrap();
        assert_eq!(described["screens"][0]["elements"], json!(["#sidebar"]));
        assert_eq!(
            last_call(&engine),
            (methods::DESCRIBE_APP.to_owned(), json!({}))
        );
    }

    #[tokio::test]
    async fn template_package_tools_forward_and_never_allow() {
        let engine = Arc::new(Engine::default());
        let tools = tools_for(engine.clone(), None);

        let exported = tools
            .call(
                EXPORT_TEMPLATE,
                &json!({ "id": "midnight", "path": "/Users/x/.ssh/x.zeron-template" }),
            )
            .await
            .unwrap();
        assert_eq!(exported["files"], 12);
        assert_eq!(
            last_call(&engine),
            (
                methods::EXPORT_TEMPLATE.to_owned(),
                json!({ "id": "midnight" })
            )
        );
        let mine = tools
            .call(
                IMPORT_TEMPLATE,
                &json!({ "path": "/x/mine.zeron-template" }),
            )
            .await
            .unwrap();
        assert_eq!(mine["installed"], "midnight");
        assert!(mine.get("next").is_none());
        assert_eq!(
            last_call(&engine),
            (
                methods::IMPORT_TEMPLATE.to_owned(),
                json!({ "path": "/x/mine.zeron-template" })
            )
        );
        let theirs = tools
            .call(
                IMPORT_TEMPLATE,
                &json!({ "path": "/x/theirs.zeron-template" }),
            )
            .await
            .unwrap();
        assert_eq!(theirs["needs_consent"], true);
        assert!(
            theirs["next"]
                .as_str()
                .unwrap()
                .contains("can't allow it for them")
        );
        assert!(
            tools
                .call(IMPORT_TEMPLATE, &json!({ "path": "/x/bad.zeron-template" }))
                .await
                .unwrap_err()
                .contains("changed after it was signed")
        );
        let before = engine.calls.lock().unwrap().len();
        assert!(
            tools
                .call(EXPORT_TEMPLATE, &json!({ "path": "/x/a.zeron-template" }))
                .await
                .unwrap_err()
                .contains("`id`")
        );
        assert!(
            tools
                .call(IMPORT_TEMPLATE, &json!({ "path": " " }))
                .await
                .unwrap_err()
                .contains("`path`")
        );
        assert_eq!(engine.calls.lock().unwrap().len(), before);
        let catalog = super::tools();
        assert!(
            catalog
                .iter()
                .all(|d| !d["name"].as_str().unwrap().contains("decide")
                    && !d["name"].as_str().unwrap().contains("allow"))
        );
        let export = catalog
            .iter()
            .find(|d| d["name"] == EXPORT_TEMPLATE)
            .unwrap();
        assert!(export["inputSchema"]["properties"].get("path").is_none());
        let import = catalog
            .iter()
            .find(|d| d["name"] == IMPORT_TEMPLATE)
            .unwrap();
        for field in ["path", "url"] {
            assert!(
                import["inputSchema"]["properties"].get(field).is_some(),
                "{field}"
            );
        }
        assert!(
            tools
                .call(
                    IMPORT_TEMPLATE,
                    &json!({ "path": "/x/a.zeron-template", "url": "ada/midnight" })
                )
                .await
                .is_err()
        );
        let before = engine.calls.lock().unwrap().len();
        let _ = tools
            .call(IMPORT_TEMPLATE, &json!({ "url": "ada/midnight" }))
            .await;
        let calls = engine.calls.lock().unwrap();
        assert_eq!(calls.len(), before + 1);
        assert!(format!("{:?}", calls.last().unwrap()).contains("ada/midnight"));
    }

    #[tokio::test]
    async fn template_tools_need_an_id() {
        let engine = Arc::new(Engine::default());
        let tools = tools_for(engine.clone(), None);
        for tool in [
            CREATE_TEMPLATE,
            USE_TEMPLATE,
            TEMPLATE_STATUS,
            DELETE_TEMPLATE,
        ] {
            let error = tools
                .call(tool, &json!({ "name": "Midnight" }))
                .await
                .unwrap_err();
            assert!(error.contains("`id`"), "{tool}: {error}");
            assert!(
                tools
                    .call(tool, &json!({ "id": " ", "name": "Midnight" }))
                    .await
                    .is_err(),
                "{tool}"
            );
        }
        assert!(
            tools
                .call(CREATE_TEMPLATE, &json!({ "id": "midnight" }))
                .await
                .unwrap_err()
                .contains("`name`")
        );
        assert!(engine.calls.lock().unwrap().is_empty());
        let catalog = super::tools();
        for tool in [
            CREATE_TEMPLATE,
            USE_TEMPLATE,
            TEMPLATE_STATUS,
            DELETE_TEMPLATE,
        ] {
            let def = catalog.iter().find(|d| d["name"] == tool).unwrap();
            assert!(
                def["inputSchema"]["required"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("id")),
                "{tool}"
            );
        }
        for tool in [LIST_TEMPLATES, DESCRIBE_APP] {
            assert!(catalog.iter().any(|d| d["name"] == tool), "{tool}");
        }
    }

    fn tools_for(engine: Arc<Engine>, chat: Option<&str>) -> ThreadTools {
        ThreadTools::new(memory_client(engine), chat.unwrap_or("chat-0"))
    }

    #[test]
    fn plugin_tool_names_are_told_apart_from_static_ones() {
        assert!(is_plugin_tool_name("word_count__count_words"));
        assert!(!is_plugin_tool_name("list_plugins"));
        assert!(!is_plugin_tool_name("__count"));
        assert!(!is_plugin_tool_name("Word__count"));
        for def in crate::tool_list().as_array().unwrap() {
            assert!(!is_plugin_tool_name(def["name"].as_str().unwrap()), "{def}");
        }
    }

    #[tokio::test]
    async fn tools_list_includes_plugin_tools() {
        let tools = tools_for(Arc::new(Engine::default()), Some("chat-1"));
        let listed = tools.tools().await;
        let listed = listed.as_array().unwrap();
        assert!(listed.iter().any(|t| t["name"] == CREATE_PLUGIN));
        assert!(listed.iter().any(|t| t["name"] == LIST_PLUGINS));
        let plugin = listed
            .iter()
            .find(|t| t["name"] == "word_count__count_words")
            .expect("plugin tool listed");
        assert_eq!(
            plugin["description"],
            "Count words in a text (From the Word Count plugin. Works while Zeron is open on Studio.)"
        );
        assert_eq!(plugin["inputSchema"]["required"], json!(["text"]));
        assert_eq!(
            listed.len(),
            crate::tool_list().as_array().unwrap().len() + 1
        );
    }

    #[tokio::test]
    async fn an_engine_without_plugins_lists_the_static_tools() {
        let tools = tools_for(
            Arc::new(Engine {
                no_plugins: true,
                ..Engine::default()
            }),
            None,
        );
        assert_eq!(
            tools.tools().await.as_array().unwrap().len(),
            crate::tool_list().as_array().unwrap().len()
        );
    }

    #[tokio::test]
    async fn a_plugin_tool_call_forwards_with_the_chat() {
        let engine = Arc::new(Engine::default());
        let tools = tools_for(engine.clone(), Some("chat-1"));
        let value = tools
            .call("word_count__count_words", &json!({ "text": "a b c" }))
            .await
            .unwrap();
        assert_eq!(value, json!({ "count": 3 }));
        let calls = engine.calls.lock().unwrap();
        let (method, params) = calls.last().unwrap();
        assert_eq!(method, methods::CALL_PLUGIN_TOOL);
        assert_eq!(
            params,
            &json!({
                "mcp_name": "word_count__count_words",
                "args": { "text": "a b c" },
                "chatId": "chat-1",
            })
        );
    }

    #[tokio::test]
    async fn a_plugin_tool_follows_the_group_switch() {
        let engine = Arc::new(Engine {
            disabled: super::tools()
                .iter()
                .chain(&crate::themes::tools())
                .map(|tool| tool["name"].as_str().unwrap().to_owned())
                .collect(),
            ..Engine::default()
        });
        let tools = tools_for(engine.clone(), None);
        let listed = tools.tools().await;
        let listed: Vec<&str> = listed
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert!(!listed.contains(&LIST_PLUGINS) && !listed.contains(&"word_count__count_words"));
        let reply = crate::respond(
            &json!({
                "id": 7, "method": "tools/call",
                "params": { "name": "word_count__count_words", "arguments": {} },
            }),
            &tools,
        )
        .await
        .unwrap();
        assert_eq!(reply["result"]["isError"], true);
        assert_eq!(
            reply["result"]["content"][0]["text"],
            "word_count__count_words is turned off in Zeron's Settings > Tools."
        );
        assert!(
            engine
                .calls
                .lock()
                .unwrap()
                .iter()
                .all(|(method, _)| method != methods::CALL_PLUGIN_TOOL)
        );
    }

    #[tokio::test]
    async fn a_plugin_tool_error_is_the_engine_message_and_an_is_error_result() {
        let engine = Arc::new(Engine {
            call_error: Some("Open Zeron on Studio to use count_words"),
            ..Engine::default()
        });
        let tools = tools_for(engine, None);
        assert_eq!(
            tools
                .call("word_count__count_words", &json!({}))
                .await
                .unwrap_err(),
            "Open Zeron on Studio to use count_words"
        );
        let reply = crate::respond(
            &json!({
                "id": 7, "method": "tools/call",
                "params": { "name": "word_count__count_words", "arguments": {} },
            }),
            &tools,
        )
        .await
        .unwrap();
        assert_eq!(reply["result"]["isError"], true);
        assert_eq!(
            reply["result"]["content"][0]["text"],
            "Open Zeron on Studio to use count_words"
        );
    }

    #[tokio::test]
    async fn create_plugin_and_list_plugins_forward() {
        let engine = Arc::new(Engine::default());
        let tools = tools_for(engine.clone(), None);
        let created = tools
            .call(
                CREATE_PLUGIN,
                &json!({ "id": "focus", "name": "Focus", "permissions": ["ui.add"] }),
            )
            .await
            .unwrap();
        assert_eq!(created["id"], "focus");
        assert_eq!(created["dir"], "/nowhere/plugins/focus");
        assert_eq!(
            created["files"],
            json!(["init.luau", "zeron-plugin.json", "zeron.luau"])
        );
        assert_eq!(
            created["hint"],
            "Read /nowhere/docs/plugins.md first (how to write a plugin, with examples). Then edit /nowhere/plugins/focus/init.luau; Zeron re-signs and reloads it as you save. Check it with plugin_status (snapshot: true)."
        );
        assert_eq!(created["guide"], "/nowhere/docs/plugins.md");
        {
            let calls = engine.calls.lock().unwrap();
            let (method, params) = calls.last().unwrap();
            assert_eq!(method, methods::NEW_PLUGIN);
            assert_eq!(
                params,
                &json!({ "id": "focus", "name": "Focus", "permissions": ["ui.add"] })
            );
        }
        let listed = tools.call(LIST_PLUGINS, &json!({})).await.unwrap();
        assert_eq!(
            listed,
            json!({ "plugins": [{
                "id": "focus", "name": "Focus", "status": "pending",
                "author": { "kind": "verified", "username": "ada" }, "dir": "/d/plugins/focus",
            }] })
        );
        assert_eq!(
            engine.calls.lock().unwrap().last().unwrap().0,
            methods::LIST_PLUGINS
        );
        let status = tools
            .call(PLUGIN_STATUS, &json!({ "id": "focus", "snapshot": true }))
            .await
            .unwrap();
        assert_eq!(status["placements"][0]["count"], 3);
        assert_eq!(
            last_call(&engine),
            (
                methods::PLUGIN_STATUS.to_owned(),
                json!({ "id": "focus", "snapshot": true })
            )
        );
        let map = tools.call(DESCRIBE_ELEMENTS, &json!({})).await.unwrap();
        assert_eq!(map["elements"][0]["selector"], ".session-row");
        assert_eq!(
            engine.calls.lock().unwrap().last().unwrap().0,
            methods::DESCRIBE_ELEMENTS
        );
    }

    #[tokio::test]
    async fn install_and_update_plugin_leave_consent_to_the_person() {
        let engine = Arc::new(Engine::default());
        let tools = tools_for(engine.clone(), None);
        let installed = tools
            .call(
                INSTALL_PLUGIN,
                &json!({ "url": "ada/focus", "ref": "v1", "folder": " " }),
            )
            .await
            .unwrap();
        assert_eq!(
            last_call(&engine),
            (
                methods::INSTALL_PLUGIN.to_owned(),
                json!({ "url": "ada/focus", "ref": "v1", "folder": null })
            )
        );
        assert_eq!(
            (installed["status"].as_str(), installed["waiting"].as_bool()),
            (Some("pending"), Some(true))
        );
        assert!(
            installed["next"]
                .as_str()
                .unwrap()
                .contains("can't allow it for them")
        );
        tools
            .call(INSTALL_PLUGIN, &json!({ "path": "/x/focus.zeron-plugin" }))
            .await
            .unwrap();
        assert_eq!(
            last_call(&engine),
            (
                methods::INSTALL_PLUGIN.to_owned(),
                json!({ "path": "/x/focus.zeron-plugin" })
            )
        );
        assert!(
            tools
                .call(INSTALL_PLUGIN, &json!({}))
                .await
                .unwrap_err()
                .contains("`path`")
        );
        assert!(
            tools
                .call(INSTALL_PLUGIN, &json!({ "path": "/a", "url": "b/c" }))
                .await
                .unwrap_err()
                .contains("one of")
        );

        let checked = tools
            .call(UPDATE_PLUGIN, &json!({ "check": true }))
            .await
            .unwrap();
        assert_eq!(checked["updates"][0]["added"], json!(["github"]));
        assert_eq!(
            last_call(&engine),
            (
                methods::CHECK_PLUGIN_UPDATES.to_owned(),
                json!({ "id": null })
            )
        );
        let updated = tools
            .call(UPDATE_PLUGIN, &json!({ "id": "focus" }))
            .await
            .unwrap();
        assert_eq!(updated["updates"][0]["version"], "2.0.0");
        assert!(
            updated["next"]
                .as_str()
                .unwrap()
                .contains("can't update it for them")
        );
        assert_eq!(
            last_call(&engine),
            (
                methods::CHECK_PLUGIN_UPDATES.to_owned(),
                json!({ "id": "focus" })
            )
        );
        tools.call(UPDATE_PLUGIN, &json!({})).await.unwrap();
        assert!(
            engine
                .calls
                .lock()
                .unwrap()
                .iter()
                .all(|(method, _)| method != methods::UPDATE_PLUGIN),
            "the tool never installs an update"
        );
    }

    #[tokio::test]
    async fn create_plugin_lists_the_folder_it_made() {
        let dir = tempfile::tempdir().unwrap();
        for file in ["zeron-plugin.json", "init.luau", "zeron.luau", ".hidden"] {
            std::fs::write(dir.path().join(file), "").unwrap();
        }
        std::fs::create_dir(dir.path().join("lib")).unwrap();
        assert_eq!(
            scaffold_files(dir.path().to_str().unwrap()).await,
            vec!["init.luau", "zeron-plugin.json", "zeron.luau"]
        );
    }

    #[tokio::test]
    async fn the_watch_says_changed_only_when_the_names_change() {
        let engine = Arc::new(Engine {
            watch: vec![
                tools_reply(&["a__one"]),
                tools_reply(&["a__one"]),
                tools_reply(&["a__one", "b__two"]),
            ],
            ..Engine::default()
        });
        let tools = Arc::new(tools_for(engine, None));
        let (tx, mut rx) = mpsc::channel(8);
        let watch = tokio::spawn({
            let tools = tools.clone();
            async move { tools.watch_plugin_tools(tx).await }
        });
        tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("a change in time")
            .expect("one change");
        assert!(
            tokio::time::timeout(Duration::from_millis(200), rx.recv())
                .await
                .is_err(),
            "exactly one change"
        );
        watch.abort();
    }

    #[tokio::test]
    async fn serve_says_list_changed_when_the_names_change() {
        let engine = Arc::new(Engine {
            watch: vec![tools_reply(&["a__one"]), tools_reply(&["a__one", "b__two"])],
            ..Engine::default()
        });
        let tools = tools_for(engine, None);
        let (client_side, server_side) = tokio::io::duplex(64 * 1024);
        let (server_read, server_write) = tokio::io::split(server_side);
        let server = tokio::spawn(async move {
            crate::serve(tokio::io::BufReader::new(server_read), server_write, &tools).await
        });
        let (client_read, mut client_write) = tokio::io::split(client_side);
        let mut lines = tokio::io::BufReader::new(client_read).lines();
        let line = tokio::time::timeout(Duration::from_secs(5), lines.next_line())
            .await
            .expect("a notification in time")
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&line).unwrap(),
            crate::plugins::list_changed_notification()
        );
        client_write
            .write_all(b"{\"id\":1,\"method\":\"initialize\",\"params\":{}}\n")
            .await
            .unwrap();
        let line = lines.next_line().await.unwrap().unwrap();
        let init: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(init["result"]["capabilities"]["tools"]["listChanged"], true);
        server.abort();
    }

    #[tokio::test]
    async fn the_watch_stops_on_an_engine_without_plugins() {
        let tools = tools_for(
            Arc::new(Engine {
                no_plugins: true,
                ..Engine::default()
            }),
            None,
        );
        let (tx, _rx) = mpsc::channel(1);
        tokio::time::timeout(Duration::from_secs(5), tools.watch_plugin_tools(tx))
            .await
            .expect("returns at once");
    }
}
