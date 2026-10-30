use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PaneTool {
    Changes,
    Files,
    Browser,
    Device,
}

impl PaneTool {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Changes => "Changes",
            Self::Files => "Files",
            Self::Browser => "Browser",
            Self::Device => "Device",
        }
    }

    pub(super) fn icon(self) -> &'static str {
        match self {
            Self::Changes => icons::ASTRA_PLUS_MINUS,
            Self::Files => icons::FILE_TREE,
            Self::Browser => icons::GLOBE,
            Self::Device => icons::SMARTPHONE,
        }
    }
}

impl Shell {
    pub(super) fn pane_tools(&self, cx: &App) -> Vec<PaneTool> {
        let chat = !self.active_chat.is_empty();
        let mut tools = Vec::new();
        if chat && self.space_git_detected(cx) {
            tools.push(PaneTool::Changes);
        }
        if chat {
            tools.push(PaneTool::Files);
        }
        tools.push(PaneTool::Browser);
        let has_device = self
            .right_surface_rows(cx)
            .iter()
            .any(|(surface, ..)| matches!(surface, RightSurface::Device(_)));
        if self.device_surface_unavailable(cx).is_none() && !has_device {
            tools.push(PaneTool::Device);
        }
        tools
    }

    fn changes_diff(&self, id: u64, cx: &App) -> bool {
        self.diffs
            .get(&id)
            .is_some_and(|changes| !changes.read(cx).is_commit())
    }

    pub(super) fn history_diff(&self, id: u64, cx: &App) -> bool {
        self.diffs
            .get(&id)
            .is_some_and(|changes| changes.read(cx).is_history())
    }

    pub(super) fn tool_surface(&self, tool: PaneTool, cx: &App) -> Option<RightSurface> {
        self.right_surface_rows(cx)
            .into_iter()
            .map(|(surface, ..)| surface)
            .find(|surface| match (tool, surface) {
                (PaneTool::Changes, RightSurface::Diff(id)) => {
                    self.changes_diff(*id, cx) && !self.history_diff(*id, cx)
                }
                (PaneTool::Files, RightSurface::Files)
                | (PaneTool::Browser, RightSurface::Browser(_)) => true,
                _ => false,
            })
    }

    pub(super) fn pane_tool_of(&self, surface: RightSurface, cx: &App) -> Option<PaneTool> {
        match surface {
            RightSurface::Diff(id) if self.changes_diff(id, cx) => Some(PaneTool::Changes),
            RightSurface::Files => Some(PaneTool::Files),
            RightSurface::Browser(_)
                if self.tool_surface(PaneTool::Browser, cx) == Some(surface) =>
            {
                Some(PaneTool::Browser)
            }
            _ => None,
        }
    }

    pub(super) fn open_pane_tool(
        &mut self,
        tool: PaneTool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_surfaces_open(true, cx);
        if tool == PaneTool::Changes {
            let owned: Vec<RightSurface> = self
                .right_surface_rows(cx)
                .into_iter()
                .map(|(surface, ..)| surface)
                .filter(|surface| matches!(surface, RightSurface::Diff(id) if self.changes_diff(*id, cx)))
                .collect();
            if owned.is_empty() {
                self.focus_changes(window, cx);
            } else {
                let surface = self
                    .panels
                    .right_surface_fallback(&self.panel_key(cx), owned.into_iter());
                self.activate_right_surface(surface, window, cx);
            }
            return;
        }
        match (tool, self.tool_surface(tool, cx)) {
            (PaneTool::Files, _) => self.add_files_surface(window, cx),
            (_, Some(surface)) => self.activate_right_surface(surface, window, cx),
            (PaneTool::Browser, None) => self.add_browser_surface(None, window, cx),
            (PaneTool::Device, None) => {
                self.add_device_surface(window, cx);
            }
            (PaneTool::Changes, None) => {}
        }
    }

    pub(super) fn focus_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let history = self
            .right_surface_rows(cx)
            .into_iter()
            .map(|(surface, ..)| surface)
            .find(
                |surface| matches!(surface, RightSurface::Diff(id) if self.history_diff(*id, cx)),
            );
        match history {
            Some(surface) => self.activate_right_surface(surface, window, cx),
            None => self.add_history_surface(window, cx),
        }
    }

    pub(super) fn land_on_default_tool(&mut self, cx: &mut Context<Self>) {
        if !self.right_pane_open(cx) || self.resolved_right_active(cx) != RightSurface::Picker {
            return;
        }
        self.defer_in_active_window(cx, |shell, window, cx| {
            if shell.right_pane_open(cx) && shell.resolved_right_active(cx) == RightSurface::Picker
            {
                let tool = shell.pane_tools(cx)[0];
                shell.open_pane_tool(tool, window, cx);
            }
        });
    }
}

pub(super) struct ChangesReview {
    pub(super) head: Option<AnyElement>,
    pub(super) findings: Option<AnyElement>,
}

impl Shell {
    pub(super) fn changes_review_shown(&self, cx: &App) -> bool {
        matches!(self.route, Route::Chat)
            && self.right_pane_open(cx)
            && matches!(
                self.resolved_right_active(cx),
                RightSurface::Diff(id) if self.changes_diff(id, cx) && !self.history_diff(id, cx)
            )
    }

    pub(super) fn render_changes_review(
        &mut self,
        id: u64,
        theme: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> ChangesReview {
        if !self.changes_diff(id, cx) {
            return ChangesReview {
                head: None,
                findings: None,
            };
        }
        let history = self.history_diff(id, cx);
        let diff_label = self
            .tool_surface(PaneTool::Changes, cx)
            .and_then(|surface| match surface {
                RightSurface::Diff(diff) => self.diffs.get(&diff),
                _ => None,
            })
            .map(|changes| changes.read(cx).scope())
            .filter(|scope| *scope != crate::changes::DiffScope::WorkingTree)
            .map_or("Uncommitted", crate::changes::DiffScope::label);
        let shell = cx.weak_entity();
        let segment = crate::settings::widgets::segmented(
            theme,
            "changes-segment",
            &[(false, diff_label), (true, "History")],
            history,
            move |history, window, cx| {
                shell
                    .update(cx, |shell, cx| {
                        if history {
                            shell.focus_history(window, cx);
                        } else {
                            shell.focus_changes(window, cx);
                        }
                    })
                    .ok();
            },
        );
        let (chip, findings) = if history {
            (None, None)
        } else {
            self.composer.update(cx, |composer, cx| {
                (
                    composer.render_review_chip(theme, window, cx),
                    composer.render_review_findings(theme, cx),
                )
            })
        };
        let head = div()
            .flex_none()
            .px(px(8.0))
            .pt(px(8.0))
            .pb(px(4.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .child(segment)
            .child(div().flex_1())
            .children(chip)
            .into_any_element();
        let findings = findings.map(|findings| {
            div()
                .id("changes-review-findings")
                .flex_none()
                .max_h(px(240.0))
                .overflow_y_scroll()
                .border_b_1()
                .border_color(theme.border)
                .child(findings)
                .into_any_element()
        });
        ChangesReview {
            head: Some(head),
            findings,
        }
    }
}
