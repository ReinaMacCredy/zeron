//! Demo mode end to end: the offline dataset through the real registry and
//! session pipeline (view derivation, ledger commands, echo adoption,
//! streaming, the shared queue, questions, interrupts).

use std::sync::Arc;
use std::time::{Duration, Instant};

use zeron_client::events::NullListener;
use zeron_client::{
    ChatIndicator, Client, ClientConfig, Credentials, DemoFixture, DemoOptions, MessageRole,
    MessageStatus, SendOutcome, SendRequest, SidebarOrganization, SidebarSort, SidebarView,
    StreamSpeed, TranscriptScale,
};

fn demo(options: DemoOptions) -> (Client, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut config = ClientConfig::new("https://edge.invalid", dir.path());
    config.device_id = "ios-test".into();
    config.device_name = "Test iPhone".into();
    let client = Client::new(config, Credentials::Demo(options), Arc::new(NullListener)).unwrap();
    (client, dir)
}

fn fast() -> DemoOptions {
    DemoOptions {
        stream_speed: StreamSpeed::Fast,
        ..Default::default()
    }
}

fn wait_for(what: &str, timeout: Duration, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(start.elapsed() < timeout, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn ids(rows: &[Arc<zeron_client::SessionRow>]) -> Vec<&str> {
    rows.iter().map(|r| r.id.as_str()).collect()
}

#[test]
fn front_page_mirrors_the_desktop_sidebar() {
    let (client, _dir) = demo(fast());
    let ws = client.workspace();
    assert!(ws.synced && ws.pins_ready);
    assert_eq!(ids(&ws.front.pinned), ["chat-veil", "chat-picker"]);
    let sections: Vec<(&str, Vec<&str>)> = ws
        .front
        .sections
        .iter()
        .map(|s| (s.name.as_str(), ids(&s.sessions)))
        .collect();
    assert_eq!(
        sections,
        [
            ("P0", vec!["chat-tabs", "chat-errored"]),
            ("Mobile", vec!["chat-ios-scroll", "chat-ios-keyboard"]),
        ]
    );
    // Inbox order; archived and settled chats leave it, step threads and
    // agent-opened chats nest under their lead.
    assert_eq!(
        ids(&ws.front.recent),
        [
            "chat-lead-shelf",
            "chat-visuals",
            "chat-lead-ios",
            "chat-approve-test",
            "chat-lead-widget",
            "chat-approve-push",
            "chat-cjk",
            "chat-unsettled",
            "chat-agents",
            "chat-home",
            "chat-hermes",
            "chat-pi",
            "chat-antigravity",
            "chat-acp",
            "chat-seams",
            "chat-deploy",
            "chat-computer",
            "chat-btw",
            "chat-old-mini"
        ]
    );
    assert_eq!(ids(&ws.archived), ["chat-oklch", "chat-presence"]);
    assert_eq!(
        ids(ws.children("chat-veil")),
        ["chat-side-easing", "chat-side"]
    );
    assert!(ws.session("chat-btw").unwrap().sidechat_expired);

    let veil = ws.session("chat-veil").unwrap();
    assert_eq!(veil.indicator, ChatIndicator::Working);
    assert!(veil.working_since_ms.is_some());
    assert!(veil.pinned);
    assert_eq!(veil.project.as_ref().unwrap().name, "zeron");
    assert_eq!(veil.device_name.as_deref(), Some("MacBook Pro"));
    assert!(veil.device_online);
    assert_eq!(veil.model_label.as_deref(), Some("Fable 5"));
    assert_eq!(veil.branch.as_deref(), Some("veil-fade"));
    assert_eq!(veil.time_label, "now");
    assert_eq!(
        ws.session("chat-picker").unwrap().indicator,
        ChatIndicator::AwaitingInput
    );
    assert_eq!(
        ws.session("chat-errored").unwrap().indicator,
        ChatIndicator::Errored
    );
    assert_eq!(
        ws.session("chat-tabs").unwrap().section_id.as_deref(),
        Some("section-p0")
    );

    let prs = &ws.pull_requests;
    assert_eq!(ids(&prs.open), ["chat-veil", "chat-ios-scroll"]);
    assert_eq!(ids(&prs.merged), ["chat-picker"]);
    assert_eq!(ids(&prs.closed), ["chat-tabs"]);

    let zeron = ws.project("space-zeron").unwrap();
    assert_eq!(zeron.indicator, ChatIndicator::AwaitingInput);
    assert!(zeron.unseen_count >= 2);
    // Clones of one repository are one project, with one name and color.
    let clone = ws.project("space-zeron-vps").unwrap();
    assert_eq!(clone.group_key, zeron.group_key);
    assert_eq!(clone.group_name, "zeron");
    assert_eq!(clone.color_index, zeron.color_index);
    let cjk = ws.session("chat-cjk").unwrap();
    assert_eq!(cjk.project.as_ref().unwrap().color_index, zeron.color_index);
    assert_ne!(ws.project("space-edge").unwrap().group_key, zeron.group_key);
    assert_eq!(ids(&ws.projectless), ["chat-visuals", "chat-home"]);
    assert!(ws.devices.iter().any(|d| d.is_self && d.id == "ios-test"));
    assert!(!ws.device("dev-studio").unwrap().online);

    let hits = client.search("scroll", 5);
    assert_eq!(hits[0].session.id, "chat-ios-scroll");
}

#[test]
fn settled_shelf_uses_each_hosts_days_and_settle_writes() {
    let (client, _dir) = demo(fast());
    let ws = client.workspace();
    // MacBook Pro settles after 3 days, the Mac mini after 7; the studio keeps the default 3.
    assert_eq!(
        ids(&ws.front.settled),
        [
            "chat-settled",
            "chat-old-mac",
            "chat-blog",
            "chat-old-1",
            "chat-old-2",
            "chat-old-3",
            "chat-old-4",
            "chat-old-5",
            "chat-old-6",
            "chat-old-7",
            "chat-old-8",
            "chat-old-9"
        ]
    );
    assert!(ws.session("chat-old-mac").unwrap().settled);
    assert!(!ws.session("chat-old-mini").unwrap().settled);
    assert!(ids(&ws.front.recent).contains(&"chat-old-mini"));
    // An Unsettle anchors a 9-day-old chat in the Inbox as fresh activity.
    let recent = ids(&ws.front.recent);
    let at = |id: &str| recent.iter().position(|r| *r == id).unwrap();
    assert!(at("chat-unsettled") < at("chat-old-mini"));
    assert!(ws.front.groups.is_empty());

    client.set_session_settled("chat-deploy", true).unwrap();
    let ws = client.workspace();
    assert_eq!(ws.front.settled[0].id, "chat-deploy");
    assert!(!ids(&ws.front.recent).contains(&"chat-deploy"));
    client.set_session_settled("chat-old-mac", false).unwrap();
    let ws = client.workspace();
    assert!(!ws.session("chat-old-mac").unwrap().settled);
    assert_eq!(ws.front.recent[0].id, "chat-old-mac");
    // A live turn never settles.
    client.set_session_settled("chat-veil", true).unwrap();
    assert!(!client.workspace().session("chat-veil").unwrap().settled);
}

#[test]
fn step_threads_and_opened_chats_nest_under_their_lead() {
    let (client, _dir) = demo(fast());
    let ws = client.workspace();
    let nested = |lead: &str| ids(ws.front.nested.get(lead).map_or(&[][..], Vec::as_slice));
    assert_eq!(
        nested("chat-lead-shelf"),
        ["chat-shelf-3", "chat-shelf-4", "chat-shelf-2", "chat-shelf-1"]
    );
    assert_eq!(nested("chat-veil"), ["chat-side-easing", "chat-side"]);
    for lists in [&ws.front.recent, &ws.front.settled, &ws.front.pinned] {
        assert!(!ids(lists).iter().any(|id| id.starts_with("chat-shelf-") || id.starts_with("chat-side")));
    }
}

#[test]
fn organize_and_sort_follow_the_desktop_menu() {
    let (client, _dir) = demo(fast());
    let inbox = ids(&client.workspace().front.recent)
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>();
    let view = |organization, sort, active_first| SidebarView {
        organization,
        sort,
        active_first,
    };

    // Sort and Active first are inert under Inbox, as on the desktop.
    client.set_sidebar_view(view(SidebarOrganization::Inbox, SidebarSort::Created, true));
    assert_eq!(ids(&client.workspace().front.recent), inbox);

    // Other views drop the Settled shelf and take the chosen sort.
    client.set_sidebar_view(view(
        SidebarOrganization::InOneList,
        SidebarSort::LastUpdated,
        false,
    ));
    let ws = client.workspace();
    assert!(ws.front.settled.is_empty() && ws.front.groups.is_empty());
    assert!(ids(&ws.front.recent).contains(&"chat-old-9"));
    let updated = |r: &zeron_client::SessionRow| r.last_activity_ms;
    assert!(
        ws.front
            .recent
            .windows(2)
            .all(|w| updated(&w[0]) >= updated(&w[1]))
    );

    client.set_sidebar_view(view(
        SidebarOrganization::InOneList,
        SidebarSort::Created,
        true,
    ));
    let ws = client.workspace();
    let live = ws
        .front
        .recent
        .iter()
        .take_while(|r| r.indicator != ChatIndicator::Idle)
        .count();
    assert!(live > 0);
    let idle = &ws.front.recent[live..];
    assert!(idle.iter().all(|r| r.indicator == ChatIndicator::Idle));
    assert!(
        idle.windows(2)
            .all(|w| w[0].created_at_ms >= w[1].created_at_ms)
    );

    // By project: the projectless Chats group leads and groups partition the list.
    client.set_sidebar_view(view(
        SidebarOrganization::ByProject,
        SidebarSort::LastUpdated,
        false,
    ));
    let ws = client.workspace();
    assert_eq!(ws.front.groups[0].name, "Chats");
    assert_eq!(
        ws.front
            .groups
            .iter()
            .map(|g| g.sessions.len())
            .sum::<usize>(),
        ws.front.recent.len()
    );

    // By device: one group per host, named for the device.
    client.set_sidebar_view(view(
        SidebarOrganization::ByDevice,
        SidebarSort::LastUpdated,
        false,
    ));
    let ws = client.workspace();
    let names: Vec<&str> = ws.front.groups.iter().map(|g| g.name.as_str()).collect();
    assert!(names.contains(&"MacBook Pro"), "{names:?}");
    for group in &ws.front.groups {
        assert!(group.sessions.iter().all(|r| r.device_id == group.id));
    }
    assert_eq!(
        ws.front
            .groups
            .iter()
            .map(|g| g.sessions.len())
            .sum::<usize>(),
        ws.front.recent.len()
    );
    client.set_sidebar_view(SidebarView::default());

    let order = |client: &Client| {
        client
            .workspace()
            .front
            .sections
            .iter()
            .map(|s| s.name.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(order(&client), ["P0", "Mobile"]);
    client
        .move_section("section-mobile", None, Some("section-p0".into()))
        .unwrap();
    assert_eq!(order(&client), ["Mobile", "P0"]);
}

#[test]
fn sidebar_writes_reorder_the_front_page() {
    let (client, _dir) = demo(fast());
    client.pin_session("chat-home").unwrap();
    assert_eq!(
        ids(&client.workspace().front.pinned),
        ["chat-veil", "chat-picker", "chat-home"]
    );
    client
        .move_pin("chat-home", None, Some("chat-veil".into()))
        .unwrap();
    assert_eq!(
        ids(&client.workspace().front.pinned),
        ["chat-home", "chat-veil", "chat-picker"]
    );
    client.unpin_session("chat-veil").unwrap();
    let section = client.create_section("Later").unwrap();
    client
        .assign_section("chat-deploy", Some(section.clone()))
        .unwrap();
    let ws = client.workspace();
    let later = ws.front.sections.iter().find(|s| s.id == section).unwrap();
    assert_eq!(ids(&later.sessions), ["chat-deploy"]);
    assert!(!ids(&ws.front.recent).contains(&"chat-deploy"));
    // A section lists its members in their placed order, as desktop does,
    // and a placement out of Pinned unpins.
    let members = |client: &Client| {
        let ws = client.workspace();
        ids(&ws
            .front
            .sections
            .iter()
            .find(|s| s.id == section)
            .unwrap()
            .sessions)
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>()
    };
    client
        .place_session(
            "chat-home",
            Some(section.clone()),
            None,
            Some("chat-deploy".into()),
        )
        .unwrap();
    assert_eq!(members(&client), ["chat-home", "chat-deploy"]);
    assert!(!ids(&client.workspace().front.pinned).contains(&"chat-home"));
    client
        .place_session(
            "chat-home",
            Some(section.clone()),
            Some("chat-deploy".into()),
            None,
        )
        .unwrap();
    assert_eq!(members(&client), ["chat-deploy", "chat-home"]);
    // A drop into Pinned lands between its neighbours.
    client
        .pin_session_at("chat-home", None, Some("chat-picker".into()))
        .unwrap();
    assert_eq!(
        ids(&client.workspace().front.pinned),
        ["chat-home", "chat-picker"]
    );
    assert_eq!(members(&client), ["chat-deploy"]);
    client.set_section_collapsed(&section, true).unwrap();
    client.rename_section(&section, "Someday").unwrap();
    let ws = client.workspace();
    let later = ws.front.sections.iter().find(|s| s.id == section).unwrap();
    assert!(later.collapsed);
    assert_eq!(later.name, "Someday");
    client.delete_section(&section).unwrap();
    assert!(ids(&client.workspace().front.recent).contains(&"chat-deploy"));

    client.archive_session("chat-cjk").unwrap();
    assert!(ids(&client.workspace().archived).contains(&"chat-cjk"));
    client.unarchive_session("chat-cjk").unwrap();
    client.rename_session("chat-cjk", "Renamed").unwrap();
    assert_eq!(
        client.workspace().session("chat-cjk").unwrap().title,
        "Renamed"
    );
    assert!(client.workspace().session("chat-tabs").unwrap().unseen);
    client.mark_seen("chat-tabs");
    assert!(!client.workspace().session("chat-tabs").unwrap().unseen);
    client.delete_session("chat-blog").unwrap();
    assert!(client.workspace().session("chat-blog").is_none());
}

#[test]
fn send_streams_a_reply_and_adopts_the_echo() {
    let (client, _dir) = demo(fast());
    let session = client.open_session("chat-home").unwrap();
    // On screen: the finished turn is stamped seen (Idle, not Completed).
    session.set_view_attached(true);
    let before = session.snapshot();
    assert!(before.hydrated);
    assert_eq!(before.transcript_len, 5);
    let first_message = before.entries[0].message.clone();

    let outcome = session
        .send(SendRequest::text("Explain the pipeline"))
        .unwrap();
    let SendOutcome::Started { message_id } = outcome else {
        panic!("idle chat starts a turn: {outcome:?}");
    };
    // Optimistic echo, instantly.
    let echo = session.snapshot();
    assert_eq!(echo.pending.len(), 1);
    assert_eq!(echo.pending[0].message_id, message_id);
    assert!(echo.entry(&message_id).unwrap().echo.is_some());
    assert!(
        !echo.working,
        "a pending send is not confirmed host activity"
    );

    let (tx, rx) = std::sync::mpsc::channel();
    let _watch = session.watch(move |snap| {
        let _ = tx.send(snap.revision);
    });
    assert!(
        rx.recv_timeout(Duration::from_secs(1)).is_ok(),
        "watch delivers the current snapshot"
    );
    wait_for("adoption", Duration::from_secs(5), || {
        session
            .snapshot()
            .entry(&message_id)
            .is_some_and(|e| e.echo.is_none())
    });
    wait_for("reply completes", Duration::from_secs(20), || {
        let snap = session.snapshot();
        let last = snap.transcript().last().unwrap();
        last.message.role == MessageRole::Assistant
            && last.message.status == Some(MessageStatus::Complete)
            && !snap.working
    });
    let after = session.snapshot();
    assert!(rx.try_iter().count() > 0, "watch follows the stream");
    assert!(after.pending.is_empty());
    assert_eq!(after.transcript_len, 7);
    // Settled entries keep their shared message across every update.
    assert!(Arc::ptr_eq(&after.entries[0].message, &first_message));
    let reply = after.transcript().last().unwrap();
    assert!(reply.message.parts.len() >= 4, "reasoning + text + tools");
    // The row settles just after the transcript does.
    wait_for("row idle", Duration::from_secs(5), || {
        client.workspace().session("chat-home").unwrap().indicator == ChatIndicator::Idle
    });
    let row = client.workspace().session("chat-home").unwrap().clone();
    assert_eq!(row.preview.as_deref(), Some("| Stage | Per token |"));
}

#[test]
fn a_stopped_turn_offers_resume() {
    let (client, _dir) = demo(fast());
    let errored = client.open_session("chat-errored").unwrap();
    wait_for("resumable", Duration::from_secs(5), || errored.composer().live.resumable);
    let settled = client.open_session("chat-deploy").unwrap();
    wait_for("loaded", Duration::from_secs(5), || settled.snapshot().transcript_len > 0);
    assert!(!settled.composer().live.resumable);
}

#[test]
fn sends_while_working_park_on_the_queue_and_drain() {
    let (client, _dir) = demo(fast());
    let session = client.open_session("chat-deploy").unwrap();
    session.send(SendRequest::text("first")).unwrap();
    wait_for("working", Duration::from_secs(5), || {
        session.composer().live.indicator == ChatIndicator::Working && session.snapshot().streaming
    });
    let outcome = session.send(SendRequest::text("second")).unwrap();
    let SendOutcome::Queued { queue_id } = outcome else {
        panic!("busy chat queues: {outcome:?}");
    };
    assert_eq!(session.composer().queue.len(), 1);
    assert_eq!(session.composer().queue[0].id, queue_id);
    assert!(session.composer().queue[0].from_this_device);
    wait_for("queue drains into a turn", Duration::from_secs(30), || {
        session.composer().queue.is_empty() && session.snapshot().entry(&queue_id).is_some()
    });
    wait_for("idle", Duration::from_secs(30), || {
        !session.snapshot().working
    });
}

#[test]
fn questions_answer_through_respond_input() {
    let (client, _dir) = demo(fast());
    let session = client.open_session("chat-deploy").unwrap();
    session
        .send(SendRequest::text("please ?ask first"))
        .unwrap();
    wait_for("question", Duration::from_secs(10), || {
        session.composer().open_input.is_some()
    });
    // The host publishes the question, then flips the chat's status.
    wait_for("awaiting input", Duration::from_secs(5), || {
        client.workspace().session("chat-deploy").unwrap().indicator == ChatIndicator::AwaitingInput
    });
    let input = session.composer().open_input.clone().unwrap();
    session
        .respond_input(
            &input.request_id,
            vec![zeron_proto_answer(&input.questions[0].id, "iOS only")],
        )
        .unwrap();
    wait_for("answered", Duration::from_secs(10), || {
        session.composer().open_input.is_none() && !session.snapshot().working
    });
    let last = session.snapshot().transcript().last().unwrap().clone();
    assert!(format!("{:?}", last.message.parts).contains("iOS only"));
}

fn zeron_proto_answer(question_id: &str, label: &str) -> zeron_client::UserInputAnswer {
    zeron_client::UserInputAnswer {
        question_id: question_id.into(),
        labels: vec![label.into()],
    }
}

#[test]
fn interrupt_aborts_the_live_turn() {
    let (client, _dir) = demo(DemoOptions {
        stream_speed: StreamSpeed::Realistic,
        ..Default::default()
    });
    let session = client.open_session("chat-deploy").unwrap();
    session.send(SendRequest::text("long one")).unwrap();
    wait_for("streaming", Duration::from_secs(5), || {
        session.snapshot().streaming
    });
    session.interrupt().unwrap();
    wait_for("aborted", Duration::from_secs(5), || {
        let snap = session.snapshot();
        snap.transcript().last().unwrap().message.status == Some(MessageStatus::Aborted)
            && !snap.working
    });
}

#[test]
fn opening_the_live_chat_finishes_its_stream() {
    let (client, _dir) = demo(fast());
    let session = client.open_session("chat-veil").unwrap();
    assert!(session.snapshot().streaming);
    wait_for("veil completes", Duration::from_secs(20), || {
        !session.snapshot().streaming && !session.snapshot().working
    });
}

#[test]
fn synthetic_transcripts_scale() {
    let (client, _dir) = demo(DemoOptions {
        transcript_scale: TranscriptScale::Huge,
        ..fast()
    });
    let session = client.open_session("chat-veil").unwrap();
    assert_eq!(session.snapshot().transcript_len, 1200);
}

#[test]
fn empty_fixtures() {
    let (client, _dir) = demo(DemoOptions {
        fixture: DemoFixture::NoProjects,
        ..fast()
    });
    let ws = client.workspace();
    assert!(ws.projects.is_empty() && ws.front.recent.is_empty());
    assert!(ws.devices.iter().any(|d| d.is_execution_host));
    let (client, _dir) = demo(DemoOptions {
        fixture: DemoFixture::IosOnly,
        ..fast()
    });
    assert!(client.workspace().execution_devices().is_empty());
}

#[test]
fn new_sessions_are_born_on_chat2() {
    let (client, _dir) = demo(fast());
    let id = client
        .create_session(zeron_client::NewSession {
            target: zeron_client::SessionTarget::Project {
                space_id: "space-edge".into(),
            },
            config: None,
            branch: None,
            cwd: None,
            title: None,
        })
        .unwrap();
    let row = client.workspace().session(&id).unwrap().clone();
    assert_eq!(row.room_gen, 2);
    assert_eq!(row.title, "New session");
    assert_eq!(client.workspace().front.recent[0].id, id);
    let session = client.open_session(&id).unwrap();
    assert_eq!(session.snapshot().transcript_len, 0);
    session
        .send(SendRequest::text("Deploy the landing page"))
        .unwrap();
    wait_for(
        "titled from the first prompt",
        Duration::from_secs(5),
        || client.workspace().session(&id).unwrap().title == "Deploy the landing page",
    );
}

#[test]
fn warm_sessions_are_capped_and_preload_follows_the_front_page() {
    let (client, _dir) = demo(fast());
    client.preload_sessions();
    let open = client.open_session_ids();
    assert_eq!(open.len(), zeron_client::PRELOAD_CAP);
    for id in ["chat-veil", "chat-picker", "chat-tabs", "chat-errored"] {
        assert!(open.iter().any(|o| o == id), "{id} preloaded");
    }
    let attached = client.open_session("chat-home").unwrap();
    attached.set_view_attached(true);
    for id in [
        "chat-cjk",
        "chat-deploy",
        "chat-blog",
        "chat-ios-scroll",
        "chat-ios-keyboard",
    ] {
        client.open_session(id).unwrap();
        client.close_session(id);
    }
    let open = client.open_session_ids();
    assert!(open.len() <= zeron_client::WARM_SESSION_CAP + 3, "{open:?}");
    assert!(
        open.iter().any(|o| o == "chat-home"),
        "attached sessions stay"
    );
    assert!(
        open.iter().any(|o| o == "chat-veil"),
        "streaming sessions stay"
    );
    assert!(
        open.iter().any(|o| o == "chat-picker"),
        "sessions awaiting input stay"
    );
}

#[test]
fn offline_sends_queue_durably_and_deliver_on_recovery() {
    let (client, _dir) = demo(fast());
    let session = client.open_session("chat-deploy").unwrap();
    client.set_network_online(false);
    wait_for("graced offline", Duration::from_secs(8), || {
        client.connectivity().state == zeron_client::ConnectivityState::Offline
    });
    let SendOutcome::Started { message_id } =
        session.send(SendRequest::text("while offline")).unwrap()
    else {
        panic!("idle chat starts a turn");
    };
    std::thread::sleep(Duration::from_millis(400));
    let composer = session.composer();
    assert_eq!(composer.send_state, Some(zeron_client::SendState::Queued));
    assert!(composer.delivery_degraded);
    assert_eq!(
        client
            .workspace()
            .session("chat-deploy")
            .unwrap()
            .send_state,
        Some(zeron_client::SendState::Queued)
    );
    assert!(
        session
            .snapshot()
            .entry(&message_id)
            .unwrap()
            .echo
            .is_some()
    );
    client.set_network_online(true);
    wait_for("delivered after recovery", Duration::from_secs(5), || {
        session
            .snapshot()
            .entry(&message_id)
            .is_some_and(|e| e.echo.is_none())
    });
    wait_for("send state clears", Duration::from_secs(5), || {
        session.composer().send_state.is_none()
    });
}

#[test]
fn idle_chat_send_completes_and_stops_working() {
    let (client, _dir) = demo(fast());
    let session = client.open_session("chat-deploy").unwrap();
    session.set_view_attached(true);
    assert!(!session.snapshot().working);
    session
        .send(SendRequest::text(
            "Summarize the launch post in three bullets.",
        ))
        .unwrap();
    wait_for("reply streams", Duration::from_secs(5), || {
        session.snapshot().streaming
    });
    wait_for(
        "turn completes, working clears",
        Duration::from_secs(15),
        || {
            let snap = session.snapshot();
            let composer = session.composer();
            !snap.working
                && !snap.streaming
                && !composer.live.turn_running
                && !composer.live.can_interrupt
                && snap.transcript().last().unwrap().message.status == Some(MessageStatus::Complete)
        },
    );
    wait_for("row idle", Duration::from_secs(5), || {
        client.workspace().session("chat-deploy").unwrap().indicator == ChatIndicator::Idle
    });
}

#[test]
fn steering_while_busy_sends_a_steer_not_a_queue_row() {
    let (client, _dir) = demo(DemoOptions::default());
    let session = client.open_session("chat-home").unwrap();
    // The composer learns mid-turn steering from the host's live catalog.
    let host = session.composer().host.device_id.clone();
    zeron_client::runtime::shared().block_on(client.list_harnesses(&host));
    session.send(SendRequest::text("first")).unwrap();
    wait_for("turn running and steerable", Duration::from_secs(5), || {
        let composer = session.composer();
        composer.live.turn_running && composer.host.capabilities.mid_turn_steering == Some(true)
    });
    let outcome = session
        .send(SendRequest {
            busy: zeron_client::BusyPolicy::Steer,
            ..SendRequest::text("also this")
        })
        .unwrap();
    // A parked row's only exit on mobile was an interrupting Send now.
    assert!(
        matches!(outcome, SendOutcome::Steered { .. }),
        "steer, not queue: {outcome:?}"
    );
    assert!(session.composer().queue.is_empty());
}

#[test]
fn queued_row_primary_action_steers_into_the_live_turn() {
    let (client, _dir) = demo(DemoOptions::default());
    let session = client.open_session("chat-home").unwrap();
    session.send(SendRequest::text("first")).unwrap();
    wait_for("turn running and steerable", Duration::from_secs(5), || {
        let composer = session.composer();
        composer.live.turn_running && composer.host.capabilities.mid_turn_steering == Some(true)
    });
    let SendOutcome::Queued { queue_id } = session.send(SendRequest::text("second")).unwrap()
    else {
        panic!("a running turn parks the send on the queue");
    };
    let row = session.composer().queue[0].clone();
    assert_eq!(
        row.primary_action,
        Some(zeron_client::QueuePrimaryAction::Steer)
    );
    assert_eq!(row.image, None);
    let delivered = zeron_client::runtime::shared()
        .block_on(session.deliver_queued_now(&queue_id))
        .unwrap();
    assert!(delivered);
    wait_for(
        "steered row left the queue",
        Duration::from_secs(10),
        || session.composer().queue.is_empty() && session.snapshot().entry(&queue_id).is_some(),
    );
}

#[test]
fn queueing_while_busy_at_realistic_speed() {
    let (client, _dir) = demo(DemoOptions::default());
    let session = client.open_session("chat-home").unwrap();
    session.send(SendRequest::text("first")).unwrap();
    wait_for("turn running", Duration::from_secs(5), || {
        session.composer().live.turn_running
    });
    let SendOutcome::Queued { queue_id } = session.send(SendRequest::text("second")).unwrap()
    else {
        panic!("a running turn parks the send on the queue");
    };
    assert_eq!(session.composer().queue.len(), 1);
    // The queued row delivers once the first turn ends.
    wait_for("queued row delivered", Duration::from_secs(60), || {
        session.composer().queue.is_empty() && session.snapshot().entry(&queue_id).is_some()
    });
    wait_for("idle again", Duration::from_secs(60), || {
        !session.snapshot().working && !session.composer().live.turn_running
    });
}

#[test]
fn sends_to_an_offline_host_park_as_queued_not_working() {
    // chat-blog lives on the Mac Studio, which is offline in the demo.
    let (client, _dir) = demo(fast());
    let session = client.open_session("chat-blog").unwrap();
    assert!(!session.composer().host.online);
    let SendOutcome::Started { message_id } =
        session.send(SendRequest::text("Pick a tone ?ask")).unwrap()
    else {
        panic!("idle chat starts a turn");
    };
    std::thread::sleep(Duration::from_millis(400));
    let snap = session.snapshot();
    let composer = session.composer();
    assert_eq!(snap.pending[0].message_id, message_id);
    assert_eq!(snap.pending[0].state, zeron_client::SendState::Queued);
    assert!(!snap.working, "a parked send is not a running turn");
    assert!(!composer.live.turn_running && !composer.live.can_interrupt);
    assert!(composer.delivery_degraded);
    // The sidebar still shows the send in flight (desktop display_status_for).
    let row = client.workspace().session("chat-blog").unwrap().clone();
    assert_eq!(row.send_state, Some(zeron_client::SendState::Queued));
    assert_eq!(row.host_indicator, ChatIndicator::Idle);
}

/// A provider change goes through only where the chat's host switches
/// providers; a model change goes through anywhere.
#[test]
fn provider_switch_needs_a_host_that_switches() {
    let (client, _dir) = demo(fast());
    let switched = |chat: &str| {
        let mut config = client
            .session_config(chat)
            .expect("demo chats have a config");
        use zeron_proto::HarnessId;
        config.harness = if config.harness == HarnessId::Codex {
            HarnessId::ClaudeCode
        } else {
            HarnessId::Codex
        };
        config.model = None;
        (client.set_session_config(chat, &config), config.harness)
    };
    // The Mac advertises provider switching.
    let (result, harness) = switched("chat-veil");
    result.expect("a host that switches takes the change");
    assert_eq!(client.session_config("chat-veil").unwrap().harness, harness);
    // The studio's engine predates it: the chat keeps its provider.
    let before = client.session_config("chat-blog").unwrap();
    let (result, _) = switched("chat-blog");
    let error = result.expect_err("an older host refuses").to_string();
    assert!(error.contains("newer Zeron"), "{error}");
    assert_eq!(client.session_config("chat-blog").unwrap(), before);
    // A model change on the same provider is not a switch.
    let mut model = before.clone();
    model.model = Some("another-model".into());
    client.set_session_config("chat-blog", &model).unwrap();
}

#[test]
fn a_model_switch_on_a_read_chat_asks_then_marks_the_transcript() {
    let (client, _dir) = demo(fast());
    let session = client.open_session("chat-home").unwrap();
    session.set_view_attached(true);
    wait_for("hydrated", Duration::from_secs(5), || {
        session.snapshot().hydrated
    });
    let config = client.session_config("chat-home").unwrap();
    let current = config.model.clone().unwrap_or_default();
    assert!(
        client
            .model_switch_prompt("chat-home", &current, "A", "A")
            .is_none()
    );
    let prompt = client
        .model_switch_prompt("chat-home", "demo-next", "Opus", "Sonnet")
        .expect("a chat with history asks first");
    assert_eq!(prompt.title, "Switch model?");
    let mut next = config.clone();
    next.model = Some("demo-next".into());
    zeron_client::runtime::shared()
        .block_on(client.set_session_config_noted(
            "chat-home",
            &next,
            &prompt.note,
            &prompt.note_hint,
        ))
        .unwrap();
    assert_eq!(
        client.session_config("chat-home").unwrap().model.as_deref(),
        Some("demo-next")
    );
    wait_for("the switch note", Duration::from_secs(5), || {
        session.snapshot().transcript().last().is_some_and(|entry| {
            entry.role() == MessageRole::System
                && format!("{:?}", entry.parts()).contains("Model changed from Opus to Sonnet.")
        })
    });
}

#[test]
fn tasks_and_supervisors_merge_across_macs() {
    let (client, _dir) = demo(fast());
    let ws = client.workspace();
    let missions: Vec<&str> = ws.tasks.missions.iter().map(|t| t.id.as_str()).collect();
    assert!(missions.contains(&"task-ios-install"));
    let hosts: Vec<&str> = ws
        .supervisor
        .hosts
        .iter()
        .map(|h| h.host.name.as_str())
        .collect();
    assert_eq!(hosts.len(), 2);
    assert!(hosts.contains(&"MacBook Pro") && hosts.contains(&"Mac mini"));
    let tagged: std::collections::BTreeSet<&str> = ws
        .needs_you
        .iter()
        .filter(|n| n.supervised)
        .map(|n| n.host.name.as_str())
        .collect();
    assert_eq!(tagged.len(), 2);

    assert!(
        client
            .task_detail("task-ios-install")
            .unwrap()
            .row
            .plan_ready
    );
    zeron_client::runtime::shared()
        .block_on(client.approve_plan("task-ios-install"))
        .unwrap();
    wait_for("plan approved", Duration::from_secs(5), || {
        !client
            .task_detail("task-ios-install")
            .unwrap()
            .row
            .plan_ready
    });

    let supervised = |client: &Client| {
        client
            .workspace()
            .needs_you
            .iter()
            .filter(|n| n.supervised)
            .count()
    };
    let before = supervised(&client);
    zeron_client::runtime::shared()
        .block_on(client.resolve_supervisor_items(
            "dev-mac",
            vec![("item-testflight".into(), "Wait for the widget".into())],
        ))
        .unwrap();
    wait_for("item resolved", Duration::from_secs(5), || {
        supervised(&client) == before - 1
    });
}

#[test]
fn needs_you_keeps_a_failed_send_with_its_reason_until_it_goes_through() {
    let (client, _dir) = demo(fast());
    let settle = |client: &Client| {
        client
            .workspace()
            .needs_you
            .iter()
            .find(|n| n.key == "dev-mini/item:item-settle")
            .cloned()
    };
    wait_for("settle item", Duration::from_secs(5), || settle(&client).is_some());
    let entry = settle(&client).unwrap();
    assert_eq!(entry.host.name, "Mac mini");
    assert!(entry.task_id.is_none());
    assert_eq!(entry.asker, "Supervisor");

    let answer = || vec![("item-settle".to_string(), "Keep 7 days".to_string())];
    let first = zeron_client::runtime::shared()
        .block_on(client.resolve_supervisor_items("dev-mini", answer()));
    assert!(first.is_err());
    match settle(&client).unwrap().send {
        Some(zeron_client::ItemSend::Failed { choice, reason }) => {
            assert_eq!(choice, "Keep 7 days");
            assert!(reason.contains("did not answer in time"), "{reason}");
        }
        other => panic!("expected a failed send, got {other:?}"),
    }

    zeron_client::runtime::shared()
        .block_on(client.resolve_supervisor_items("dev-mini", answer()))
        .unwrap();
    wait_for("settle item resolved", Duration::from_secs(5), || {
        settle(&client).is_none()
    });
}

#[test]
fn a_large_idle_claude_chat_offers_compaction_until_it_compacts() {
    let (client, _dir) = demo(fast());
    let session = client.open_session("chat-home").unwrap();
    session.set_view_attached(true);
    wait_for("the resume offer", Duration::from_secs(5), || {
        session.composer().resume_compaction.is_some()
    });
    assert_eq!(session.composer().compact_blocked, None);
    let notice = zeron_client::runtime::shared()
        .block_on(session.compact(Some("auth".into())))
        .unwrap();
    assert_eq!(notice, None);
    wait_for("the boundary entry", Duration::from_secs(5), || {
        session.snapshot().transcript().last().is_some_and(|entry| {
            entry.role() == MessageRole::System
                && format!("{:?}", entry.parts()).contains("Context compacted")
        })
    });
    wait_for("the offer to go", Duration::from_secs(5), || {
        let composer = session.composer();
        composer.resume_compaction.is_none() && !composer.compacting
    });
}

#[test]
fn the_demo_catalog_offers_zeron_commands_and_skills_apart() {
    let (client, _dir) = demo(fast());
    let session = client.open_session("chat-home").unwrap();
    let host = session.composer().host.device_id.clone();
    let rt = zeron_client::runtime::shared();
    let commands = rt.block_on(client.invocation_catalog(
        &host,
        "claude-code",
        Some("chat-home"),
        None,
        false,
    ));
    let names: Vec<&str> = commands.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        &names[..4],
        ["btw", "sidechat", "compact", "computer-use"],
        "Zeron's own commands ride first"
    );
    assert!(names.contains(&"review") && !names.contains(&"pdf"));
    assert!(commands.warning.is_none());
    let skills = rt.block_on(client.invocation_catalog(&host, "claude-code", None, None, true));
    assert!(!skills.entries.is_empty());
    assert!(skills.entries.iter().all(|e| e.invocation.prefix() == '$'));
}

#[test]
fn an_open_chat_predicts_the_next_message_only_while_on_screen() {
    let (client, _dir) = demo(fast());
    let session = client.open_session("chat-home").unwrap();
    assert_eq!(session.composer().prediction, None);
    session.set_view_attached(true);
    assert_eq!(
        session.composer().prediction.as_deref(),
        Some("Run the iOS layout tests on the simulator")
    );
    session.set_view_attached(false);
    assert_eq!(session.composer().prediction, None);
}

#[test]
fn continuing_in_a_new_thread_lands_on_the_picked_agent() {
    let (client, _dir) = demo(fast());
    let session = client.open_session("chat-home").unwrap();
    session.set_view_attached(true);
    wait_for("hydrated", Duration::from_secs(5), || {
        session.snapshot().hydrated
    });
    assert!(!session.handoff_blocked());
    let rt = zeron_client::runtime::shared();
    let id = rt
        .block_on(client.continue_in_new_thread("chat-home", Some(("codex", "gpt-5.5")), None))
        .unwrap();
    assert_ne!(id, "chat-home");
    let config = client.session_config(&id).expect("the new chat is listed");
    assert_eq!(config.harness, zeron_proto::HarnessId::Codex);
    assert_eq!(config.model.as_deref(), Some("gpt-5.5"));
    assert_eq!(
        client.session_config("chat-home").unwrap().harness,
        zeron_proto::HarnessId::ClaudeCode
    );
}

#[test]
fn a_bare_btw_reopens_the_live_sidechat_and_a_prompt_starts_a_new_one() {
    let (client, _dir) = demo(fast());
    let rt = zeron_client::runtime::shared();
    let first = rt
        .block_on(client.open_sidechat("chat-home", None))
        .unwrap();
    assert_eq!(
        rt.block_on(client.open_sidechat("chat-home", None))
            .unwrap(),
        first
    );
    let second = rt
        .block_on(client.open_sidechat("chat-home", Some("Why 2ⁿ and not jitter?".into())))
        .unwrap();
    assert_ne!(second, first);
    let rows = client.sidechats_of("chat-home");
    assert_eq!(
        rows.iter().map(|r| r.chat_id.as_str()).collect::<Vec<_>>(),
        [&first, &second]
    );
    assert!(rows[0].time_left.ends_with("left of 24 h"));
    let sent = client.open_session(&second).unwrap();
    wait_for("the prompt is sent", Duration::from_secs(5), || {
        let snapshot = sent.snapshot();
        snapshot.pending.iter().any(|p| p.text.contains("jitter"))
            || snapshot
                .entries
                .iter()
                .any(|e| e.role() == zeron_doc::MessageRole::User)
    });
    assert!(rt.block_on(client.create_sidechat(&second)).is_err());
}

#[test]
fn the_roster_lists_subagents_and_shells_and_stops_them() {
    use zeron_proto::background::RunState;
    let (client, _dir) = demo(fast());
    let session = client.open_session("chat-home").unwrap();
    session.set_view_attached(true);
    wait_for("hydrated", Duration::from_secs(5), || {
        session.snapshot().hydrated
    });
    let roster = session.background_roster();
    let runs: Vec<_> = roster
        .agents
        .iter()
        .map(|run| (run.title.as_str(), run.state))
        .collect();
    assert_eq!(
        runs,
        [
            ("Check the sheet layout", RunState::Completed),
            ("Audit the composer pickers", RunState::Working),
            ("Measure the picker hit areas", RunState::Working),
            ("Trace the draft restore path", RunState::Working),
            ("Reproduce the composer jump when the key…", RunState::Failed),
        ]
    );
    assert_eq!(roster.agents[2].parent.as_deref(), Some("spawn-pickers"));
    assert_eq!(roster.harness, Some(zeron_proto::HarnessId::ClaudeCode));
    let compact = session.compact_section(0).unwrap();
    assert_eq!(compact.heading, "Idle compaction");
    assert_eq!(compact.rows.last().unwrap().0, "Compacts at");
    let shell = roster.shells[0].id.clone();
    assert_eq!(roster.shells[0].label, "cargo test -p zeron-mobile");
    session.watch_shell_output(Some(shell.clone()));
    assert!(
        session
            .shell_output(&shell)
            .iter()
            .any(|line| line.contains("running 214 tests"))
    );

    let rt = zeron_client::runtime::shared();
    rt.block_on(session.stop_background_run("spawn-pickers", false))
        .unwrap();
    wait_for("the subagent settles", Duration::from_secs(5), || {
        session.background_roster().agents[1].state == RunState::Stopped
    });
    rt.block_on(session.stop_background_run(&shell, false))
        .unwrap();
    assert!(session.background_roster().shells.is_empty());

    session.set_view_attached(false);
    assert!(session.shell_output(&shell).is_empty());
}

#[test]
fn the_model_menu_lists_instances_and_applies_profiles() {
    use zeron_client::model_menu::InstanceCatalog;
    let (client, _dir) = demo(fast());
    let session = client.open_session("chat-home").unwrap();
    session.set_view_attached(true);
    wait_for("hydrated", Duration::from_secs(5), || {
        session.snapshot().hydrated
    });
    let rt = zeron_client::runtime::shared();
    let device = client
        .workspace()
        .session("chat-home")
        .unwrap()
        .device_id
        .clone();
    let instances = rt
        .block_on(client.list_provider_instances(&device))
        .unwrap();
    let labels: Vec<_> = instances.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels, ["Claude · Personal", "Codex · Work", "Work", "Gemini CLI"]);
    assert_eq!(
        rt.block_on(client.list_instance_models(&device, "codex", "codex_work", true)),
        InstanceCatalog::Failed("timed out after 20s".into())
    );
    let InstanceCatalog::Ready(models) =
        rt.block_on(client.list_instance_models(&device, "claude-code", "claude_personal", false))
    else {
        panic!("a working instance lists its models");
    };
    assert!(!models.is_empty());

    let rows = rt.block_on(client.profile_rows("chat-home")).unwrap();
    let reviewer = &rows[0];
    assert_eq!(reviewer.name, "iOS reviewer");
    assert!(reviewer.active && reviewer.savable);
    assert_eq!(reviewer.drift.as_deref(), Some("effort High → Medium"));
    assert!(!rows[1].active && rows[1].drift.is_none());

    rt.block_on(client.save_chat_into_profile("chat-home", "ios-reviewer"))
        .unwrap();
    let rows = rt.block_on(client.profile_rows("chat-home")).unwrap();
    assert_eq!(rows[0].drift, None);

    let prompt = rt
        .block_on(client.profile_switch_prompt("chat-home", "planner"))
        .unwrap()
        .expect("a chat with history confirms a harness move");
    assert_eq!(prompt.title, "Switch provider?");
    rt.block_on(client.apply_agent_profile("chat-home", "planner", None))
        .unwrap();
    let config = client.session_config("chat-home").unwrap();
    assert_eq!(config.harness, zeron_proto::HarnessId::Codex);
    assert_eq!(config.profile_id.as_deref(), Some("planner"));
    assert_eq!(config.instructions, "Plan first; do not edit files.");
    client.detach_agent_profile("chat-home").unwrap();
    let config = client.session_config("chat-home").unwrap();
    assert_eq!(
        (config.profile_id, config.instructions),
        (None, String::new())
    );
}

#[test]
fn an_open_chat_leads_its_mission_or_a_new_lead_reads_it() {
    let (client, _dir) = demo(fast());
    let rt = zeron_client::runtime::shared();
    let link = |id: &str| zeron_proto::chat_mentions::chat_mention_link(id, id).unwrap();
    let draft = || zeron_client::TaskDraft {
        title: "Fix scrolling".into(),
        notes: format!(
            "See {} and {}",
            link("chat-ios-keyboard"),
            link("chat-deploy")
        ),
        ..Default::default()
    };
    let context = |task_id: &str| {
        client.task_detail(task_id).map(|detail| {
            detail
                .context
                .iter()
                .map(|c| c.chat_id.clone())
                .collect::<Vec<_>>()
        })
    };

    let reads = rt
        .block_on(client.start_mission_from_chat(
            "chat-ios-scroll",
            draft(),
            zeron_client::MissionOptions::default(),
            false,
        ))
        .unwrap();
    wait_for("new lead started", Duration::from_secs(5), || {
        client.task_detail(&reads.task_id).is_some_and(|detail| {
            detail.row.mission
                && detail
                    .row
                    .lead_chat_id
                    .as_deref()
                    .is_some_and(|lead| lead != "chat-ios-scroll")
        })
    });
    assert_eq!(
        context(&reads.task_id).unwrap(),
        ["chat-ios-scroll", "chat-ios-keyboard"],
        "the source first, then mentioned chats on its host"
    );

    let led = rt
        .block_on(client.start_mission_from_chat(
            "chat-ios-scroll",
            draft(),
            zeron_client::MissionOptions::default(),
            true,
        ))
        .unwrap();
    wait_for("chat leads", Duration::from_secs(5), || {
        client.task_detail(&led.task_id).is_some_and(|detail| {
            detail.row.mission && detail.row.lead_chat_id.as_deref() == Some("chat-ios-scroll")
        })
    });
    assert_eq!(context(&led.task_id).unwrap(), ["chat-ios-keyboard"]);
}

#[test]
fn phone_starts_edits_and_watches_missions() {
    let (client, _dir) = demo(fast());
    let rt = zeron_client::runtime::shared();

    let draft = zeron_client::TaskDraft {
        title: "Ship the widget".into(),
        notes: "Use the shared layout.".into(),
        ..Default::default()
    };
    let task_id = rt
        .block_on(client.start_mission(
            "dev-mac",
            draft,
            None,
            zeron_client::MissionOptions::default(),
        ))
        .unwrap();
    wait_for("mission started", Duration::from_secs(5), || {
        client
            .task_detail(&task_id)
            .is_some_and(|detail| detail.row.mission && detail.row.lead_chat_id.is_some())
    });

    let plain = rt
        .block_on(client.create_task(
            "dev-mac",
            zeron_client::TaskDraft {
                title: "Hand me off".into(),
                ..Default::default()
            },
        ))
        .unwrap();
    wait_for("task created", Duration::from_secs(5), || {
        client.task_detail(&plain).is_some()
    });
    rt.block_on(client.update_task(
        &plain,
        zeron_proto::TaskPatch {
            context: Some(vec!["chat-picker".into()]),
            ..Default::default()
        },
    ))
    .unwrap();
    rt.block_on(client.assign_task(&plain, "chat-veil", false))
        .unwrap();
    wait_for("assigned with context", Duration::from_secs(5), || {
        client.task_detail(&plain).is_some_and(|detail| {
            detail.row.lead_chat_id.as_deref() == Some("chat-veil")
                && detail
                    .context
                    .iter()
                    .map(|c| c.chat_id.as_str())
                    .eq(["chat-picker"])
        })
    });

    let detail = client.task_detail("task-ios-install").unwrap();
    let first = detail.steps[0].id.clone();
    let second = detail.steps[1].id.clone();
    let edit = zeron_proto::PlanEdit {
        steps: vec![
            zeron_proto::PlanStepEdit {
                step_id: Some(first.clone()),
                title: "Renamed first step".into(),
                ..Default::default()
            },
            zeron_proto::PlanStepEdit {
                title: "A new last step".into(),
                ..Default::default()
            },
        ],
        order: detail
            .steps
            .iter()
            .map(|s| s.id.clone())
            .filter(|id| *id != first)
            .chain([first.clone()])
            .collect(),
        ..Default::default()
    };
    rt.block_on(client.edit_task_plan("task-ios-install", Some(edit), true))
        .unwrap();
    wait_for("plan edited and approved", Duration::from_secs(5), || {
        let detail = client.task_detail("task-ios-install").unwrap();
        !detail.row.plan_ready
            && detail.steps[0].id == second
            && detail
                .steps
                .iter()
                .any(|s| s.id == first && s.title == "Renamed first step")
            && detail.steps.last().unwrap().title == "A new last step"
    });

    let watch = client.watch_state("chat-lead-ios").unwrap();
    assert!(watch.mission && watch.watched);
    rt.block_on(client.set_watched("chat-lead-ios", false))
        .unwrap();
    wait_for("mission unwatched", Duration::from_secs(5), || {
        !client.watch_state("chat-lead-ios").unwrap().watched
    });
    rt.block_on(client.set_watched("chat-veil", true)).unwrap();
    wait_for("tree watched", Duration::from_secs(5), || {
        client
            .watch_state("chat-veil")
            .is_some_and(|w| !w.mission && w.watched)
    });

    let brief = client.brief("task-ios-install", None).unwrap();
    assert_eq!(brief.thread_chat_id.as_deref(), Some("chat-lead-ios"));
    let messages = rt.block_on(client.brief_messages(&brief)).unwrap();
    assert!(
        messages
            .first()
            .is_some_and(|m| m.from_you && m.tag == "From you")
    );
}

#[test]
fn phone_stops_resumes_and_starts_supervisors() {
    let (client, _dir) = demo(fast());
    let rt = zeron_client::runtime::shared();
    let host = |client: &Client| {
        client
            .workspace()
            .supervisor
            .hosts
            .iter()
            .find(|h| h.host.device_id == "dev-mac")
            .cloned()
            .unwrap()
    };
    assert_eq!(host(&client).chat_id.as_deref(), Some("supervisor-dev-mac"));

    rt.block_on(client.stop_supervisor("dev-mac")).unwrap();
    wait_for("stopped", Duration::from_secs(5), || {
        let h = host(&client);
        h.state == zeron_proto::supervisor::SupervisorState::Stopped && h.waiting > 0
    });
    rt.block_on(client.resume_supervisor("dev-mac")).unwrap();
    wait_for("resumed", Duration::from_secs(5), || {
        let h = host(&client);
        h.state != zeron_proto::supervisor::SupervisorState::Stopped && h.waiting == 0
    });

    let err = rt.block_on(client.ensure_supervisor("dev-mini", None));
    assert!(err.is_err());
    let chat = rt
        .block_on(client.ensure_supervisor(
            "dev-mini",
            Some(zeron_proto::ChatConfig {
                harness: zeron_proto::HarnessId::ClaudeCode,
                model: Some("haiku".into()),
                reasoning: None,
                model_options: Default::default(),
                sandbox: zeron_proto::SandboxLevel::WorkspaceWrite,
                instructions: String::new(),
                profile_id: None,
                instance_id: None,
                disabled_tools: Vec::new(),
                access: Default::default(),
                plan: false,
            }),
        ))
        .unwrap();
    assert_eq!(chat, "supervisor-dev-mini");
}

#[test]
fn plan_estimate_prices_children_against_the_lead() {
    let (client, _dir) = demo(fast());
    let rt = zeron_client::runtime::shared();
    let estimate = rt
        .block_on(client.plan_estimate("task-ios-install"))
        .unwrap();
    assert_eq!(estimate.lines.len(), 6);
    assert_eq!(estimate.lines[0].multiplier.as_deref(), Some("0.4x"));
    assert!(estimate.lines[0].runs_on.starts_with("Codex"));
    let validator = &estimate.lines[2];
    assert_eq!(validator.role.as_deref(), Some("validator"));
    assert_eq!(validator.step_id, "task-ios-install-s3");
    assert_eq!(validator.multiplier.as_deref(), Some("0.6x"));
    assert!(
        validator
            .fallback
            .as_deref()
            .is_some_and(|f| f.contains("Codex out of quota"))
    );
    assert!(estimate.lines[5].runs_on.starts_with("agent picks"));
    assert_eq!(estimate.cost, "Relative cost 3.2x");
    assert!(
        estimate
            .raised
            .as_deref()
            .is_some_and(|r| r.contains("3x → 3.2x"))
    );
    assert!(estimate.baseline.starts_with("vs 6.0x"));
}

#[test]
fn today_card_merges_both_macs_runs_tasks_and_tightest_usage() {
    let (client, _dir) = demo(fast());
    let before = client.workspace().today.clone();
    assert!(!before.runs_loaded);
    assert!(before.usage.is_none());
    zeron_client::runtime::shared().block_on(client.refresh_today());
    let today = client.workspace().today.clone();
    assert!(today.runs_loaded);
    assert!(today.prompts > 7, "runs from both Macs: {}", today.prompts);
    assert!(!today.greeting.is_empty());
    assert!(today.summary.ends_with("to do."), "{}", today.summary);

    let prs = today.pull_requests.clone().expect("PR attention row");
    assert_eq!(
        (prs.title.as_str(), prs.meta.as_str(), prs.count, prs.device_id.as_str()),
        (
            "3 pull requests need you",
            "GitHub, 2 to review · 1 need changes · 1 failing CI",
            3,
            "dev-mac"
        )
    );
    assert!(today.summary.contains("things need you"), "{}", today.summary);

    let usage = today.usage.expect("tightest window");
    assert_eq!(
        (usage.harness.as_str(), usage.window.as_str(), usage.host.device_id.as_str()),
        ("claude-code", "Session", "dev-mac")
    );
    assert_eq!(usage.resets_in.as_deref(), Some("2h 5m"));

    let task = |id: &str| today.tasks.iter().find(|t| t.id == id).cloned();
    let signin = task("task-signin-fix").expect("linked task");
    let picker = client.workspace().session("chat-picker").unwrap().title.clone();
    assert_eq!(signin.meta, Some(format!("In {picker}")));
    assert_eq!(signin.host.map(|h| h.device_id), Some("dev-mac".into()));
    let done = task("task-notes-115").expect("done today");
    assert!(done.done && done.meta.is_none());
    let docs = task("task-relay-docs").expect("due yesterday");
    assert_eq!(docs.meta.as_deref(), Some(if docs.overdue { "Overdue" } else { "Today" }));
    assert_eq!(
        today.open_tasks as usize,
        today.tasks.iter().filter(|t| !t.done).count()
    );
    assert!(before.updated.is_none());
    assert!(today.updated.as_deref().is_some_and(|u| u.starts_with("Updated ")));
}

#[test]
fn today_add_task_is_due_today() {
    let (client, _dir) = demo(fast());
    let rt = zeron_client::runtime::shared();
    assert!(rt.block_on(client.add_today_task("  ")).is_err());
    let id = rt
        .block_on(client.add_today_task(" Ship the Today sheet "))
        .unwrap();
    wait_for("the new task", Duration::from_secs(5), || {
        client.workspace().today.tasks.iter().any(|t| t.id == id)
    });
    let today = client.workspace().today.clone();
    let task = today.tasks.iter().find(|t| t.id == id).unwrap();
    assert_eq!(task.title, "Ship the Today sheet");
    assert_eq!(task.meta.as_deref(), Some("Today"));
}

#[test]
fn a_provider_added_on_the_phone_is_edited_and_an_acp_agent_installs() {
    let (client, _dir) = demo(fast());
    let host = client
        .open_session("chat-home")
        .unwrap()
        .composer()
        .host
        .device_id
        .clone();
    let rt = zeron_client::runtime::shared();
    let row = |page: &zeron_client::host_settings::Page, id: &str| {
        page.sections
            .iter()
            .flat_map(|s| &s.rows)
            .find(|r| r.id == id)
            .cloned()
    };

    let added = rt
        .block_on(client.host_action(&host, "add-provider", "add:codex", None))
        .unwrap();
    let codex = added.page_id.unwrap();
    rt.block_on(client.host_action(&host, &codex, "label", Some("Codex Work".into())))
        .unwrap();
    let page = rt.block_on(client.host_page(&host, &codex)).unwrap();
    assert_eq!(
        row(&page, "label").unwrap().value.as_deref(),
        Some("Codex Work")
    );

    let search = rt
        .block_on(client.host_page(&host, "acp-search:goose"))
        .unwrap();
    let goose = row(&search, "prepare:goose").unwrap();
    assert!(goose.actions.iter().any(|a| a.confirm.is_some()));
    let installed = rt
        .block_on(client.host_action(&host, "acp-search:goose", "prepare:goose", None))
        .unwrap();
    let goose = installed.page_id.unwrap();

    let providers = rt.block_on(client.host_page(&host, "providers")).unwrap();
    assert!(row(&providers, &codex).is_some());
    assert!(row(&providers, &goose).is_some());
}

#[test]
fn profiles_reorder_and_tool_switches_write_the_profile() {
    let (client, _dir) = demo(fast());
    let host = client
        .open_session("chat-home")
        .unwrap()
        .composer()
        .host
        .device_id
        .clone();
    let rt = zeron_client::runtime::shared();
    let page = |id: &str| rt.block_on(client.host_page(&host, id)).unwrap();
    let rows = |page: &zeron_client::host_settings::Page| -> Vec<(String, Option<String>)> {
        page.sections
            .iter()
            .flat_map(|s| &s.rows)
            .map(|r| (r.id.clone(), r.value.clone()))
            .collect()
    };
    let order = |page: &zeron_client::host_settings::Page| -> Vec<String> {
        page.sections[0]
            .rows
            .iter()
            .filter_map(|r| r.id.strip_prefix("profile:").map(str::to_owned))
            .collect()
    };

    assert_eq!(
        order(&page("profiles")),
        ["ios-reviewer", "planner", "ui-work", "reviewer", "quick-fix"]
    );
    rt.block_on(client.host_action(&host, "profiles", "down:ui-work", None))
        .unwrap();
    assert_eq!(
        order(&page("profiles")),
        ["ios-reviewer", "planner", "reviewer", "ui-work", "quick-fix"]
    );

    let tools = page("profile:ui-work:tools");
    let ids = rows(&tools);
    assert!(
        ids.iter()
            .any(|(id, _)| id == "unlisted:mcp__retired_server")
    );
    assert!(!ids.iter().any(|(id, _)| id == "unlisted:WebSearch"));
    rt.block_on(client.host_action(
        &host,
        "profile:ui-work:tools",
        "builtin:web",
        Some("true".into()),
    ))
    .unwrap();
    rt.block_on(client.host_action(
        &host,
        "profile:ui-work:tools",
        "unlisted:mcp__retired_server",
        None,
    ))
    .unwrap();
    rt.block_on(client.host_action(
        &host,
        "profile:ui-work:tools:computer",
        "group",
        Some("true".into()),
    ))
    .unwrap();
    let editor = rows(&page("profile:ui-work"));
    assert!(editor.contains(&("tools".into(), Some("2 tools off".into()))));
}

#[test]
fn a_row_names_the_background_work_its_host_reports() {
    let (client, _dir) = demo(fast());
    let ws = client.workspace();
    let ring = |id: &str| {
        let row = &ws.sessions[id];
        (row.background_ring, row.background_label.as_deref())
    };
    assert_eq!(ring("chat-home"), (Some(1), None));
    assert_eq!(ring("chat-veil"), (Some(1), Some("1 shell")));
    assert_eq!(ring("chat-ios-scroll"), (None, None));
}

#[test]
fn a_new_chat_draft_takes_and_drops_a_profile() {
    let (client, _dir) = demo(fast());
    let rt = zeron_client::runtime::shared();
    let device = client.workspace().sessions["chat-home"].device_id.clone();
    let draft = zeron_proto::ChatConfig {
        harness: zeron_proto::HarnessId::ClaudeCode,
        model: None,
        reasoning: None,
        model_options: Default::default(),
        sandbox: zeron_proto::SandboxLevel::WorkspaceWrite,
        instructions: String::new(),
        profile_id: None,
        instance_id: None,
        disabled_tools: Vec::new(),
        access: Default::default(),
        plan: false,
    };
    let rows = rt
        .block_on(client.draft_profile_rows(&device, draft.clone()))
        .unwrap();
    assert!(rows.iter().all(|row| !row.active));

    let planned = rt
        .block_on(client.apply_draft_profile(&device, draft, "planner"))
        .unwrap();
    assert_eq!(planned.harness, zeron_proto::HarnessId::Codex);
    assert_eq!(planned.profile_id.as_deref(), Some("planner"));
    assert_eq!(planned.instructions, "Plan first; do not edit files.");
    assert_eq!(planned.access, zeron_proto::AccessMode::Supervised);
    let rows = rt
        .block_on(client.draft_profile_rows(&device, planned))
        .unwrap();
    let planner = rows.iter().find(|row| row.id == "planner").unwrap();
    assert!(planner.active && planner.drift.is_none());
}
