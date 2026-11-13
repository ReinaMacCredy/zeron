local ds = require("@ds")
local Icon = require("@ds/icon")
local Menu = require("@ds/menu")
local locale = require("@ds/locale")
local links = require("@ds/links")
local motion = require("@ds/motion")
local sessions = require("@account/sessions")
local route = require("@app/screens/shell/route")
local prefs = require("@app/screens/settings/prefs")
local host = require("@app/screens/settings/host")

local M = {}

local COLUMN_GAP, CARD_RADIUS, ROW_H = 10, 10, 28
local GAP = ui.raw(COLUMN_GAP, "gpui COLUMN_GAP")
local STATS_MS, SERVICES_MS, SLOW_MS = 3000, 5000, 120000
local FAILING_LOOKUPS = 3
local REVEAL = { duration = 300, ease = "cubicBezier(.32,.72,0,1)" }

local WIDGETS = { "pull_requests", "git_activity", "this_mac", "running_services", "ci_failures" }
local ORDER = { "pull_requests", "git_activity", "running_services", "this_mac", "ci_failures" }

export type Source = { value: any?, err: string? }

local function source(name: string): State<Source>
	return ui.state(`home_widgets_{name}`, {} :: Source)
end
M.stats = source("stats")
M.services = source("services")
M.git = source("git")
M.prs = source("prs")
-- When each was last fetched, and what it was fetched for: kept across a reload of the scripts, which asks the engine nothing again.
local kept = ui.state("home_widgets_fetched", { at = {}, projects = "" } :: any)
local function fetched(name: string): number?
	return kept:peek().at[name]
end
local function mark(name: string, at: number?)
	kept:set_key({ "at", name }, at)
end
local busy: { [string]: boolean } = {}
local git_filter = ui.state("home_widgets_git_filter", nil :: string?)
local pr_pick = ui.state("home_widgets_pr_pick", nil :: string?)
local failing_checks = ui.state("home_widgets_failing_checks", {} :: { [string]: string })

function M.any_on(): boolean
	for _, k in WIDGETS do
		if prefs.home_widget(k) then
			return true
		end
	end
	return false
end

function M.raised(): boolean
	return M.any_on() and not prefs.home_widget("collapsed")
end

local function why(err: any): string
	local w = if type(err) == "table" and err.message then tostring(err.message) else tostring(err)
	return locale.s("chat.home.widgets.failed", { why = w })
end

local function settle(slot: State<Source>, name: string, value: any?, err: any?)
	busy[name] = false
	mark(name, ui.now())
	slot:set(if err ~= nil then { err = why(err) } else { value = value })
end

local function engine(slot: State<Source>, name: string, method: string, params: any)
	busy[name] = true
	local ok, p = pcall(host.call, "settings.engine", { method = method, params = params })
	if not ok or p == nil then
		settle(slot, name, nil, "failed")
		return
	end
	p:after(function(v: any)
		settle(slot, name, v)
	end, function(err: any)
		settle(slot, name, nil, err)
	end)
end

local function github(asked: any): any
	local ok, p = pcall(host.call, "plugin.github.github_call", asked)
	return if ok then p else nil
end

local function local_git_projects(): { any }
	local now = sessions.now:get()
	local out = {}
	for _, p in now.projects do
		if p.git and p.device == now.here then
			table.insert(out, p)
		end
	end
	return out
end

local function failing(prs: any): { any }
	local out = {}
	for _, item in prs and prs.mine or {} do
		if item.ci and item.ci.state == "failed" then
			table.insert(out, item)
		end
	end
	return out
end

local FAILED = { FAILURE = true, ERROR = true, TIMED_OUT = true, ACTION_REQUIRED = true, CANCELLED = true, STARTUP_FAILURE = true }

local function lookup_failing(prs: any)
	local known = failing_checks:peek()
	local n = 0
	for _, item in failing(prs) do
		if n >= FAILING_LOOKUPS then
			break
		end
		if known[item.url] == nil then
			n += 1
			local p = github({ op = "pr", url = item.url })
			if p then
				p:after(function(detail: any)
					for _, check in detail and detail.statusCheckRollup or {} do
						local status = ""
						for _, v in { check.conclusion, check.state, check.status } do
							if v and v ~= "" then
								status = string.upper(v)
								break
							end
						end
						if FAILED[status] then
							local next = table.clone(failing_checks:peek())
							next[item.url] = if check.name ~= "" then check.name else check.context
							failing_checks:set(next)
							return
						end
					end
				end, function() end)
			end
		end
	end
end

