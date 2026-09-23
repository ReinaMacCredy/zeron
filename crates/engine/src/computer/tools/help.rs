use serde_json::{Map, Value, json};
use zeron_proto::computer::{McpToolDefinition, McpToolResult};

use super::desktop::Handled;
use super::guidance::{COMPUTER_HELP_INDEX, COMPUTER_HELP_SECTIONS, computer_help_section};
use super::results::ToolFailure;
use super::tool_input::{ToolArgs, ToolInputError, read_string_arg};
use super::{
    ComputerToolContext, ComputerToolEntry, ComputerTools, RUN_CONDITION_FIELDS,
    batch_step_type_for, is_inspection_tool_name, run_step_fields,
};

fn index_line(entry: &ComputerToolEntry) -> String {
    match entry.title() {
        Some(title) => format!("- {} — {title}", entry.name()),
        None => format!("- {}", entry.name()),
    }
}

fn validate_inspection_arguments(
    value: &Value,
    definition: &McpToolDefinition,
) -> Result<ToolArgs, ToolInputError> {
    let args = value
        .as_object()
        .ok_or_else(|| ToolInputError::new("\"arguments\" must be an object."))?;
    let schema = &definition.input_schema;
    let unvalidatable = || ToolInputError::new("The inspection schema cannot be validated.");
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .filter(|_| {
            schema.get("type") == Some(&json!("object"))
                && schema.get("additionalProperties") == Some(&json!(false))
        })
        .ok_or_else(unvalidatable)?;
    for key in schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if !args.contains_key(key) {
            return Err(ToolInputError::new(format!(
                "Missing required argument \"{key}\"."
            )));
        }
    }
    for (key, property) in properties {
        let kind = property.get("type").and_then(Value::as_str);
        let plain = property
            .as_object()
            .is_some_and(|fields| fields.keys().all(|field| field == "type" || field == "description"));
        if !matches!(kind, Some("string" | "number")) || !plain {
            return Err(unvalidatable());
        }
        let Some(value) = args.get(key) else {
            continue;
        };
        let matches = match kind {
            Some("string") => value.is_string(),
            _ => value.as_f64().is_some_and(f64::is_finite),
        };
        if !matches {
            return Err(ToolInputError::new(format!(
                "Argument \"{key}\" must be a {}.",
                kind.unwrap_or_default()
            )));
        }
    }
    if let Some(unknown) = args.keys().find(|key| !properties.contains_key(*key)) {
        return Err(ToolInputError::new(format!(
            "Unknown argument \"{unknown}\"."
        )));
    }
    Ok(args.clone())
}

impl ComputerTools {
    fn tool_index_text(&self) -> String {
        let catalog = self.entries();
        let hidden: Vec<&ComputerToolEntry> =
            catalog.iter().filter(|entry| entry.discovery_only).collect();
        let mut lines = vec!["Advertised in tools/list:".to_owned()];
        lines.extend(
            catalog
                .iter()
                .filter(|entry| !entry.discovery_only)
                .map(index_line),
        );
        lines.push(String::new());
        lines.push(
            "Available as computer_run steps (read computer_help with tool for fields):".into(),
        );
        lines.extend(
            hidden
                .iter()
                .filter(|entry| batch_step_type_for(entry.name()).is_some())
                .map(|entry| index_line(entry)),
        );
        lines.push(String::new());
        lines.push(
            "Available through computer_inspect (read computer_help with tool for fields):".into(),
        );
        lines.extend(
            hidden
                .iter()
                .filter(|entry| is_inspection_tool_name(entry.name()))
                .map(|entry| index_line(entry)),
        );
        lines.extend(
            hidden
                .iter()
                .filter(|entry| {
                    batch_step_type_for(entry.name()).is_none()
                        && !is_inspection_tool_name(entry.name())
                })
                .map(|entry| format!("{} (callable by exact name only)", index_line(entry))),
        );
        lines.join("\n")
    }

