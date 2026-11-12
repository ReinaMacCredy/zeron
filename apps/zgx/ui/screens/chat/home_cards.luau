local ds = require("@ds")
local Icon = require("@ds/icon")
local locale = require("@ds/locale")
local motion = require("@ds/motion")
local sessions = require("@account/sessions")
local route = require("@app/screens/shell/route")
local keys = require("@app/screens/shell/keys")
local live = require("@app/screens/shell/live_pill")
local waiting = require("@app/screens/chat/waiting")
local tasks_store = require("@app/screens/tasks/store")
local tasks_page = require("@app/screens/tasks")
local task_card = require("@app/screens/tasks/card")

local M = {}

local ROWS, ROW_H = 3, 46
local WORKING, DONE = "$accent", "$zeron.status.done"

local function project_name(id: string?): string?
	if not id then
		return nil
	end
	for _, p in sessions.now:get().projects do
		if p.id == id then
			return p.name
		end
	end
	return nil
end

local function ago(at: number?): string
	local t = os.time() - (at or os.time())
	if t < 60 then
		return locale.s("time.just_now")
	end
	return locale.s("chat.home.cards.ago", { when = locale.ago_short(t) })
end

local function with_project(c: any, words: string): string
	local name = project_name(c.project)
	return if name then `{name} · {words}` else words
end

local function chat_title(c: any): string
	local t = c.title
	return if t and t ~= "" then (string.gsub(t, "%s+", " ")) else locale.s("chat.home.cards.untitled")
end