local function fetch_prs(projects: { any })
	busy.prs = true
	local prs = { repos = {}, mine = {}, review = {} }
	local left = #projects
	local function finish()
		for _, list in { prs.mine, prs.review } do
			table.sort(list, function(a: any, b: any): boolean
				return (a.updatedAt or "") > (b.updatedAt or "")
			end)
		end
		settle(M.prs, "prs", prs)
		lookup_failing(prs)
	end
	local function lists()
		local pending = #prs.repos * 2
		if pending == 0 then
			finish()
			return
		end
		local function one()
			pending -= 1
			if pending == 0 then
				finish()
			end
		end
		for _, repo in prs.repos do
			for _, filter in { "authored", "reviewing" } do
				local p = github({ op = "list", repository = repo.repository, filter = filter })
				if not p then
					prs.error = prs.error or why("failed")
					one()
				else
					p:after(function(page: any)
						local into = if filter == "authored" then prs.mine else prs.review
						for _, item in page and page.items or {} do
							table.insert(into, item)
						end
						one()
					end, function(err: any)
						prs.error = prs.error or why(err)
						one()
					end)
				end
			end
		end
	end
	if left == 0 then
		lists()
		return
	end
	local function done_one()
		left -= 1
		if left == 0 then
			lists()
		end
	end
	for _, project in projects do
		local p = github({ op = "repository", project = project.id })
		if not p then
			done_one()
		else
			p:after(function(repository: any)
				if type(repository) == "string" and repository ~= "" then
					local found = nil
					for _, r in prs.repos do
						if string.lower(r.repository) == string.lower(repository) then
							found = r
						end
					end
					if found then
						table.insert(found.projects, project.id)
					else
						table.insert(prs.repos, { repository = repository, name = project.name, projects = { project.id } })
					end
				end
				done_one()
			end, function()
				done_one()
			end)
		end
	end
end


local function due(name: string, every: number): boolean
	local at = fetched(name)
	return not busy[name] and (at == nil or ui.now() - at >= every)
end

local function refresh()
	local projects = local_git_projects()
	local ids = {}
	for _, p in projects do
		table.insert(ids, p.id)
	end
	local sig = table.concat(ids, ",")
	if sig ~= kept:peek().projects then
		kept:set_key({ "projects" }, sig)
		mark("git", nil)
		mark("prs", nil)
	end
	local all_ports = prefs.home_widget("services_all_ports")
	if all_ports ~= kept:peek().ports then
		kept:set_key({ "ports" }, all_ports)
		mark("services", nil)
		M.services:set({})
	end
	if prefs.home_widget("this_mac") and due("stats", STATS_MS) then
		engine(M.stats, "stats", "GetSystemStats", {})
	end
	if prefs.home_widget("running_services") and due("services", SERVICES_MS) then
		engine(M.services, "services", "ListListeningServices", { allPorts = all_ports })
	end
	if prefs.home_widget("git_activity") and due("git", SLOW_MS) then
		engine(M.git, "git", "GetGitActivity", { spaceId = git_filter:peek(), days = 14 })
	end
	if (prefs.home_widget("pull_requests") or prefs.home_widget("ci_failures")) and due("prs", SLOW_MS) then
		fetch_prs(projects)
	end
end
M.refresh = refresh

