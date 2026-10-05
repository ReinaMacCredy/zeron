use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{GitFileState, HarnessId, WorkspaceTarget};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GitAgentBusyPolicy {
    #[default]
    SafeOnly,
    BlockAll,
    AllowAll,
}

impl GitAgentBusyPolicy {
    pub fn allows(self, action: &WorkingTreeAction) -> bool {
        match self {
            Self::SafeOnly => action.safe_while_agent_runs(),
            Self::BlockAll => false,
            Self::AllowAll => true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetWorkingTreeRequest {
    #[serde(flatten)]
    pub target: WorkspaceTarget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkingTree {
    pub checkout_id: String,
    pub root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
    #[serde(default)]
    pub ahead: u32,
    #[serde(default)]
    pub behind: u32,
    pub revision: String,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub agent_active: bool,
    #[serde(default)]
    pub files: Vec<WorkingTreeFile>,
    #[serde(default)]
    pub stashes: Vec<StashEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkingTreeFile {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    pub index: GitFileState,
    pub worktree: GitFileState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub staged: Option<LineDelta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unstaged: Option<LineDelta>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LineDelta {
    pub additions: u32,
    pub deletions: u32,
    #[serde(default)]
    pub binary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StashEntry {
    pub oid: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetWorkingTreeFileDiffRequest {
    #[serde(flatten)]
    pub target: WorkspaceTarget,
    pub path: String,
    pub staged: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkingTreeFileDiff {
    pub checkout_id: String,
    pub path: String,
    pub staged: bool,
    pub patch: String,
    #[serde(default)]
    pub binary: bool,
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunWorkingTreeActionRequest {
    #[serde(flatten)]
    pub target: WorkspaceTarget,
    pub expected_revision: String,
    #[serde(default)]
    pub busy_policy: GitAgentBusyPolicy,
    pub action: WorkingTreeAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum WorkingTreeAction {
    Stage {
        paths: Vec<String>,
    },
    Unstage {
        paths: Vec<String>,
    },
    StageAll,
    UnstageAll,
    Commit {
        message: String,
        #[serde(default)]
        amend: bool,
        #[serde(default)]
        all: bool,
    },
    Discard {
        paths: Vec<String>,
    },
    StashPush {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    StashPop {
        oid: String,
    },
    StashDrop {
        oid: String,
    },
    #[serde(rename_all = "camelCase")]
    Checkout {
        target_ref: String,
    },
    CherryPick {
        sha: String,
    },
    AbortCherryPick,
    #[serde(rename_all = "camelCase")]
    CreateBranch {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start_point: Option<String>,
        #[serde(default)]
        checkout: bool,
    },
}

impl WorkingTreeAction {
    pub fn safe_while_agent_runs(&self) -> bool {
        matches!(
            self,
            Self::Stage { .. }
                | Self::Unstage { .. }
                | Self::StageAll
                | Self::UnstageAll
                | Self::Commit {
                    amend: false,
                    all: false,
                    ..
                }
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkingTreeActionReply {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<CommitSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitSummary {
    pub sha: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub subject: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncBranchRequest {
    #[serde(flatten)]
    pub target: WorkspaceTarget,
    pub expected_head_sha: String,
    #[serde(default)]
    pub busy_policy: GitAgentBusyPolicy,
    pub direction: SyncDirection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SyncDirection {
    #[serde(rename_all = "camelCase")]
    Push {
        #[serde(default)]
        set_upstream: bool,
    },
    Pull,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncBranchReply {
    pub head_sha: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
    #[serde(default)]
    pub ahead: u32,
    #[serde(default)]
    pub behind: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateCommitMessageRequest {
    #[serde(flatten)]
    pub target: WorkspaceTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<HarnessId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedCommitMessage {
    pub message: String,
    pub harness: HarnessId,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateChangeRequestRequest {
    #[serde(flatten)]
    pub target: WorkspaceTarget,
    pub host: String,
    pub repository: String,
    pub head: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub draft: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedChangeRequest {
    pub host: String,
    pub repository: String,
    pub number: u64,
    pub url: String,
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use serde::de::DeserializeOwned;
    use serde_json::json;

    use super::*;

    fn round_trip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(
        value: &T,
    ) -> serde_json::Value {
        let json = serde_json::to_value(value).unwrap();
        let back: T = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(&back, value);
        json
    }

    fn space_target() -> WorkspaceTarget {
        WorkspaceTarget {
            chat_id: None,
            space_id: Some("space-1".into()),
            checkout_path: Some("/tmp/repo/.maestro/worktree/a".into()),
        }
    }

    #[test]
    fn busy_policy_defaults_to_safe_only_and_is_camel_case() {
        assert_eq!(GitAgentBusyPolicy::default(), GitAgentBusyPolicy::SafeOnly);
        for (policy, wire) in [
            (GitAgentBusyPolicy::SafeOnly, "safeOnly"),
            (GitAgentBusyPolicy::BlockAll, "blockAll"),
            (GitAgentBusyPolicy::AllowAll, "allowAll"),
        ] {
            assert_eq!(round_trip(&policy), json!(wire));
        }
    }

    #[test]
    fn each_busy_policy_gates_actions_while_an_agent_runs() {
        let commit = |amend: bool, all: bool| WorkingTreeAction::Commit {
            message: "m".into(),
            amend,
            all,
        };
        let safe = [
            WorkingTreeAction::Stage {
                paths: vec!["a".into()],
            },
            WorkingTreeAction::Unstage {
                paths: vec!["a".into()],
            },
            WorkingTreeAction::StageAll,
            WorkingTreeAction::UnstageAll,
            commit(false, false),
        ];
        let unsafe_actions = [
            commit(false, true),
            commit(true, false),
            WorkingTreeAction::Discard {
                paths: vec!["a".into()],
            },
            WorkingTreeAction::StashPush { message: None },
            WorkingTreeAction::StashPop { oid: "o".into() },
            WorkingTreeAction::StashDrop { oid: "o".into() },
            WorkingTreeAction::Checkout {
                target_ref: "main".into(),
            },
            WorkingTreeAction::CherryPick { sha: "s".into() },
            WorkingTreeAction::AbortCherryPick,
            WorkingTreeAction::CreateBranch {
                name: "b".into(),
                start_point: None,
                checkout: false,
            },
        ];
        for action in &safe {
            assert!(GitAgentBusyPolicy::SafeOnly.allows(action), "{action:?}");
            assert!(!GitAgentBusyPolicy::BlockAll.allows(action), "{action:?}");
            assert!(GitAgentBusyPolicy::AllowAll.allows(action), "{action:?}");
        }
        for action in &unsafe_actions {
            assert!(!GitAgentBusyPolicy::SafeOnly.allows(action), "{action:?}");
            assert!(!GitAgentBusyPolicy::BlockAll.allows(action), "{action:?}");
            assert!(GitAgentBusyPolicy::AllowAll.allows(action), "{action:?}");
        }
    }

    #[test]
    fn working_tree_requests_flatten_the_target() {
        let json = round_trip(&GetWorkingTreeRequest {
            target: space_target(),
        });
        assert_eq!(
            json,
            json!({"spaceId": "space-1", "checkoutPath": "/tmp/repo/.maestro/worktree/a"})
        );
        let json = round_trip(&GetWorkingTreeFileDiffRequest {
            target: space_target(),
            path: "dir/a file.txt".into(),
            staged: true,
        });
        assert_eq!(json["path"], "dir/a file.txt");
        assert_eq!(json["staged"], true);
        assert_eq!(json["spaceId"], "space-1");
    }

    #[test]
    fn working_tree_round_trips_and_parses_a_minimal_reply() {
        let tree = WorkingTree {
            checkout_id: "checkout-1".into(),
            root: "/tmp/repo".into(),
            branch: Some("main".into()),
            head_sha: Some("abc123".into()),
            upstream: Some("origin/main".into()),
            ahead: 2,
            behind: 1,
            revision: "rev-1".into(),
            truncated: false,
            agent_active: true,
            files: vec![
                WorkingTreeFile {
                    path: "src/new name.rs".into(),
                    old_path: Some("src/old.rs".into()),
                    index: GitFileState::Renamed,
                    worktree: GitFileState::Modified,
                    staged: Some(LineDelta {
                        additions: 3,
                        deletions: 1,
                        binary: false,
                    }),
                    unstaged: Some(LineDelta {
                        additions: 1,
                        deletions: 0,
                        binary: false,
                    }),
                },
                WorkingTreeFile {
                    path: "logo.png".into(),
                    old_path: None,
                    index: GitFileState::Unchanged,
                    worktree: GitFileState::Untracked,
                    staged: None,
                    unstaged: Some(LineDelta {
                        binary: true,
                        ..Default::default()
                    }),
                },
            ],
            stashes: vec![StashEntry {
                oid: "def456".into(),
                message: "WIP on main: abc123 subject".into(),
                created_at: Some(Utc.with_ymd_and_hms(2026, 10, 5, 8, 0, 0).unwrap()),
            }],
        };
        let json = round_trip(&tree);
        assert_eq!(json["headSha"], "abc123");
        assert_eq!(json["agentActive"], true);
        assert_eq!(json["files"][0]["oldPath"], "src/old.rs");
        assert_eq!(json["files"][1]["worktree"], "untracked");
        assert!(json["files"][1].get("staged").is_none());

        let minimal: WorkingTree =
            serde_json::from_value(json!({"checkoutId": "c", "root": "/r", "revision": "r"}))
                .unwrap();
        assert_eq!(minimal.branch, None);
        assert_eq!((minimal.ahead, minimal.behind), (0, 0));
        assert!(minimal.files.is_empty() && minimal.stashes.is_empty());
        assert!(!minimal.agent_active);
    }

    #[test]
    fn file_diff_round_trips() {
        let json = round_trip(&WorkingTreeFileDiff {
            checkout_id: "checkout-1".into(),
            path: "-leading dash".into(),
            staged: false,
            patch: "@@ -1 +1 @@\n-a\n+b\n".into(),
            binary: false,
            truncated: true,
        });
        assert_eq!(json["checkoutId"], "checkout-1");
        assert_eq!(json["truncated"], true);
    }

    #[test]
    fn every_working_tree_action_round_trips_with_a_kind_tag() {
        let actions = [
            (
                WorkingTreeAction::Stage {
                    paths: vec!["a b".into(), ":(glob)*".into()],
                },
                "stage",
            ),
            (
                WorkingTreeAction::Unstage {
                    paths: vec!["a".into()],
                },
                "unstage",
            ),
            (WorkingTreeAction::StageAll, "stageAll"),
            (WorkingTreeAction::UnstageAll, "unstageAll"),
            (
                WorkingTreeAction::Commit {
                    message: "Subject\n\nBody".into(),
                    amend: true,
                    all: false,
                },
                "commit",
            ),
            (
                WorkingTreeAction::Discard {
                    paths: vec!["a".into()],
                },
                "discard",
            ),
            (WorkingTreeAction::StashPush { message: None }, "stashPush"),
            (
                WorkingTreeAction::StashPush {
                    message: Some("before rebase".into()),
                },
                "stashPush",
            ),
            (
                WorkingTreeAction::StashPop { oid: "def".into() },
                "stashPop",
            ),
            (
                WorkingTreeAction::StashDrop { oid: "def".into() },
                "stashDrop",
            ),
            (
                WorkingTreeAction::Checkout {
                    target_ref: "feature/x".into(),
                },
                "checkout",
            ),
            (
                WorkingTreeAction::CherryPick { sha: "abc".into() },
                "cherryPick",
            ),
            (WorkingTreeAction::AbortCherryPick, "abortCherryPick"),
            (
                WorkingTreeAction::CreateBranch {
                    name: "feature/y".into(),
                    start_point: Some("abc".into()),
                    checkout: true,
                },
                "createBranch",
            ),
        ];
        for (action, kind) in actions {
            let request = RunWorkingTreeActionRequest {
                target: space_target(),
                expected_revision: "rev-1".into(),
                busy_policy: GitAgentBusyPolicy::BlockAll,
                action,
            };
            let json = round_trip(&request);
            assert_eq!(json["action"]["kind"], kind);
            assert_eq!(json["expectedRevision"], "rev-1");
            assert_eq!(json["busyPolicy"], "blockAll");
        }
        let json = serde_json::to_value(WorkingTreeAction::Checkout {
            target_ref: "main".into(),
        })
        .unwrap();
        assert_eq!(json["targetRef"], "main");
        let json = serde_json::to_value(WorkingTreeAction::CreateBranch {
            name: "b".into(),
            start_point: Some("abc".into()),
            checkout: false,
        })
        .unwrap();
        assert_eq!(json["startPoint"], "abc");
    }

    #[test]
    fn an_action_request_without_a_policy_reads_as_safe_only() {
        let request: RunWorkingTreeActionRequest = serde_json::from_value(json!({
            "spaceId": "space-1",
            "expectedRevision": "rev-1",
            "action": {"kind": "commit", "message": "Subject"},
        }))
        .unwrap();
        assert_eq!(request.busy_policy, GitAgentBusyPolicy::SafeOnly);
        assert_eq!(
            request.action,
            WorkingTreeAction::Commit {
                message: "Subject".into(),
                amend: false,
                all: false,
            }
        );
    }

    #[test]
    fn action_replies_are_objects_even_when_empty() {
        let json = round_trip(&WorkingTreeActionReply::default());
        assert_eq!(json, json!({}));
        let json = round_trip(&WorkingTreeActionReply {
            commit: Some(CommitSummary {
                sha: "abc".into(),
                branch: None,
                subject: "Subject".into(),
            }),
            conflicts: vec!["src/lib.rs".into()],
        });
        assert_eq!(json["commit"]["subject"], "Subject");
        assert_eq!(json["conflicts"][0], "src/lib.rs");
    }

    #[test]
    fn sync_branch_round_trips_both_directions() {
        for (direction, kind) in [
            (SyncDirection::Push { set_upstream: true }, "push"),
            (SyncDirection::Pull, "pull"),
        ] {
            let json = round_trip(&SyncBranchRequest {
                target: space_target(),
                expected_head_sha: "abc".into(),
                busy_policy: GitAgentBusyPolicy::default(),
                direction,
            });
            assert_eq!(json["direction"]["kind"], kind);
            assert_eq!(json["expectedHeadSha"], "abc");
        }
        let json = serde_json::to_value(SyncDirection::Push { set_upstream: true }).unwrap();
        assert_eq!(json["setUpstream"], true);
        let json = round_trip(&SyncBranchReply {
            head_sha: "abc".into(),
            upstream: Some("origin/feature".into()),
            ahead: 0,
            behind: 0,
        });
        assert_eq!(json["upstream"], "origin/feature");
    }

    #[test]
    fn commit_message_generation_round_trips() {
        let json = round_trip(&GenerateCommitMessageRequest {
            target: space_target(),
            instruction: Some("Write a short commit message for this staged diff.".into()),
            harness: Some(HarnessId::ClaudeCode),
            model: Some("haiku".into()),
        });
        assert_eq!(
            json["harness"],
            serde_json::to_value(HarnessId::ClaudeCode).unwrap()
        );
        let bare = round_trip(&GenerateCommitMessageRequest {
            target: space_target(),
            instruction: None,
            harness: None,
            model: None,
        });
        assert!(bare.get("harness").is_none() && bare.get("instruction").is_none());
        round_trip(&GeneratedCommitMessage {
            message: "Fix the thing".into(),
            harness: HarnessId::Codex,
            model: "gpt-5".into(),
        });
    }

    #[test]
    fn change_request_creation_round_trips() {
        let json = round_trip(&CreateChangeRequestRequest {
            target: space_target(),
            host: "github.com".into(),
            repository: "acme/zeron".into(),
            head: "feature/x".into(),
            base: None,
            title: "Title".into(),
            body: "Body".into(),
            draft: true,
        });
        assert!(json.get("base").is_none());
        assert_eq!(json["draft"], true);
        round_trip(&CreatedChangeRequest {
            host: "github.com".into(),
            repository: "acme/zeron".into(),
            number: 7,
            url: "https://github.com/acme/zeron/pull/7".into(),
        });
    }
}
