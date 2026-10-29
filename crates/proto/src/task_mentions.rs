//! Task mentions: `[title](zeron-task:<id>)` names a task on the Tasks page.
//! The agent reads it whole with the `zeron` MCP `tasks` tool (`list` with
//! `taskId`), so the link carries only the id and the title at insert time.
use std::ops::Range;

use crate::chat_mentions::{canonical_links, valid_chat_id};
use crate::file_mentions::escape_mention_label;

pub const TASK_MENTION_SCHEME: &str = "zeron-task:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskMentionLink {
    pub range: Range<usize>,
    pub title: String,
    pub task_id: String,
}

fn clean_title(title: &str) -> String {
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        "Untitled task".into()
    } else {
        title
    }
}

/// `[title](zeron-task:<id>)`, or `None` for an id that isn't a plain id.
pub fn task_mention_link(task_id: &str, title: &str) -> Option<String> {
    valid_chat_id(task_id).then(|| {
        format!(
            "[{}]({TASK_MENTION_SCHEME}{task_id})",
            escape_mention_label(&clean_title(title))
        )
    })
}

/// Only canonical links, never escaped text or code examples.
pub fn task_mention_links(text: &str) -> Vec<TaskMentionLink> {
    canonical_links(text, TASK_MENTION_SCHEME, task_mention_link)
        .into_iter()
        .map(|(range, title, task_id)| TaskMentionLink {
            range,
            title: clean_title(&title),
            task_id,
        })
        .collect()
}

/// The one message that hands several tasks to a thread: a new thread works
/// through them; an existing one is told to also take them.
pub fn gathered_tasks_prompt<'a>(
    tasks: impl IntoIterator<Item = (&'a str, &'a str)>,
    existing: bool,
) -> String {
    let links: Vec<String> = tasks
        .into_iter()
        .filter_map(|(id, title)| task_mention_link(id, title))
        .collect();
    let lead = match (links.len(), existing) {
        (1, false) => "Work through this task in this thread. Mark it done".to_string(),
        (1, true) => "Also take this task in this thread. Mark it done".to_string(),
        (n, false) => {
            format!("Work through these {n} tasks in this thread, in order. Mark each one done")
        }
        (n, true) => {
            format!("Also take these {n} tasks in this thread, in order. Mark each one done")
        }
    };
    let list: Vec<String> = links
        .iter()
        .enumerate()
        .map(|(ix, link)| format!("{}. {link}", ix + 1))
        .collect();
    format!(
        "{lead} with the `tasks` tool when it is finished.\n\n{}",
        list.join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_round_trip_and_only_canonical_ones_decode() {
        let link = task_mention_link("t-1", "Fix  the\n[login] bug").unwrap();
        assert_eq!(link, "[Fix the \\[login\\] bug](zeron-task:t-1)");
        let text = format!("Do {link} first, not `[x](zeron-task:t2)` or [y](zeron-task:../z)");
        let links = task_mention_links(&text);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].task_id, "t-1");
        assert_eq!(links[0].title, "Fix the [login] bug");
        assert_eq!(&text[links[0].range.clone()], link);
        assert!(task_mention_link("../x", "t").is_none());
        assert!(task_mention_links("[a](zeron-chat:t1)").is_empty());
    }

    #[test]
    fn gathered_prompt_lists_every_task_in_order() {
        let prompt = gathered_tasks_prompt([("a1", "One"), ("b2", "Two")], false);
        assert_eq!(
            prompt,
            "Work through these 2 tasks in this thread, in order. Mark each one done with \
             the `tasks` tool when it is finished.\n\n1. [One](zeron-task:a1)\n2. [Two](zeron-task:b2)"
        );
        let more = gathered_tasks_prompt([("a1", "One")], true);
        assert!(more.starts_with("Also take this task in this thread. Mark it done"));
        assert_eq!(task_mention_links(&prompt).len(), 2);
    }

    #[test]
    fn task_links_reach_the_agent_verbatim_and_never_as_chat_ids() {
        let text = format!("Do {} now", task_mention_link("t1", "One").unwrap());
        for harness in [crate::HarnessId::ClaudeCode, crate::HarnessId::Codex] {
            assert_eq!(crate::invocation::harness_prompt(&text, harness), text);
        }
        assert!(crate::chat_mentions::mentioned_chat_ids(&text).is_empty());
    }
}