function M.summary(): string
	local parts: { string } = {}
	local prs = M.prs:get().value
	if prs then
		if #prs.review > 0 then
			table.insert(parts, locale.s("chat.home.widgets.to_review", { n = #prs.review }))
		end
		local f = #failing(prs)
		if f > 0 then
			table.insert(parts, locale.s("chat.home.widgets.failing", { n = f }))
		end
	end
	local git = M.git:get().value
	if git then
		table.insert(parts, locale.s("chat.home.widgets.commits", { n = git.commits or 0 }))
	end
	return if #parts == 0 then locale.s("chat.home.widgets.home") else table.concat(parts, " · ")
end

local function mono(text: string | () -> string, color: string): Node
	return ui.text { text, literal = true, shrink = 0, wrap = false, font = "$font.mono", text_size = "$font.size.xs", color = color }
end

local function dot(color: string): Node
	return ui.div { w = 6, h = 6, shrink = 0, rounded = "$radius.full", bg = color }
end

local function empty_line(text: string | () -> string): Node
	return ui.text { text, literal = true, py = ui.raw(4, "gpui empty_line"), text_size = "$font.size.sm", color = "$text.tertiary" }
end

local function sub_heading(text: string, color: string): Node
	return ui.text { text, literal = true, pt = ui.raw(4, "gpui sub_heading"), pb = ui.raw(2, "gpui sub_heading"), text_size = "$font.size.xs", color = color }
end

local function chip(text: string | () -> string): Node
	return ui.row {
		px = ui.raw(8, "gpui hover chip"),
		py = ui.raw(3, "gpui hover chip"),
		rounded = "$radius.md",
		border = 1,
		border_color = "$border",
		bg = "$zeron.menu.solid",
		text_size = "$font.size.xs",
		color = "$text.primary",
		ui.text { text, literal = true, wrap = false },
	}
end

local function row_box(props: { [any]: any }): Node
	props.h = ROW_H
	props.items = "center"
	props.gap = ui.raw(8, "gpui widget row")
	props.px = ui.raw(6, "gpui widget row")
	props.mx = ui.raw(-6, "gpui widget row: its wash past the card's text")
	props.rounded = "$radius.md"
	props.cursor = "pointer"
	props.hover = { bg = "$hover" }
	return ui.row(props)
end

local function card(id: string, icon: string, title: string, count: (() -> string?)?, accessory: Node?, body: Node): Node
	return ui.column {
		id = `home-card-{id}`,
		role = "group",
		label = title,
		w_full = true,
		rounded = ui.raw(CARD_RADIUS, "gpui CARD_RADIUS"),
		border = 1,
		border_color = "$border",
		bg = "$zeron.home.card",
		px = ui.raw(12, "gpui card"),
		pt = ui.raw(10, "gpui card"),
		pb = ui.raw(10, "gpui card"),
		ui.row {
			gap = ui.raw(6, "gpui card head"),
			pb = ui.raw(6, "gpui card head"),
			items = "center",
			text_size = "$font.size.sm",
			Icon { name = icon :: any, size = 13, color = "$text.secondary" },
			ui.text { title, literal = true, wrap = false, color = "$text.secondary" },
			if count
				then ui.text { function(): string
					return count() or ""
				end, literal = true, wrap = false, color = "$text.tertiary" }
				else nil,
			if accessory then ui.div { grow = 1 } else nil,
			accessory,
		},
		body,
	}
end

local function filter_trigger(id: string, label: () -> string, open: State<boolean>): Node
	return ui.button {
		id = id,
		label = label,
		expanded = open,
		h = 20,
		max_w = 180,
		px = ui.raw(6, "gpui filter_trigger"),
		gap = ui.raw(4, "gpui filter_trigger"),
		items = "center",
		rounded = "$radius.md",
		border = 1,
		border_color = "$border",
		text_size = "$font.size.xs",
		color = "$text.secondary",
		bg = function(): string
			return if open:get() then "$hover" else "#00000000"
		end,
		hover = { bg = "$hover" },
		on_click = function()
			open:set(not open:peek())
		end,
		ui.text { label, literal = true, truncate = true, min_w = 0 },
		Icon { name = "alt-arrow-down", size = 10, color = "$text.tertiary" },
	}
end

local function radio_menu(id: string, open: State<boolean>, anchor: Node, align: string, rows: () -> { any }, pick: (any) -> ()): Node
	return ui.popover(Menu.panel {
		open = open,
		anchor = anchor,
		w = 220,
		placement = { side = "bottom", align = align, offset = 4 },
		build = function(): Node
			return Menu.Group {
				id = id,
				ui.each(ui.computed(rows), function(r: any, i: number): Node
					return Menu.Row {
						id = `{id}-{i - 1}`,
						role = "menuitemradio",
						bare = true,
						label = r.name,
						checked = function(): boolean
							return r.selected
						end,
						trail = if r.count then { mono(tostring(r.count), "$text.tertiary") } else nil,
						on_click = function()
							open:set(false)
							pick(r.value)
						end,
					}
				end),
			}
		end,
	})
end

local function pr_row(item: any, right: string): Node
	local repo = string.match(item.repository or "", "([^/]+)$") or item.repository or ""
	local state = item.ci and item.ci.state
	local check: Node? = nil
	if state == "passed" then
		check = Icon { name = "lucide-check", size = 12, color = "$success" }
	elseif state == "failed" then
		check = Icon { name = "lucide-circle-x", size = 12, color = "$danger" }
	elseif state == "pending" then
		check = Icon { name = "lucide-circle-dashed", size = 12, color = "$text.tertiary" }
	end
	return row_box {
		id = `home-pr-{item.url}`,
		role = "button",
		label = item.title,
		on_click = function()
			links.open(item.url)
		end,
		Icon { name = "lucide-git-pull-request", size = 12, color = if item.isDraft then "$text.tertiary" else "$success" },
		ui.text { item.title, literal = true, truncate = true, grow = 1, min_w = 0, color = "$text.primary" },
		mono(`{repo}#{item.number}`, "$text.tertiary"),
		ui.div { w = 14, shrink = 0, check },
		ui.row { w = 68, shrink = 0, justify = "end", ui.text { right, literal = true, truncate = true, text_size = "$font.size.sm", color = "$text.secondary" } },
	}
end

local function review_label(item: any): string
	if item.isDraft then
		return locale.s("chat.home.widgets.draft")
	end
	if item.reviewDecision == "approved" then
		return locale.s("chat.home.widgets.approved")
	elseif item.reviewDecision == "changesRequested" then
		return locale.s("chat.home.widgets.changes")
	end
	return locale.s("chat.home.widgets.in_review")
end

local function pr_scope(prs: any): { kind: string, repository: string?, name: string? }
	local pick = pr_pick:get()
	if pick == "" then
		return { kind = "all" }
	elseif pick then
		return { kind = "repo", repository = pick }
	end
	local id = route.new_chat_project:get()
	local now = sessions.now:get()
	local project = nil
	for _, p in now.projects do
		if p.id == id then
			project = p
		end
	end
	if not project then
		return { kind = "all" }
	end
	local function same(other: string): boolean
		if other == project.id then
			return true
		end
		if not project.repository then
			return false
		end
		for _, p in now.projects do
			if p.id == other then
				return p.repository == project.repository
			end
		end
		return false
	end
	for _, repo in prs and prs.repos or {} do
		for _, other in repo.projects do
			if same(other) then
				return { kind = "repo", repository = repo.repository }
			end
		end
	end
	return { kind = "unlinked", name = project.name }
end

local function includes(scope: any, item: any): boolean
	if scope.kind == "all" then
		return true
	elseif scope.kind == "repo" then
		return string.lower(item.repository or "") == string.lower(scope.repository)
	end
	return false
end

local function PullRequests(): Node
	local open = ui.use_state(false)
	local scope = ui.computed(function(): any
		return pr_scope(M.prs:get().value)
	end)
	local label = function(): string
		local s = scope:get()
		if s.kind == "all" then
			return locale.s("chat.home.widgets.all_projects")
		elseif s.kind == "unlinked" then
			return s.name
		end
		for _, r in (M.prs:get().value or {}).repos or {} do
			if r.repository == s.repository then
				return r.name
			end
		end
		return s.repository
	end
	local lists = ui.computed(function(): any
		local prs = M.prs:get().value
		local s = scope:get()
		local mine, review = {}, {}
		for _, item in prs and prs.mine or {} do
			if includes(s, item) then
				table.insert(mine, item)
			end
		end
		for _, item in prs and prs.review or {} do
			if includes(s, item) then
				table.insert(review, item)
			end
		end
		return { mine = mine, review = review }
	end)
	local trigger = filter_trigger("home-pr-menu", label, open)
	local menu = radio_menu("home-pr-project", open, trigger, "end", function(): { any }
		local s = scope:get()
		local rows = { { name = locale.s("chat.home.widgets.all_projects"), value = "", selected = s.kind == "all" } }
		for _, r in (M.prs:get().value or {}).repos or {} do
			table.insert(rows, { name = r.name, value = r.repository, selected = s.kind == "repo" and s.repository == r.repository })
		end
		return rows
	end, function(v: any)
		pr_pick:set(v)
	end)
	local body = ui.switch(ui.computed(function(): string
		local src = M.prs:get()
		if src.err then
			return "error"
		elseif not src.value then
			return "loading"
		end
		local l = lists:get()
		return if #l.mine + #l.review == 0 then "empty" else "list"
	end), {
		loading = empty_line(locale.t("chat.home.widgets.loading") :: any),
		error = empty_line(function(): string
			return M.prs:get().err or ""
		end),
		empty = empty_line(function(): string
			local s = scope:get()
			if s.kind == "unlinked" then
				return locale.s("chat.home.widgets.no_repository", { name = s.name })
			end
			local prs = M.prs:get().value
			return (prs and prs.error) or locale.s("chat.home.widgets.nothing_waiting")
		end),
		list = ui.column {
			ui.show(function(): boolean
				return #lists:get().review > 0
			end, ui.column {
				ui.text { function(): string
					return locale.s("chat.home.widgets.waiting_review", { n = #lists:get().review })
				end, literal = true, pt = ui.raw(4, "gpui sub_heading"), pb = ui.raw(2, "gpui sub_heading"), text_size = "$font.size.xs", color = "$warning" },
				ui.each(ui.computed(function(): { any }
					return table.move(lists:get().review, 1, 4, 1, {})
				end), function(item: any): Node
					return pr_row(item, item.author and item.author.login or "")
				end, { key = function(item: any): string
					return item.url
				end }),
			}),
			ui.show(function(): boolean
				return #lists:get().mine > 0
			end, ui.column {
				ui.text { function(): string
					return locale.s("chat.home.widgets.yours", { n = #lists:get().mine })
				end, literal = true, pt = ui.raw(4, "gpui sub_heading"), pb = ui.raw(2, "gpui sub_heading"), text_size = "$font.size.xs", color = "$text.tertiary" },
				ui.each(ui.computed(function(): { any }
					return table.move(lists:get().mine, 1, 5, 1, {})
				end), function(item: any): Node
					return pr_row(item, review_label(item))
				end, { key = function(item: any): string
					return item.url
				end }),
			}),
		},
	})
	return card("pull-requests", "lucide-git-pull-request", locale.s("chat.home.widgets.pull_requests"), function(): string?
		local n = #lists:get().review
		return if n > 0 then tostring(n) else nil
	end, menu, body)
end

local function line_count(n: number): string
	return if n >= 1000 then string.format("%.1fk", n / 1000) else tostring(n)
end

local function day_time(date: string): number?
	local y, m, d = string.match(date or "", "^(%d+)-(%d+)-(%d+)")
	if not y then
		return nil
	end
	return os.time({ year = tonumber(y) :: number, month = tonumber(m) :: number, day = tonumber(d) :: number, hour = 12 })
end

local function GitChart(): Node
	local hovered = ui.use_state(nil :: number?)
	local days = ui.computed(function(): { any }
		local g = M.git:get().value
		return g and g.days or {}
	end)
	local most = ui.computed(function(): number
		local m = 1
		for _, d in days:get() do
			m = math.max(m, d.commits or 0)
		end
		return m
	end)
	return ui.column {
		ui.row {
			items = "end",
			justify = "between",
			ui.row {
				items = "end",
				gap = ui.raw(6, "gpui git_chart"),
				ui.text { function(): string
					local g = M.git:get().value
					return tostring(g and g.commits or 0)
				end, literal = true, text_size = ui.raw(26, "gpui git_chart"), line_height = ui.raw(30, "gpui git_chart"), color = "$text.primary", tabular = true },
				ui.text { function(): string
					return locale.s("chat.home.widgets.commits_days", { n = #days:get() })
				end, literal = true, pb = ui.raw(3, "gpui git_chart"), text_size = "$font.size.sm", color = "$text.secondary" },
			},
			ui.row {
				pb = ui.raw(3, "gpui git_chart"),
				gap = ui.raw(6, "gpui git_chart"),
				mono(function(): string
					local g = M.git:get().value
					return "+" .. line_count(g and g.additions or 0)
				end, "$success"),
				mono(function(): string
					local g = M.git:get().value
					return "-" .. line_count(g and g.deletions or 0)
				end, "$danger"),
			},
		},
		ui.row {
			id = "home-git-bars",
			mt = ui.raw(10, "gpui git_chart"),
			h = 56,
			relative = true,
			items = "end",
			gap = ui.raw(5, "gpui git_chart"),
			ui.each(days, function(d: any, i: number): Node
				return ui.row {
					id = `home-git-day-{i - 1}`,
					grow = 1,
					basis = 0,
					h_full = true,
					items = "end",
					on_hover = function(on: boolean)
						if on then
							hovered:set(i)
						elseif hovered:peek() == i then
							hovered:set(nil)
						end
					end,
					ui.div {
						w_full = true,
						h = function(): number
							local n = d.commits or 0
							return if n == 0 then 2 else math.max(4, n / most:get() * 56)
						end,
						rounded_tl = 2,
						rounded_tr = 2,
						bg = function(): string
							local n = d.commits or 0
							if n == 0 then
								return "$hover"
							end
							local h = hovered:get()
							if h == i or (h == nil and i == #days:get()) then
								return "$accent"
							end
							return "$zeron.home.bar"
						end,
					},
				}
			end),
			ui.show(function(): boolean
				return hovered:get() ~= nil
			end, ui.div {
				absolute = true,
				top = -26,
				left = function(): any
					local i, n = hovered:get() or 1, math.max(1, #days:get())
					return if (i - 1) * 2 < n then `{(i - 1) / n * 100}%` else nil
				end,
				right = function(): any
					local i, n = hovered:get() or 1, math.max(1, #days:get())
					return if (i - 1) * 2 < n then nil else `{(n - i) / n * 100}%`
				end,
				pointer_events = "none",
				chip(function(): string
					local d = days:get()[hovered:get() or 1]
					if not d then
						return ""
					end
					local at = day_time(d.date) or os.time()
					return locale.s("chat.home.widgets.day", { day = `{locale.weekday(at)}, {locale.month_day(at)}`, n = d.commits or 0 })
				end),
			}),
		},
		ui.row {
			mt = ui.raw(3, "gpui git_chart"),
			gap = ui.raw(5, "gpui git_chart"),
			ui.each(days, function(d: any): Node
				local at = day_time(d.date)
				local initial = if at then utf8.char(utf8.codepoint(locale.weekday(at), 1)) else ""
				return ui.row { grow = 1, basis = 0, justify = "center", ui.text { initial, literal = true, text_size = ui.raw(9, "gpui weekday initial"), color = "$text.tertiary" } }
			end),
		},
	}
end

local function GitActivity(): Node
	local open = ui.use_state(false)
	local label = function(): string
		local id = git_filter:get()
		if id == nil then
			return locale.s("chat.home.widgets.all_projects")
		end
		for _, p in (M.git:get().value or {}).projects or {} do
			if p.spaceId == id then
				return p.name
			end
		end
		return locale.s("chat.home.widgets.project")
	end
	local trigger = filter_trigger("home-git-menu", label, open)
	local menu = radio_menu("home-git-project", open, trigger, "start", function(): { any }
		local id = git_filter:get()
		local rows = { { name = locale.s("chat.home.widgets.all_projects"), value = false, selected = id == nil } }
		for _, p in (M.git:get().value or {}).projects or {} do
			table.insert(rows, { name = p.name, value = p.spaceId, count = p.commits, selected = id == p.spaceId })
		end
		return rows
	end, function(v: any)
		local id = if v == false then nil else v
		if git_filter:peek() ~= id then
			git_filter:set(id)
			busy.git = false
			mark("git", nil)
			refresh()
		end
	end)
	local body = ui.switch(ui.computed(function(): string
		local src = M.git:get()
		return if src.err then "error" elseif src.value then "chart" else "loading"
	end), {
		loading = empty_line(locale.t("chat.home.widgets.loading") :: any),
		error = empty_line(function(): string
			return M.git:get().err or ""
		end),
		chart = GitChart(),
	})
	return card("git-activity", "lucide-git-commit-horizontal", locale.s("chat.home.widgets.git_activity"), nil, menu, body)
end

local function gb_total(bytes: number): string
	local gb = bytes / 1e9
	return if gb >= 1000 then string.format("%.1f TB", gb / 1000) else string.format("%.0f GB", gb)
end

local function gib(bytes: number): number
	return bytes / (1024 * 1024 * 1024)
end

local function used_of(used: number, total: number): string
	local u = if used < 10 then string.format("%.1f", used) else string.format("%.0f", used)
	return string.format("%s / %.0f GB", u, total)
end

local function Meter(props: { id: string, label: string, share: () -> number, detail: () -> string }): Node
	local hovered = ui.use_state(false)
	return ui.column {
		id = `home-meter-{props.id}`,
		role = "group",
		label = props.label,
		grow = 1,
		basis = 0,
		relative = true,
		gap = ui.raw(3, "gpui meter"),
		on_hover = function(on: boolean)
			hovered:set(on)
		end,
		ui.row {
			justify = "between",
			text_size = ui.raw(10, "gpui meter"),
			color = function(): string
				return if hovered:get() then "$text.secondary" else "$text.tertiary"
			end,
			ui.text { props.label, literal = true },
			ui.text { function(): string
				return string.format("%.0f%%", math.clamp(props.share(), 0, 1) * 100)
			end, literal = true, tabular = true },
		},
		ui.div {
			h = 3,
			rounded = "$radius.full",
			bg = "$hover",
			ui.div {
				h_full = true,
				w = function(): string
					return `{math.clamp(props.share(), 0, 1) * 100}%`
				end,
				rounded = "$radius.full",
				bg = function(): string
					return if hovered:get() then "$text.primary" else "$text.secondary"
				end,
			},
		},
		ui.show(function(): boolean
			return hovered:get()
		end, ui.div { absolute = true, left = 0, top = -28, pointer_events = "none", chip(props.detail) }),
	}
end

local function ThisMac(): Node
	local function stats(): any
		return M.stats:get().value or {}
	end
	local function share(used: number?, total: number?): number
		return if not total or total == 0 then 0 else (used or 0) / total
	end
	local name = function(): string
		local now = sessions.now:get()
		for _, d in now.devices do
			if d.id == now.here then
				return d.name
			end
		end
		return locale.s("chat.home.widgets.this_mac")
	end
	local body = ui.switch(ui.computed(function(): string
		local src = M.stats:get()
		if src.err then
			return "error"
		elseif not src.value then
			return "loading"
		end
		return if src.value.available then "stats" else "unavailable"
	end), {
		loading = empty_line(locale.t("chat.home.widgets.loading") :: any),
		error = empty_line(function(): string
			return M.stats:get().err or ""
		end),
		unavailable = empty_line(locale.t("chat.home.widgets.unavailable") :: any),
		stats = ui.column {
			ui.row {
				items = "center",
				gap = ui.raw(6, "gpui this_mac"),
				dot("$success"),
				ui.text { name, literal = true, shrink = 0, wrap = false, color = "$text.primary" },
				ui.text { function(): string
					local s = stats()
					return locale.s("chat.home.widgets.mac_line", { cores = s.cores or 0, memory = string.format("%.0f GB", gib(s.memoryTotal or 0)), disk = gb_total(s.diskTotal or 0) })
				end, literal = true, truncate = true, min_w = 0, text_size = "$font.size.xs", color = "$text.tertiary" },
			},
			ui.row {
				gap = ui.raw(10, "gpui this_mac"),
				pt = ui.raw(6, "gpui this_mac"),
				Meter {
					id = "CPU",
					label = locale.s("chat.home.widgets.cpu"),
					share = function(): number
						return (stats().cpuPercent or 0) / 100
					end,
					detail = function(): string
						local s = stats()
						return locale.s("chat.home.widgets.cpu_detail", { pct = string.format("%.0f", s.cpuPercent or 0), n = s.cores or 0 })
					end,
				},
				Meter {
					id = "Mem",
					label = locale.s("chat.home.widgets.mem"),
					share = function(): number
						local s = stats()
						return share(s.memoryUsed, s.memoryTotal)
					end,
					detail = function(): string
						local s = stats()
						return used_of(gib(s.memoryUsed or 0), gib(s.memoryTotal or 0))
					end,
				},
				Meter {
					id = "Disk",
					label = locale.s("chat.home.widgets.disk"),
					share = function(): number
						local s = stats()
						return share(s.diskUsed, s.diskTotal)
					end,
					detail = function(): string
						local s = stats()
						return used_of((s.diskUsed or 0) / 1e9, (s.diskTotal or 0) / 1e9)
					end,
				},
			},
		},
	})
	return card("this-mac", "laptop", locale.s("chat.home.widgets.this_mac"), nil, nil, body)
end

local function service_row(s: any): Node
	local owned = s.project ~= nil
	local label = if s.project and s.worktree then `{s.project} / {s.worktree}` elseif s.project then s.project else s.command
	local url = `http://localhost:{s.port}`
	return row_box {
		id = `home-service-{s.port}`,
		role = "link",
		label = label,
		on_click = function()
			links.outside(url)
		end,
		dot(if owned then "$success" else "$text.tertiary"),
		ui.div { w = 48, shrink = 0, mono(`:{s.port}`, "$text.secondary") },
		ui.text { label, literal = true, truncate = true, grow = 1, min_w = 0, color = if owned then "$text.primary" else "$text.secondary" },
		ui.text { s.framework or s.command, literal = true, shrink = 0, wrap = false, text_size = "$font.size.sm", color = "$text.tertiary" },
	}
end

local function RunningServices(): Node
	local list = ui.computed(function(): { any }
		local v = M.services:get().value
		return v and v.services or {}
	end)
	local body = ui.switch(ui.computed(function(): string
		local src = M.services:get()
		return if src.err then "error" elseif not src.value then "loading" elseif #list:get() == 0 then "empty" else "list"
	end), {
		loading = empty_line(locale.t("chat.home.widgets.loading") :: any),
		error = empty_line(function(): string
			return M.services:get().err or ""
		end),
		empty = empty_line(locale.t("chat.home.widgets.nothing_listening") :: any),
		list = ui.column {
			ui.each(ui.computed(function(): { any }
				return table.move(list:get(), 1, 5, 1, {})
			end), service_row, { key = function(s: any): string
				return tostring(s.port)
			end }),
			ui.show(function(): boolean
				return #list:get() > 5
			end, empty_line(function(): string
				return locale.s("chat.home.widgets.more_listening", { n = #list:get() - 5 })
			end)),
		},
	})
	return card("running-services", "global", locale.s("chat.home.widgets.running_services"), function(): string?
		return locale.s(if prefs.home_widget("services_all_ports") then "chat.home.widgets.all_ports" else "chat.home.widgets.by_zeron")
	end, nil, body)
end

local function short_age(iso: string?): string
	local y, mo, d, h, mi, s = string.match(iso or "", "^(%d+)-(%d+)-(%d+)T(%d+):(%d+):(%d+)")
	if not y then
		return ""
	end
	local utc = os.time({ year = tonumber(y) :: number, month = tonumber(mo) :: number, day = tonumber(d) :: number, hour = tonumber(h) :: number, min = tonumber(mi) :: number, sec = tonumber(s) :: number })
	local offset = os.time() - os.time(os.date("!*t") :: any)
	return locale.ago_short(os.time() - (utc + offset))
end

local function CiFailures(): Node
	local list = ui.computed(function(): { any }
		return failing(M.prs:get().value)
	end)
	local body = ui.switch(ui.computed(function(): string
		local src = M.prs:get()
		return if src.err then "error" elseif not src.value then "loading" elseif #list:get() == 0 then "passing" else "list"
	end), {
		loading = empty_line(locale.t("chat.home.widgets.loading") :: any),
		error = empty_line(function(): string
			return M.prs:get().err or ""
		end),
		passing = empty_line(locale.t("chat.home.widgets.all_passing") :: any),
		list = ui.each(ui.computed(function(): { any }
			return table.move(list:get(), 1, 4, 1, {})
		end), function(item: any): Node
			local repo = string.match(item.repository or "", "([^/]+)$") or item.repository or ""
			return row_box {
				id = `home-ci-{item.url}`,
				role = "button",
				label = function(): string
					return failing_checks:get()[item.url] or locale.s("chat.home.widgets.checks_failing")
				end,
				on_click = function()
					links.open(item.url)
				end,
				Icon { name = "lucide-circle-x", size = 13, color = "$danger" },
				ui.row {
					grow = 1,
					min_w = 0,
					ui.text { function(): string
						return failing_checks:get()[item.url] or locale.s("chat.home.widgets.checks_failing")
					end, literal = true, truncate = true, min_w = 0, color = "$text.primary" },
					ui.text { `· {repo}#{item.number}`, literal = true, ml = ui.raw(6, "gpui ci row"), shrink = 0, wrap = false, color = "$text.tertiary" },
				},
				mono(short_age(item.updatedAt), "$text.tertiary"),
			}
		end, { key = function(item: any): string
			return item.url
		end }),
	})
	return card("ci-failures", "lucide-circle-x", locale.s("chat.home.widgets.ci_failures"), nil, nil, body)
end

local BUILD: { [string]: () -> Node } = {
	pull_requests = PullRequests,
	git_activity = GitActivity,
	this_mac = ThisMac,
	running_services = RunningServices,
	ci_failures = CiFailures,
}

local function estimated_rows(w: string): number
	if w == "pull_requests" then
		local prs = M.prs:get().value
		return if prs then math.min(#prs.mine, 5) + math.min(#prs.review, 4) + 2 else 2
	elseif w == "git_activity" then
		return 4
	elseif w == "this_mac" then
		return 2
	elseif w == "running_services" then
		local v = M.services:get().value
		return if v then math.clamp(#(v.services or {}), 1, 6) else 1
	end
	local prs = M.prs:get().value
	return if prs then math.clamp(#failing(prs), 1, 4) else 1
end

function M.columns(widgets: { { id: string, h: number } }): { { string } }
	local cols: { { string } } = { {}, {} }
	local heights = { 0, 0 }
	for _, w in widgets do
		local c = if heights[2] < heights[1] then 2 else 1
		table.insert(cols[c], w.id)
		heights[c] += w.h + COLUMN_GAP
	end
	return cols
end

local placed = ui.computed(function(): string
	local list = {}
	for _, w in ORDER do
		if prefs.home_widget(w) then
			table.insert(list, { id = w, h = estimated_rows(w) * ROW_H + 40 })
		end
	end
	local cols = M.columns(list)
	return table.concat(cols[1], ",") .. "|" .. table.concat(cols[2], ",")
end)

local function grid_column(which: number): Node
	return ui.column {
		hidden = function(): boolean
			return (string.split(placed:get(), "|")[which] or "") == ""
		end,
		grow = 1,
		basis = 0,
		min_w = 0,
		gap = GAP,
		ui.each(ui.computed(function(): { string }
			local part = string.split(placed:get(), "|")[which] or ""
			return if part == "" then {} else string.split(part, ",")
		end), function(id: string): Node
			return BUILD[id]()
		end, { key = function(id: string): string
			return id
		end }),
	}
end

local function choice(id: string, label: string, all_ports: boolean): Node
	local selected = function(): boolean
		return prefs.home_widget("services_all_ports") == all_ports
	end
	return ui.button {
		id = id,
		label = label,
		checked = selected,
		h = 24,
		px = ui.raw(8, "gpui choice_chip"),
		items = "center",
		rounded = "$radius.md",
		border = 1,
		border_color = function(): string
			return if selected() then "$zeron.home.choice_edge" else "$border"
		end,
		bg = function(): string
			return if selected() then "$pressed" else "#00000000"
		end,
		text_size = "$font.size.xs",
		weight = "$font.weight.medium",
		color = function(): string
			return if selected() then "$text.primary" else "$text.secondary"
		end,
		on_click = function()
			prefs.set_home_widget("services_all_ports", all_ports)
		end,
		ui.text { label, literal = true, wrap = false },
	}
end

local function widget_toggle(key: string): Node
	local on = ui.use_state(prefs.home_widget(key))
	ui.effect(function()
		local want = prefs.home_widget(key)
		if on:peek() ~= want then
			on:set(want)
		end
	end)
	local label = locale.s(`chat.home.widgets.{key}`)
	return Menu.Row {
		id = `home-toggle-{key}-quick`,
		bare = true,
		label = label,
		on_click = function()
			prefs.set_home_widget(key, not prefs.home_widget(key))
		end,
		trail = {
			ds.Switch { value = on, label = label, on_change = function(v: boolean)
				prefs.set_home_widget(key, v)
			end },
		},
	}
end

local Customize = ui.component(function(): Node
	local open = ui.use_state(false)
	local trigger = ui.button {
		id = "home-customize",
		label = locale.s("chat.home.widgets.customize"),
		expanded = open,
		h = 24,
		px = ui.raw(6, "gpui customize"),
		gap = ui.raw(6, "gpui customize"),
		items = "center",
		rounded = "$radius.md",
		text_size = "$font.size.sm",
		bg = function(): string
			return if open:get() then "$hover" else "#00000000"
		end,
		color = function(): string
			return if open:get() then "$text.primary" else "$text.secondary"
		end,
		hover = { bg = "$hover", color = "$text.primary" },
		on_click = function()
			open:set(not open:peek())
		end,
		Icon { name = "widget", size = 13 },
		ui.text { locale.t("chat.home.widgets.customize"), wrap = false },
	}
	return ui.div {
		ui.popover(Menu.panel {
			open = open,
			anchor = trigger,
			w = 260,
			placement = { side = "bottom", align = "end", offset = 4 },
			build = function(): Node
				local rows: { any } = { id = "home-customize-menu", heading = locale.t("chat.home.widgets.widgets_heading") }
				for _, key in WIDGETS do
					table.insert(rows, widget_toggle(key))
				end
				return ui.column {
					Menu.Group(rows),
					Menu.Rule {},
					Menu.Group {
						heading = locale.t("chat.home.widgets.running_services"),
						ui.row {
							px = ui.raw(8, "gpui customize modes"),
							pb = ui.raw(6, "gpui customize modes"),
							gap = ui.raw(6, "gpui customize modes"),
							choice("home-quick-by-zeron", locale.s("chat.home.widgets.by_zeron"), false),
							choice("home-quick-all-ports", locale.s("chat.home.widgets.all_ports"), true),
						},
					},
				}
			end,
		}),
	}
end)

M.Widgets = ui.component(function(): Node
	local collapsed = function(): boolean
		return prefs.home_widget("collapsed")
	end
	local reveal = ui.use_state(if collapsed() then 0 else 1)
	ui.effect(function()
		local target = if collapsed() then 0 else 1
		if reveal:peek() ~= target then
			anime.animate(reveal, motion.params({ value = target, duration = REVEAL.duration, ease = REVEAL.ease }))
		end
	end)
	ui.effect(function()
		if not ui.window.active:get() then
			return
		end
		refresh()
		ui.every(1000, function()
			refresh()
			return true
		end)
	end)
	local grid = ui.ref()
	local summary = ui.computed(M.summary)
	local pill = ui.button {
		id = "home-collapse",
		label = function(): string
			return locale.s(if collapsed() then "chat.home.widgets.show" else "chat.home.widgets.hide")
		end,
		expanded = function(): boolean
			return not collapsed()
		end,
		h = 28,
		pl = ui.raw(8, "gpui collapse pill"),
		pr = ui.raw(4, "gpui collapse pill"),
		gap = ui.raw(8, "gpui collapse pill"),
		items = "center",
		rounded = ui.raw(7, "gpui collapse pill"),
		bg = "#00000000",
		hover = { bg = "$hover" },
		on_click = function()
			prefs.set_home_widget("collapsed", not collapsed())
		end,
		Icon { name = "home", size = 13, color = "$text.secondary" },
		ui.text { summary, literal = true, wrap = false, text_size = "$font.size.sm", weight = "$font.weight.medium", color = "$text.primary" },
		ui.div {
			w = 20,
			h = 20,
			shrink = 0,
			items = "center",
			justify = "center",
			rotate = function(): number
				return 180 * reveal:get()
			end,
			Icon { name = "alt-arrow-down", size = 14, color = "$text.secondary" },
		},
	}
	return ui.column {
		id = "home-widgets",
		w_full = true,
		text_size = "$font.size.md",
		ui.row {
			h = 40,
			px = ui.raw(8, "gpui home header"),
			items = "center",
			border_sides = { 1, 0, 0, 0 },
			border_color = "$border",
			pill,
			ui.div { ml = "auto", Customize {} },
		},
		ui.show(function(): boolean
			return reveal:get() > 0
		end, ui.div {
			overflow = "hidden",
			h = function(): any
				local r, natural = reveal:get(), grid.height or 0
				return if r < 1 and natural > 0 then natural * r else nil
			end,
			ui.row {
				ref = grid,
				id = "home-grid",
				opacity = function(): number
					return reveal:get()
				end,
				y = function(): number
					return -8 * (1 - reveal:get())
				end,
				px = ui.raw(10, "gpui grid"),
				pb = ui.raw(10, "gpui grid"),
				items = "start",
				gap = GAP,
				grid_column(1),
				grid_column(2),
			},
		}),
	}
end)

return M
