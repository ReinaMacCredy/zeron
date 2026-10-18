use serde_json::Value;
use zeron_mcp::ui_driver;

use super::{Status, StepReport};

const RESULT_CHARS: usize = 160;

pub(super) fn status_word(status: Status) -> &'static str {
    match status {
        Status::Pass => "pass",
        Status::Fail => "FAIL",
        Status::Skipped => "skipped",
    }
}

pub(super) fn file_label(label: &str) -> String {
    label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

pub(super) fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn short(value: &Value) -> String {
    let line = one_line(&ui_driver::text(value));
    match line.char_indices().nth(RESULT_CHARS) {
        Some((cut, _)) => format!("{}…", &line[..cut]),
        None => line,
    }
}

pub(super) fn summary(reports: &[StepReport]) -> String {
    let passed = reports.iter().filter(|r| r.status == Status::Pass).count();
    let total = reports.len();
    match reports.iter().find(|r| r.status == Status::Fail) {
        None => format!("PASS {passed}/{total}"),
        Some(failed) => format!(
            "FAIL {passed}/{total}: step {} {}: {}",
            failed.index,
            failed.name,
            one_line(failed.error.as_deref().unwrap_or_default())
        ),
    }
}

pub(super) fn markdown(name: &str, target: &Value, reports: &[StepReport]) -> String {
    let cell = |text: &str| one_line(text).replace('|', "\\|");
    let mut md = format!(
        "# Scenario {name}\n\n**{}**\n\nTarget: {} (pid {}, engine port {})\n\n| # | Step | Kind | Status | ms | Result or error |\n|---|---|---|---|---|---|\n",
        summary(reports),
        target["label"].as_str().unwrap_or_default(),
        target["pid"],
        target["ipcPort"],
    );
    for report in reports {
        let detail = match (&report.error, &report.shot, &report.result) {
            (Some(error), _, _) => cell(error),
            (None, Some(shot), _) => format!("[{shot}]({shot})"),
            (None, None, Some(result)) => cell(result),
            (None, None, None) => String::new(),
        };
        let detail = match &report.warning {
            Some(warning) => format!("warning: {}; {detail}", cell(warning)),
            None => detail,
        };
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {detail} |\n",
            report.index,
            cell(&report.name),
            report.kind,
            status_word(report.status),
            report.ms,
        ));
    }
    let shots: Vec<&String> = reports.iter().filter_map(|r| r.shot.as_ref()).collect();
    if !shots.is_empty() {
        md.push_str("\n## Snapshots\n");
        for shot in shots {
            md.push_str(&format!("\n### {shot}\n\n![{shot}]({shot})\n"));
        }
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_report_tables_every_step_and_links_each_snapshot() {
        let report = |index, status, shot: Option<&str>, error: Option<&str>| StepReport {
            index,
            kind: "ui",
            name: format!("step | {index}"),
            status,
            ms: 5,
            result: Some("ok".into()),
            error: error.map(str::to_owned),
            warning: None,
            shot: shot.map(str::to_owned),
        };
        let reports = [
            report(1, Status::Pass, Some("shots/01-before.png"), None),
            report(2, Status::Fail, None, Some("timed out\nlast seen: x")),
            report(3, Status::Skipped, None, None),
        ];
        let md = markdown(
            "demo",
            &json!({ "label": "Zeron Scenario", "pid": 42, "ipcPort": 49880 }),
            &reports,
        );
        assert!(
            md.starts_with(
                "# Scenario demo\n\n**FAIL 1/3: step 2 step | 2: timed out last seen: x**"
            ),
            "{md}"
        );
        assert!(
            md.contains("Target: Zeron Scenario (pid 42, engine port 49880)"),
            "{md}"
        );
        assert!(
            md.contains(
                "| 1 | step \\| 1 | ui | pass | 5 | [shots/01-before.png](shots/01-before.png) |"
            ),
            "{md}"
        );
        assert!(
            md.contains("| 2 | step \\| 2 | ui | FAIL | 5 | timed out last seen: x |"),
            "{md}"
        );
        assert!(
            md.contains("| 3 | step \\| 3 | ui | skipped | 5 | ok |"),
            "{md}"
        );
        assert!(
            md.contains("![shots/01-before.png](shots/01-before.png)"),
            "{md}"
        );
        assert_eq!(summary(&reports[..1]), "PASS 1/1");
    }
}