M.needs = ui.computed(function(): { any }
	local out: { any } = {}
	for _, r in live.board:get().needs do
		local item, c = r.item, r.chat
		local meta: string
		if r.kind == "plan" then
			meta = locale.s("chat.home.cards.plan_steps", { n = #(item and item.steps or {}) })
		elseif r.kind == "failed" then
			meta = locale.s("chat.home.cards.failed_ago", { when = ago(c.updated) })
		else
			local a = item and item.ask
			local q = if a and a.question then a.question.text elseif a and a.approval then a.approval.title else nil
			meta = if q and q ~= "" then locale.s("chat.home.cards.asks", { q = (string.gsub(q, "%s+", " ")) }) else locale.s("chat.home.cards.asking")
		end
		table.insert(out, {
			key = if item then item.key else c.id,
			chat = c.id,
			title = chat_title(c),
			meta = meta,
			waits = item ~= nil,
			plan = r.kind == "plan",
			danger = r.kind == "failed",
		})
	end
	for _, t in tasks_store.parts:get().open do
		local reason = (tasks_page.line(t).state :: any).reason
		if reason == "not_sent" or reason == "review" then
			table.insert(out, {
				key = `task-{t.id}`,
				task = t.id,
				title = tasks_page.title_of(t),
				meta = locale.s("chat.home.cards.task", { why = locale.s(`chat.home.cards.task_{reason}`) }),
				danger = reason == "not_sent",
			})
		end
	end
	return out
end)

M.running = ui.computed(function(): { any }
	local out: { any } = {}
	for _, r in live.board:get().working do
		local c = r.chat
		local n = c.running_subagents or 0
		local doing = if n == 0 then locale.s("chat.home.cards.working") else locale.s("chat.home.cards.agents", { n = n })
		table.insert(out, { key = c.id, chat = c.id, title = chat_title(c), meta = with_project(c, doing) })
	end
	return out
end)

M.recent = ui.computed(function(): { any }
	local done: { any } = {}
	for _, c in sessions.now:get().chats or {} do
		if c.status == nil or c.status == "done" then
			table.insert(done, c)
		end
	end
	table.sort(done, function(a: any, b: any): boolean
		return (a.updated or a.created or 0) > (b.updated or b.created or 0)
	end)
	local out: { any } = {}
	for i = 1, math.min(ROWS, #done) do
		local c = done[i]
		local completed = c.status == "done"
		local when = ago(c.updated or c.created)
		table.insert(out, {
			key = c.id,
			chat = c.id,
			title = chat_title(c),
			meta = with_project(c, if completed then locale.s("chat.home.cards.done_ago", { when = when }) else when),
			completed = completed,
		})
	end
	return out
end)

local function first(list: { any }): { any }
	local out = {}
	for i = 1, math.min(ROWS, #list) do
		out[i] = list[i]
	end
	return out
end

local function open_need(r: any)
	if r.task then
		route.plugin_page("zeron", "tasks")
		task_card.open(r.task)
	else
		route.open_session(r.chat)
	end
end

local function button(id: string, label: string, accent: boolean, run: () -> ()): Node
	return ui.button {
		id = id,
		label = label,
		h = ui.raw(26, "gpui card_button"),
		px = ui.raw(10, "gpui card_button"),
		shrink = 0,
		items = "center",
		rounded = ui.raw(7, "gpui card_button"),
		text_size = "$font.size.sm",
		weight = "$font.weight.semibold",
		bg = if accent then "$zeron.home.answer" else "$hover",
		color = if accent then "$accent" else "$text.primary",
		hover = { bg = if accent then "$zeron.home.answer_hover" else "$pressed" },
		on_click = run,
		ui.text { label, literal = true, wrap = false },
	}
end

local function row(id: string, r: any, dot: string, meta_ink: string, on_open: () -> (), action: Node?): Node
	return ui.row {
		id = id,
		class = "home-card-row",
		role = "button",
		label = r.title,
		h = ROW_H,
		px = ui.raw(6, "gpui card_row_shell"),
		gap = ui.raw(8, "gpui card_row_shell"),
		items = "center",
		rounded = ui.raw(9, "gpui card_row_shell"),
		cursor = "pointer",
		hover = { bg = "$hover" },
		on_click = on_open,
		ui.div { w = 7, h = 7, shrink = 0, rounded = "$radius.full", bg = dot },
		ui.column {
			grow = 1,
			min_w = 0,
			ui.text { r.title, literal = true, truncate = true, weight = "$font.weight.medium", color = "$text.primary" },
			ui.text { r.meta, literal = true, truncate = true, text_size = ui.raw(11.5, "gpui card meta"), color = meta_ink },
		},
		action,
	}
end

local function need_row(r: any): Node
	local tint = if r.danger then "$danger" else "$accent"
	local action = if r.waits
		then button(`card-answer-{r.key}`, locale.s(if r.plan then "chat.home.cards.review" else "chat.home.cards.answer"), true, function()
			waiting.open_at(r.key)
		end)
		else button(`card-open-{r.key}`, locale.s("chat.home.cards.open"), false, function()
			open_need(r)
		end)
	return row(`card-need-{r.key}`, r, tint, tint, function()
		open_need(r)
	end, action)
end

local function more(id: string, label: () -> string, hint: boolean, run: () -> ()): Node
	return ui.row {
		id = id,
		role = "button",
		label = label,
		gap = ui.raw(6, "gpui"),
		px = ui.raw(6, "gpui"),
		py = ui.raw(5, "gpui"),
		items = "center",
		rounded = "$radius.lg",
		cursor = "pointer",
		text_size = ui.raw(11.5, "gpui"),
		color = "$text.secondary",
		hover = { bg = "$hover", color = "$text.primary" },
		on_click = run,
		ui.text { label, literal = true, wrap = false },
		if hint then ds.Kbd { keys = keys.combo("shell.toggle_tray"), tone = "quiet", size = "sm" } else nil,
	}
end

local function dim(empty: () -> boolean): any
	return motion.tween(ui.computed(function(): number
		return if empty() then 0.45 else 1
	end), { duration = 150, ease = "cubicBezier(.25,.1,.25,1)" })
end

local column = ui.component(function(props: {
	name: string,
	icon: string,
	title: string,
	list: any,
	build: (any) -> Node,
	count: boolean,
	first: boolean,
	more: Node?,
	action: Node?,
}): Node
	local empty = function(): boolean
		return #props.list:get() == 0
	end
	local hovered = ui.use_state(false)
	return ui.column {
		role = "group",
		label = props.title,
		grow = 1,
		basis = 0,
		min_w = 0,
		pt = ui.raw(10, "gpui work_card"),
		px = ui.raw(8, "gpui work_card"),
		pb = ui.raw(8, "gpui work_card"),
		border_sides = if props.first then nil else { 0, 0, 0, 1 },
		border_color = "$border",
		opacity = dim(empty),
		on_hover = function(on: boolean)
			hovered:set(on)
		end,
		ui.row {
			gap = ui.raw(6, "gpui work_card"),
			px = ui.raw(6, "gpui work_card"),
			pb = ui.raw(6, "gpui work_card"),
			items = "center",
			text_size = "$font.size.sm",
			weight = "$font.weight.semibold",
			color = "$text.secondary",
			Icon { name = props.icon :: any, size = 15, color = "$text.secondary" },
			ui.text { props.title, literal = true, wrap = false },
			ui.row {
				ml = "auto",
				h = 20,
				items = "center",
				ui.show(function(): boolean
					return props.count and not empty() and not (props.action and hovered:get())
				end, ui.text {
					function(): string
						return tostring(#props.list:get())
					end,
					id = `{props.name}-count`,
					weight = "$font.weight.medium",
					tabular = true,
				}),
				if props.action
					then ui.show(function(): boolean
						return hovered:get()
					end, ui.div { mr = ui.raw(-4, "gpui: the button's edge past the count's"), props.action })
					else nil,
			},
		},
		ui.each(ui.computed(function(): { any }
			return first(props.list:get())
		end), props.build, {
			key = function(r: any): string
				return r.key
			end,
			animate = { duration = 220, ease = "out(2)" },
			exit = { opacity = 0, duration = 220 },
		}),
		ui.show(empty, ui.text {
			locale.t("chat.home.cards.nothing"),
			px = ui.raw(6, "gpui work_card"),
			py = ui.raw(6, "gpui work_card"),
			text_size = "$font.size.sm",
			color = "$text.tertiary",
		}),
		props.more,
	}
end)

local function unseen(): boolean
	for _, c in sessions.now:get().chats or {} do
		if c.status == "done" then
			return true
		end
	end
	return false
end

local function mark_read(): Node
	return ui.button {
		id = "card-mark-all-read",
		label = locale.s("chat.home.cards.mark_read_label"),
		tooltip = locale.s("chat.home.cards.mark_read_tip"),
		h = 20,
		px = ui.raw(6, "gpui mark_all_read_button"),
		gap = ui.raw(4, "gpui mark_all_read_button"),
		items = "center",
		rounded = ui.raw(5, "gpui mark_all_read_button"),
		text_size = ui.raw(11.5, "gpui mark_all_read_button"),
		weight = "$font.weight.medium",
		color = "$text.secondary",
		bg = "#00000000",
		hover = { bg = "$hover", color = "$text.primary" },
		on_click = function()
			for _, c in sessions.now:peek().chats do
				if c.status == "done" then
					sessions.seen(c.id)
				end
			end
		end,
		Icon { name = "lucide-check", size = 12, color = "$text.secondary" },
		ui.text { locale.t("chat.home.cards.mark_read"), wrap = false },
	}
end

function M.Cards(): Node
	local hidden_needs = ui.computed(function(): number
		return math.max(0, #M.needs:get() - ROWS)
	end)
	local hidden_wait = ui.computed(function(): boolean
		local list = M.needs:get()
		for i = ROWS + 1, #list do
			if not list[i].waits then
				return false
			end
		end
		return true
	end)
	local hidden_running = ui.computed(function(): number
		return math.max(0, #M.running:get() - ROWS)
	end)
	return ui.row {
		id = "new-work-cards",
		w_full = true,
		items = "stretch",
		text_size = "$font.size.md",
		column {
			id = "card-needs",
			name = "card-needs",
			icon = "bell",
			title = locale.s("chat.home.cards.needs"),
			list = M.needs,
			build = need_row,
			count = true,
			first = true,
			action = ui.show(unseen, mark_read()),
			more = ui.show(function(): boolean
				return hidden_needs:get() > 0
			end, ui.switch(ui.computed(function(): string
				return if hidden_wait:get() then "wait" else "tray"
			end), {
				wait = more("card-needs-more", function(): string
					return locale.s("chat.home.cards.more_waiting", { n = hidden_needs:get() })
				end, false, function()
					local items = waiting.visible(nil)
					if items[1] then
						waiting.open_at(items[1].key)
					end
				end),
				tray = more("card-needs-more", function(): string
					return locale.s("chat.home.cards.more_tray", { n = hidden_needs:get() })
				end, true, function()
					live.open:set(true)
				end),
			})),
		},
		column {
			id = "card-running",
			name = "card-running",
			icon = "speedometer",
			title = locale.s("chat.home.cards.running"),
			list = M.running,
			build = function(r: any): Node
				return row(`card-run-{r.key}`, r, WORKING, "$text.secondary", function()
					route.open_session(r.chat)
				end)
			end,
			count = true,
			first = false,
			more = ui.show(function(): boolean
				return hidden_running:get() > 0
			end, more("card-running-more", function(): string
				return locale.s("chat.home.cards.more", { n = hidden_running:get() })
			end, true, function()
				live.open:set(true)
			end)),
		},
		column {
			id = "card-recent",
			name = "card-recent",
			icon = "clock-circle",
			title = locale.s("chat.home.cards.recent"),
			list = M.recent,
			build = function(r: any): Node
				return row(`card-recent-{r.key}`, r, if r.completed then DONE else "$text.tertiary", "$text.secondary", function()
					route.open_session(r.chat)
				end)
			end,
			count = false,
			first = false,
		},
	}
end

return M
