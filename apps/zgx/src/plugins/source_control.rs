use serde_json::{Value as Json, json};

pub fn commit_request(asked: &Json, sessions: &Json, may: impl Fn(&str) -> bool) -> Result<Json, String> {
    if !may("source_control.ai") {
        return Err("This plugin wasn't allowed to write commit messages".into());
    }
    let id = asked["session"].as_str().filter(|s| !s.is_empty()).ok_or("Which session: its id")?;
    let chat = sessions["chats"].as_array().into_iter().flatten().find(|c| c["id"] == id).ok_or("There's no such session")?;
    let text = |key: &str| chat[key].as_str().filter(|s| !s.is_empty());
    let project = text("project").ok_or("Commit messages are written for a session in a project")?;
    let mut params = json!({ "spaceId": project });
    if let Some(cwd) = text("cwd") {
        params["checkoutPath"] = json!(cwd);
    }
    if let Some(device) = text("device") {
        params["targetDeviceId"] = json!(device);
    }
    // zgx has no commit model setting, so the session's own agent writes it, never one it doesn't use.
    if let Some(harness) = text("harness") {
        params["harness"] = json!(harness);
    }
    Ok(params)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_in_a_project_asks_on_its_device_with_its_own_agent_and_never_without_the_permission() {
        let sessions = json!({ "chats": [
            { "id": "s1", "project": "p1", "cwd": "/work/atlas", "device": "box", "harness": "mock" },
            { "id": "s2", "device": "box" },
        ] });
        let asked = json!({ "session": "s1" });
        assert_eq!(
            commit_request(&asked, &sessions, |p| p == "source_control.ai").unwrap(),
            json!({ "spaceId": "p1", "checkoutPath": "/work/atlas", "targetDeviceId": "box", "harness": "mock" })
        );
        assert!(commit_request(&asked, &sessions, |p| p == "github.write").is_err());
        assert!(commit_request(&json!({ "session": "s2" }), &sessions, |_| true).is_err(), "no project");
        assert!(commit_request(&json!({ "session": "s9" }), &sessions, |_| true).is_err(), "no such session");
        assert!(commit_request(&json!({}), &sessions, |_| true).is_err());
    }
}
