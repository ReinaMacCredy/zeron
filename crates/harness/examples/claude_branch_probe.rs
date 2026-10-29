//! Live probe for session branching on the real `claude` CLI: two turns in
//! one session, then a third run resumed with a branch cut after turn 1.
//! Prints each run's session id and reply; exits 0 only when the branched run
//! got a NEW session id and its reply knows turn 1's codeword but not turn 2's.
//!
//!     cargo run -p zeron-harness --example claude_branch_probe

use futures::StreamExt;
use tokio::sync::mpsc;
use zeron_harness::{CancellationToken, ClaudeHarness, Harness, RunControls};
use zeron_proto::{AgentEvent, AnchorRole, RunRequest, SandboxLevel, SessionBranch};

struct Outcome {
    session_id: Option<String>,
    prompt_anchor: Option<String>,
    reply_anchor: Option<String>,
    text: String,
    error: Option<String>,
}

async fn run(harness: &ClaudeHarness, request: RunRequest) -> Outcome {
    let (_steer_tx, steering) = mpsc::channel(8);
    let mut stream = harness
        .run(
            request,
            RunControls {
                realtime: None,
                execution_lease: None,
                request_input: Box::new(|_| panic!("probe must not ask for input")),
                steering,
                interrupt: CancellationToken::new(),
                tool_server: None,
                stop_tasks: None,
                turn: Default::default(),
            },
        )
        .await
        .expect("run starts");
    let mut out = Outcome {
        session_id: None,
        prompt_anchor: None,
        reply_anchor: None,
        text: String::new(),
        error: None,
    };
    loop {
        let ev =
            match tokio::time::timeout(std::time::Duration::from_secs(120), stream.next()).await {
                Ok(Some(ev)) => ev,
                Ok(None) => break,
                Err(_) => {
                    eprintln!("--- timed out");
                    std::process::exit(2);
                }
            };
        match ev {
            Ok(AgentEvent::SessionStarted { session_id, .. }) => out.session_id = Some(session_id),
            Ok(AgentEvent::Anchor { role, id, .. }) => match role {
                AnchorRole::User => out.prompt_anchor = Some(id),
                AnchorRole::Assistant => out.reply_anchor = Some(id),
            },
            Ok(AgentEvent::TextDelta { text }) => out.text.push_str(&text),
            Ok(AgentEvent::Done {
                error, session_id, ..
            }) => {
                out.error = error;
                if out.session_id.is_none() {
                    out.session_id = session_id;
                }
                break;
            }
            Ok(_) => {}
            Err(err) => {
                out.error = Some(err.to_string());
                break;
            }
        }
    }
    out
}

fn request(prompt: &str, cwd: &str) -> RunRequest {
    RunRequest {
        prompt: prompt.into(),
        harness: None,
        model: Some("haiku".into()),
        reasoning: None,
        model_options: serde_json::Map::new(),
        cwd: cwd.into(),
        sandbox: SandboxLevel::ReadOnly,
        access: zeron_proto::AccessMode::FullAccess,
        plan: false,
        auto_approve: false,
        attachments: Vec::new(),
        resume: None,
        worktree: None,
        branch: None,
        forked_at: None,
        computer_control: false,
        mcp: None,
        disabled_tools: Vec::new(),
        env: Default::default(),
    }
}

#[tokio::main]
async fn main() {
    let workspace = tempfile::tempdir().expect("create isolated probe workspace");
    let cwd = workspace.path().to_str().expect("UTF-8 workspace path");
    let harness = ClaudeHarness::new();

    let one = run(
        &harness,
        request("The first codeword is PELICAN. Reply with exactly: OK", cwd),
    )
    .await;
    println!(
        "turn 1: session={:?} prompt_anchor={:?} reply_anchor={:?} err={:?}\n  {}",
        one.session_id,
        one.prompt_anchor,
        one.reply_anchor,
        one.error,
        one.text.trim()
    );
    let session = one.session_id.clone().expect("turn 1 session id");
    let keep = one.reply_anchor.clone().expect("turn 1 reply anchor");

    let mut second = request("The second codeword is WALRUS. Reply with exactly: OK", cwd);
    second.resume = Some(session.clone());
    let two = run(&harness, second).await;
    println!(
        "turn 2: session={:?} prompt_anchor={:?} reply_anchor={:?} err={:?}\n  {}",
        two.session_id,
        two.prompt_anchor,
        two.reply_anchor,
        two.error,
        two.text.trim()
    );
    let drop = two.prompt_anchor.clone();

    let mut third = request(
        "List every codeword you were given in this conversation, comma-separated, nothing else.",
        cwd,
    );
    third.resume = Some(session.clone());
    third.branch = Some(SessionBranch {
        keep: Some(keep),
        drop,
    });
    let three = run(&harness, third).await;
    println!(
        "branch: session={:?} err={:?}\n  {}",
        three.session_id,
        three.error,
        three.text.trim()
    );

    let mut original = request(
        "List every codeword you were given in this conversation, comma-separated, nothing else.",
        cwd,
    );
    original.resume = Some(session.clone());
    let four = run(&harness, original).await;
    println!(
        "original after branch: session={:?} err={:?}\n  {}",
        four.session_id,
        four.error,
        four.text.trim()
    );

    let reply = three.text.to_uppercase();
    let ok = three.error.is_none()
        && three.session_id.as_deref().is_some_and(|id| id != session)
        && reply.contains("PELICAN")
        && !reply.contains("WALRUS")
        && four.text.to_uppercase().contains("WALRUS");
    println!("{}", if ok { "PASS" } else { "FAIL" });
    std::process::exit(if ok { 0 } else { 1 });
}
