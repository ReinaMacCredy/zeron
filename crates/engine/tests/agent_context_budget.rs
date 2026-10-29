use serde_json::{Value, json};
use zeron_engine::routing::{RoutingSettings, guidance};

const BUDGET: &str = include_str!("agent_context_budget.json");

fn system_text() -> String {
    [
        guidance::guidance(&RoutingSettings::default(), None),
        zeron_proto::html_render::GUIDANCE.to_owned(),
        zeron_engine::device_hub::DEVICE_TOOL_INSTRUCTIONS.to_owned(),
    ]
    .join("\n\n")
}

fn advertised(tool: &Value) -> Value {
    json!({ "name": tool["name"], "description": tool["description"], "inputSchema": tool["inputSchema"] })
}

#[test]
fn agent_context_matches_the_committed_budget() {
    let system = system_text();
    let tools = zeron_mcp::tool_list();
    let tools = tools.as_array().unwrap();
    let mut rows: Vec<(String, usize, usize)> = tools
        .iter()
        .map(|tool| {
            (
                tool["name"].as_str().unwrap().to_owned(),
                tool["description"].as_str().unwrap_or_default().len(),
                tool["inputSchema"].to_string().len(),
            )
        })
        .collect();
    rows.sort_by_key(|(_, description, schema)| std::cmp::Reverse(description + schema));
    let tool_list = Value::Array(tools.iter().map(advertised).collect()).to_string();
    let (loaded, deferred): (Vec<&Value>, Vec<&Value>) = tools
        .iter()
        .partition(|tool| tool["_meta"]["anthropic/alwaysLoad"] == true);
    let always_loaded = Value::Array(loaded.into_iter().map(advertised).collect()).to_string();
    let deferred: String = deferred
        .iter()
        .map(|tool| format!("mcp__zeron__{}\n", tool["name"].as_str().unwrap()))
        .collect();
    println!(
        "{:<28} {:>7} {:>7} {:>7}",
        "tool", "desc", "schema", "tokens"
    );
    for (name, description, schema) in &rows {
        println!(
            "{name:<28} {description:>7} {schema:>7} {:>7}",
            (description + schema) / 4
        );
    }
    let off = system.len() + tool_list.len();
    let on = system.len() + deferred.len() + always_loaded.len();
    println!(
        "{} tools; system text {} B; descriptions {} B; schemas {} B; tool list JSON {} B",
        tools.len(),
        system.len(),
        rows.iter().map(|row| row.1).sum::<usize>(),
        rows.iter().map(|row| row.2).sum::<usize>(),
        tool_list.len()
    );
    println!(
        "tool search off: {off} B (~{} tokens); on: {on} B (~{} tokens)",
        off / 4,
        on / 4
    );
    let measured = json!({
        "systemTextBytes": system.len(),
        "toolListJsonBytes": tool_list.len(),
        "deferredNamesBytes": deferred.len(),
        "alwaysLoadedBytes": always_loaded.len(),
    });
    println!("measured: {measured}");
    let budget: Value = serde_json::from_str(BUDGET).unwrap();
    assert_eq!(
        measured, budget,
        "Zeron's agent context changed size. A cut lowers the budget; growth raises it in a reviewed change. Run `cargo test -p zeron-engine --test agent_context_budget -- --nocapture` and copy its `measured` line into crates/engine/tests/agent_context_budget.json"
    );
}
