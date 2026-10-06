use super::*;
use gpui::{AppContext, TestAppContext, VisualTestContext};

struct HeroHost {
    shell: Entity<Shell>,
    _data_dir: tempfile::TempDir,
}

impl Render for HeroHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.shell.update(cx, |shell, cx| {
            div()
                .size_full()
                .child(shell.render_main(window, 1000., 1000., cx))
        })
    }
}

fn setup(cx: &mut TestAppContext) -> (Entity<Shell>, &mut VisualTestContext) {
    let dir = tempfile::tempdir().unwrap();
    cx.update(|cx| {
        gpui_base::init(cx);
        cx.set_global(Theme::default());
        crate::app_menus::init(cx);
        settings::init(UiSettings::default(), dir.path(), cx);
        crate::history::init(
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            cx,
        );
    });
    let (host, cx) = cx.add_window_view(|_, cx| {
        let shell = cx.new(|cx| {
            let state = cx.new(|_| AppState::new());
            let mut shell = Shell::new(
                state,
                EngineBootConfig {
                    data_dir: dir.path().into(),
                    ipc_port: 0,
                    edge_url: "http://127.0.0.1:1".into(),
                    edge_token: None,
                    org_id: None,
                    workos_client_id: None,
                    default_harness: zeron_proto::HarnessId::Mock,
                },
                cx,
            );
            shell._state_observation = cx.observe(&shell.state, |_, _, _| {});
            shell.state.update(cx, |state, _| {
                state.workspace_scope = Some(WorkspaceScope::Local);
                state.connection = ConnectionStatus::Ready;
                state.local_device_id = Some("local".into());
                state.no_project = true;
            });
            shell.viewport_width = 1000.0;
            shell.viewport_height = 1000.0;
            shell.splash = SplashPhase::Gone;
            shell.reduced_motion = true;
            shell
        });
        HeroHost {
            shell,
            _data_dir: dir,
        }
    });
    let shell = host.read_with(cx, |host, _| host.shell.clone());
    for _ in 0..3 {
        cx.update(|window, cx| window.draw(cx).clear());
    }
    (shell, cx)
}

#[gpui::test]
fn home_grid_sits_below_a_raised_composer(cx: &mut TestAppContext) {
    let (_shell, cx) = setup(cx);
    let pill = cx.debug_bounds("composer-surface").expect("composer");
    let customize = cx.debug_bounds("home-customize").expect("home grid");
    assert!(customize.top() > pill.bottom(), "{customize:?} vs {pill:?}");
    let centered = (1000.0 - f32::from(pill.size.height)) * 0.5;
    assert!(
        f32::from(pill.top()) < centered,
        "composer stays raised while widgets show: {pill:?}"
    );
}

#[gpui::test]
fn turning_every_widget_off_restores_the_centered_hero(cx: &mut TestAppContext) {
    let (_shell, cx) = setup(cx);
    cx.update(|_, cx| {
        settings::update_home_widgets(cx, |widgets| {
            *widgets = settings::HomeWidgets {
                working_now: false,
                pull_requests: false,
                git_activity: false,
                this_mac: false,
                running_services: false,
                recent_areas: false,
                ci_failures: false,
                services_all_ports: false,
                collapsed: false,
            }
        })
    });
    for _ in 0..3 {
        cx.update(|window, cx| window.draw(cx).clear());
    }
    assert!(cx.debug_bounds("home-customize").is_none());
    let pill = cx.debug_bounds("composer-surface").expect("composer");
    let composer_top = f32::from(pill.top());
    assert!(composer_top > 300.0, "centered again: {pill:?}");
}