    fn section_text(&self, name: &str) -> Option<String> {
        let text = computer_help_section(name)?;
        Some(if name == "tools" {
            format!("{text}\n\n{}", self.tool_index_text())
        } else {
            text.to_owned()
        })
    }

    pub(super) async fn help(&self, args: &ToolArgs) -> Handled {
        let tool = read_string_arg(args, "tool", false)?;
        let topic = read_string_arg(args, "topic", false)?;
        if let Some(tool) = tool {
            if topic.is_some() {
                return Err(ToolInputError::new("Use either tool or topic, not both.").into());
            }
            let entry = self.entry(&tool).ok_or_else(|| {
                ToolInputError::new(format!(
                    "Unknown desktop tool \"{tool}\". Read topic \"tools\" for the index."
                ))
            })?;
            let mut payload = Map::new();
            payload.insert(
                "definition".into(),
                serde_json::to_value(&entry.definition).unwrap_or(Value::Null),
            );
            payload.insert("advertised".into(), json!(!entry.discovery_only));
            match batch_step_type_for(&tool) {
                Some(step) => {
                    let mut fields = run_step_fields(step).unwrap_or_default();
                    fields.extend_from_slice(RUN_CONDITION_FIELDS);
                    payload.insert(
                        "batchStep".into(),
                        json!({
                            "type": step,
                            "fields": fields,
                            "instruction": "Use type plus these fields in computer_run.steps. The tool schema describes their values; per-step screenshots and delivery_mode are not supported.",
                        }),
                    );
                }
                None if is_inspection_tool_name(&tool) => {
                    payload.insert(
                        "inspection".into(),
                        json!({
                            "name": "computer_inspect",
                            "tool": tool,
                            "instruction": "Pass this schema's arguments in the arguments object.",
                        }),
                    );
                }
                None if entry.discovery_only => {
                    payload.insert(
                        "availability".into(),
                        json!("Callable by exact name only; help does not register the tool."),
                    );
                }
                None => {}
            }
            return Ok(Value::Object(payload).into());
        }
        match topic.as_deref() {
            None => Ok(json!({ "topics": COMPUTER_HELP_INDEX }).into()),
            Some("all") => {
                let chapters = COMPUTER_HELP_SECTIONS
                    .iter()
                    .map(|(name, _)| {
                        format!("## {name}\n{}", self.section_text(name).unwrap_or_default())
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n");
                Ok(json!({ "chapters": chapters }).into())
            }
            Some(topic) => {
                let text = self.section_text(topic).ok_or_else(|| {
                    let topics: Vec<&str> =
                        COMPUTER_HELP_SECTIONS.iter().map(|(name, _)| *name).collect();
                    ToolInputError::new(format!(
                        "Unknown computer_help topic \"{topic}\". Topics: {}, all.",
                        topics.join(", ")
                    ))
                })?;
                Ok(json!({ "topic": topic, "text": text }).into())
            }
        }
    }

    pub(super) async fn inspect(
        &self,
        args: &ToolArgs,
        context: &ComputerToolContext,
    ) -> McpToolResult {
        let prepared = (|| -> Result<(String, ToolArgs), ToolFailure> {
            if args.keys().any(|key| key != "tool" && key != "arguments") {
                return Err(ToolInputError::new(
                    "computer_inspect accepts only \"tool\" and \"arguments\".",
                )
                .into());
            }
            let name = args
                .get("tool")
                .and_then(Value::as_str)
                .filter(|name| is_inspection_tool_name(name))
                .ok_or_else(|| ToolInputError::new("Unknown Computer inspection tool."))?;
            let entry = self
                .entry(name)
                .ok_or_else(|| ToolInputError::new("Unknown Computer inspection tool."))?;
            let empty = json!({});
            let tool_args =
                validate_inspection_arguments(args.get("arguments").unwrap_or(&empty), &entry.definition)?;
            Ok((name.to_owned(), tool_args))
        })();
        match prepared {
            Ok((name, tool_args)) => self.call(&name, tool_args, context).await,
            Err(error) => McpToolResult::error(error.text()),
        }
    }
}
