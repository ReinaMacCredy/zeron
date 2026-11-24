local ds = require("@ds")
local Icon = require("@ds/icon")
local Ask = require("@ds/composer_ask")
local P = require("@ds/pills")
local FadeText = require("@ds/fade_text")
local Menu = require("@ds/menu")
local locale = require("@ds/locale")
local sessions = require("@account/sessions")
local route = require("@app/screens/shell/route")
local store = require("@app/screens/chat/store")
local shared = require("@app/screens/chat/shared")
local motion = require("@ds/motion")

local M = {}

local CARD_W, TAB_W = 520, 180

local list = ui.shared("chat.waiting", {} :: { any })

local kept: { [string]: { chosen: any, text: any } } = {}
local function held(key: string, page: number): { chosen: any, text: any }
	local slot = `{key}_{page}`
	local now = kept[slot]
	if not now then
		now = { chosen = ui.state(`pills_waiting_chosen_{slot}`, {} :: { [string]: boolean }), text = ui.state(`pills_waiting_text_{slot}`, "") }
		kept[slot] = now
	end
	return now
end

local function project_name(id: string?): string
	if not id then
		return ""
	end
	for _, p in sessions.now:get().projects do
		if p.id == id then
			return p.name
		end
	end
	return ""
end

local WHY = { offline = true, signed_out = true, failed = true, missing = true }

local function fail(err: any)
	local why = if type(err) == "table" and err.message then tostring(err.message) else tostring(err)
	ds.toast.show { title = locale.s("ds.pills.answer_failed", { err = if WHY[why] then locale.s(`ds.pills.why.{why}`) else why }), tone = "danger" }
end

function M.answer(item: any, args: any, on_fail: (() -> ())?)
	args.id = item.chat
	args.request = item.request
	local function failed(err: any)
		fail(err)
		if on_fail then
			on_fail()
		end
	end
	local p = store.waiting_answer(args)
	if p then
		p:after(function() end, failed)
	else
		failed("failed")
	end
end

function M.visible(here: string?): { any }
	local out: { any } = {}
	for _, item in list:get() do
		if (item.chat ~= here or item.kind == "plan_approval") and not shared.plan_dismissed(item) then
			table.insert(out, item)
		end
	end
	return out
end

local function question_glyph(): Node
	return ui.div {
		w = 14,
		h = 14,
		shrink = 0,
		items = "center",
		justify = "center",
		rounded = "$radius.full",
		border = 1,
		border_color = "$accent",
		ui.text { "?", text_size = ui.raw(9.5, "waiting.rs question_glyph"), weight = "$font.weight.bold", color = "$accent" },
	}
end

local function kind_glyph(kind: string): Node
	if kind == "plan" or kind == "plan_approval" then
		return ui.div { color = "$text.secondary", Icon { name = "checklist", size = 13 } }
	end
	return question_glyph()
end

local function ask_body(item: any): Node
	local a = item.ask
	if item.secret then
		local q = a.question or {}
		return ui.column {
			id = "pills-waiting-secret",
			px = ui.raw(16, "ask_pill.rs secret_body"),
			py = ui.raw(14, "ask_pill.rs secret_body"),
			gap = ui.raw(6, "ask_pill.rs secret_body"),
			ui.text { q.text or "", wrap = true, text_size = ui.raw(13, "ask_pill.rs secret_body"), color = "$text.secondary" },
			ui.row {
				gap = ui.raw(6, "ask_pill.rs secret_body"),
				items = "center",
				color = "$text.secondary",
				Icon { name = "shield-check", size = 13 },
				ui.text { locale.t("ds.pills.secret"), text_size = ui.raw(12, "ask_pill.rs secret_body"), wrap = true },
			},
		}
	end
	if a.kind == "approval" then
		return ui.column {
			px = "$space.4",
			pt = ui.raw(6, "waiting.rs body"),
			pb = "$space.2",
			gap = "$space.2",
			Ask.Approval { approval = a.approval },
			ui.row {
				gap = ui.raw(6, "waiting.rs plan actions"),
				ds.Button { locale.t("ds.composer.deny"), id = "pills-waiting-deny", tone = "secondary", size = "sm", on_click = function()
					M.answer(item, { decision = "deny" })
				end },
				ds.Button { locale.t("ds.composer.allow"), id = "pills-waiting-allow", size = "sm", on_click = function()
					M.answer(item, { decision = "once" })
				end },
			},
		}
	end
	local q = a.question
	local h = held(item.key, q.page or 1)
	local sending = false
	local function submit()
		if sending then
			return
		end
		local values: { string } = {}
		for _, o in q.options or {} do
			if h.chosen:peek()[o.value] then
				table.insert(values, o.value)
			end
		end
		local text = h.text:peek()
		if #values == 0 and not string.find(text, "%S") then
			return
		end
		sending = true
		M.answer(item, { decision = "answer", values = values, text = text }, function()
			sending = false
		end)
	end
	local last = (q.page or 1) >= (q.pages or 1)
	local function pick(value: string)
		local now = table.clone(h.chosen:peek())
		if q.multi then
			now[value] = not now[value] or nil
		else
			now = { [value] = true }
		end
		h.chosen:set(now)
	end
	return ui.column {
		id = "pills-waiting-question",
		pt = ui.raw(2, "waiting.rs body"),
		pb = "$space.2",
		gap = "$space.1",
		on_key = function(e: any): boolean
			local digit = tonumber(string.match(e.key or "", "^(%d)$"))
			if digit and not (e.cmd or e.ctrl or e.alt or e.meta or e.shift) and (not e.editing or h.text:peek() == "") then
				local o = (q.options or {})[digit]
				if o then
					pick(o.value)
					return true
				end
			end
			return false
		end,
		Ask.Question { question = q, chosen = h.chosen, on_pick = pick },
		ui.div { px = "$space.4", w_full = true, ds.TextField {
			id = "pills-waiting-input",
			value = h.text,
			name = locale.t("ds.composer.placeholder.answer"),
			placeholder = function(): string
				local any = next(h.chosen:get()) ~= nil
				return locale.s(if any then "ds.composer.placeholder.answer_picked" else "ds.composer.placeholder.answer")
			end,
			autofocus = true,
			size = "sm",
			on_key = function(e: any): boolean
				if e.key == "Enter" and not e.shift then
					submit()
					return true
				end
				return false
			end,
		} },
		ui.row {
			gap = ui.raw(6, "waiting.rs actions"),
			pt = "$space.1",
			px = "$space.4",
			justify = "end",
			if (q.page or 1) > 1
				then ds.Button { locale.t("ds.composer.back"), id = "pills-waiting-back", tone = "ghost", size = "sm", on_click = function()
					local p = store.waiting_back(item.chat, item.request)
					if p then
						p:after(function(page: any)
							local before = held(item.key, (q.page or 1) - 1)
							if page and next(before.chosen:peek()) == nil and before.text:peek() == "" then
								local now: { [string]: boolean } = {}
								for _, v in page.chosen or page do
									now[v] = true
								end
								before.chosen:set(now)
								before.text:set(page.text or "")
							end
						end, fail)
					else
						fail("failed")
					end
				end }
				else nil,
			ds.Button {
				locale.t(if last then "ds.composer.submit" else "ds.composer.next"),
				id = "pills-waiting-submit",
				size = "sm",
				disabled = function(): boolean
					return next(h.chosen:get()) == nil and not string.find(h.text:get(), "%S")
				end,
				on_click = submit,
			},
		},
	}
end

local function plan_body(item: any): Node
	local body: { any } = {
		id = "pills-waiting-plan",
		px = "$space.4",
		pt = ui.raw(6, "waiting.rs plan"),
		pb = "$space.2",
		gap = ui.raw(6, "waiting.rs plan"),
		ui.text {
			string.upper(locale.s("ds.pills.plan_head", { n = #item.steps })),
			text_size = ui.raw(10.5, "waiting.rs plan"),
			weight = "$font.weight.medium",
			color = "$text.secondary",
		},
	}
	for i, step in item.steps do
		table.insert(body, ui.row {
			gap = "$space.2",
			items = "center",
			ui.div {
				w = 18,
				h = 18,
				shrink = 0,
				items = "center",
				justify = "center",
				rounded = "$radius.full",
				border = 1,
				border_color = "$border.strong",
				ui.text { tostring(i), text_size = ui.raw(10.5, "waiting.rs plan step"), color = "$text.secondary" },
			},
			FadeText { step, literal = true, min_w = 0, color = "$text.primary" },
		})
	end
	table.insert(body, ui.text { locale.t("ds.pills.nothing_runs"), text_size = ui.raw(12, "waiting.rs plan"), color = "$zeron.composer.faint" })
	table.insert(body, ui.row {
		pt = "$space.1",
		gap = ui.raw(6, "waiting.rs plan actions"),
		ds.Button { locale.t("ds.pills.approve"), id = "pills-waiting-approve", size = "sm", on_click = function()
			M.answer(item, { decision = "once" })
		end },
		if item.kind == "plan_approval" and item.mark
			then ds.Button { locale.t("common.dismiss"), id = "pills-waiting-dismiss", tone = "secondary", size = "sm", on_click = function()
				shared.dismiss_plan(item)
			end }
			else nil,
	})
	return ui.column(body)
end

M.want = ui.state("chat_waiting_want", nil :: string?)

function M.open_at(key: string)
	M.want:set(key)
end

local Card = ui.component(function(props: { items: any, on_close: () -> () }): Node
	local current = ui.use_state(nil :: string?)
	ui.effect(function()
		local key = M.want:get()
		if key then
			current:set(key)
			M.want:set(nil)
		end
	end)
	local ix = 1
	local function now(): any
		local items = props.items:get()
		local want = current:get()
		for i, it in items do
			if it.key == want then
				ix = i
				return it
			end
		end
		return items[math.min(ix, #items)]
	end
	local shown = ui.computed(now)
	local function kind_word(kind: string): string
		return locale.s(if kind == "plan" or kind == "plan_approval" then "ds.pills.plan" else "ds.pills.question")
	end
	return ui.column {
		id = "pills-waiting-card",
		role = "dialog",
		label = locale.t("ds.pills.waiting_card"),
		w_full = true,
		text_size = ui.raw(13, "waiting.rs card"),
		ui.row {
			h = 38,
			gap = ui.raw(6, "waiting.rs header"),
			items = "center",
			pl = ui.raw(10, "waiting.rs header"),
			pr = "$space.2",
			border = 1,
			border_sides = { 0, 0, 1, 0 },
			border_color = "$border.subtle",
			text_size = ui.raw(12.5, "waiting.rs header"),
			ui.row {
				id = "pills-waiting-tabs",
				role = "tablist",
				label = locale.t("ds.pills.waiting_card"),
				grow = 1,
				basis = 0,
				min_w = 0,
				gap = "$space.1",
				overflow = "hidden",
				ui.each(props.items, function(tab: any): Node
					local function selected(): boolean
						local s = shown:get()
						return s ~= nil and s.key == tab.key
					end
					return ui.button {
						class = "pills-waiting-tab",
						role = "tab",
						label = tab.title or locale.t("ds.pills.untitled"),
						selected = selected,
						min_w = 32,
						shrink = function(): number
							return if selected() then 0 else 1
						end,
						h = 26,
						gap = ui.raw(6, "waiting.rs tab"),
						items = "center",
						px = "$space.2",
						rounded = ui.raw(7, "waiting.rs tab"),
						overflow = "hidden",
						bg = function(): string
							return if selected() then "$zeron.secondary.bg" else "#00000000"
						end,
						color = function(): string
							return if selected() then "$text.primary" else "$text.secondary"
						end,
						hover = { bg = "$hover", color = "$text.primary" },
						on_click = function()
							current:set(tab.key)
						end,
						kind_glyph(tab.kind),
						FadeText { tab.title or locale.t("ds.pills.untitled"), literal = tab.title ~= nil, min_w = 0, max_w = TAB_W },
					}
				end, {
					key = function(t: any): string
						return t.key
					end,
					direction = "row",
				}),
			},
			ui.button {
				bg = "#00000000",
				id = "pills-waiting-close",
				label = locale.t("ds.pills.close"),
				w = 22,
				h = 22,
				shrink = 0,
				items = "center",
				justify = "center",
				rounded = "$radius.sm",
				color = "$text.secondary",
				hover = { bg = "$hover" },
				on_click = props.on_close,
				Icon { name = "close", size = 12 },
			},
		},
		ui.presence(function(): string
			local s = shown:get()
			return if s then `{s.key}\0{(s.ask and s.ask.question and s.ask.question.page) or 1}` else ""
		end, function(which: string): Node
			if which == "" then
				return ui.div {}
			end
			local item = shown:peek()
			local from = project_name(item.project)
			return ui.column {
				w_full = true,
				ui.row {
					px = "$space.4",
					pt = ui.raw(12, "waiting.rs who"),
					gap = "$space.1",
					text_size = ui.raw(11.5, "waiting.rs who"),
					color = "$text.secondary",
					ui.text { kind_word(item.kind), weight = "$font.weight.medium", color = "$accent", wrap = false },
					if from ~= "" then ui.text { `· {from}`, wrap = false } else nil,
				},
				if item.kind == "plan" or item.kind == "plan_approval" then plan_body(item) else ask_body(item),
			}
		end, { w_full = true, enter = { opacity = { 0, 1 }, x = { 8, 0 }, duration = 150 }, exit = { opacity = 0, duration = 80 } }),
		ui.row {
			gap = "$space.1",
			items = "center",
			px = "$space.4",
			pt = ui.raw(2, "waiting.rs footer"),
			pb = "$space.2",
			text_size = ui.raw(11.5, "waiting.rs footer"),
			color = "$zeron.composer.faint",
			ui.text { locale.t("ds.pills.later"), grow = 1, wrap = false },
			ui.button {
				bg = "#00000000",
				id = "pills-waiting-open",
				label = locale.t("ds.pills.open_thread"),
				px = ui.raw(6, "waiting.rs footer link"),
				h = 22,
				items = "center",
				rounded = "$radius.sm",
				color = "$text.secondary",
				hover = { bg = "$hover", color = "$text.primary" },
				on_click = function()
					local s = shown:peek()
					props.on_close()
					if s then
						route.open_session(s.chat)
					end
				end,
				ui.text { locale.t("ds.pills.open_thread"), wrap = false },
			},
		},
	}
end)

local function kind_of(item: any): string
	if item.kind == "plan" or item.kind == "plan_approval" then
		return "plan"
	end
	return if item.ask and item.ask.kind == "approval" then "approval" else "question"
end

local function waiting_words(items: { any }): string
	local kind: string? = nil
	for _, item in items do
		local k = kind_of(item)
		if kind and kind ~= k then
			return locale.s("ds.pills.waiting_mixed")
		end
		kind = k
	end
	return locale.s(`ds.pills.waiting_{kind or "question"}`, { n = #items })
end

local function fresh_words(item: any): string
	local kind = kind_of(item)
	if kind == "plan" then
		return item.steps[1] or locale.s("ds.pills.approve_plan")
	end
	if kind == "approval" then
		return item.ask.approval.title or ""
	end
	return (item.ask.question or {}).text or ""
end

local FRESH_MS, FRESH_W = 4000, 640
local known: { [string]: boolean } = {}

function M.Pill(props: { open: any, width: () -> number, elsewhere_of: () -> string? }): Node
	local items = ui.computed(function(): { any }
		return M.visible(props.elsewhere_of())
	end)
	ui.effect(function()
		if #items:get() == 0 and props.open:peek() then
			props.open:set(false)
		end
	end)
	ui.effect(function()
		if M.want:get() and not props.open:peek() then
			props.open:set(true)
		end
	end)
	local fresh = ui.use_state(nil :: any)
	local alive = ui.use_state(false)
	local wide = ui.use_state(false)
	local turn = 0
	local function fold()
		turn += 1
		local mine = turn
		wide:set(false)
		ui.after(260, function()
			if mine == turn then
				alive:set(false)
			end
		end)
	end
	ui.effect(function()
		local shown: { [string]: any } = {}
		for _, item in items:get() do
			shown[item.key] = item
		end
		local here = props.elsewhere_of()
		local now: { [string]: boolean } = {}
		local came: any = nil
		for _, item in list:get() do
			now[item.key] = true
			if not known[item.key] and shown[item.key] and item.chat ~= here and not came then
				came = item
			end
		end
		known = now
		if not came or props.open:peek() then
			return
		end
		turn += 1
		local mine = turn
		fresh:set(came)
		alive:set(true)
		ui.after(0, function()
			if mine == turn then
				wide:set(true)
			end
		end)
		ui.after(FRESH_MS, function()
			if mine == turn then
				fold()
			end
		end)
	end)
	ui.effect(function()
		if props.open:get() and alive:peek() then
			turn += 1
			wide:set(false)
			alive:set(false)
		end
	end)
	local quick = motion.reduced_now()
	local function of_fresh(read: (any) -> string): () -> string
		return function(): string
			local item = fresh:get()
			return if item then read(item) else ""
		end
	end
	local pill = P.Pill {
		id = "pills-waiting",
		class = "pills-pill",
		shrink = 1,
		min_w = 0,
		label = function(): string
			return locale.s("ds.pills.waiting_label", { n = #items:get() })
		end,
		active = function(): boolean
			return props.open:get()
		end,
		expanded = function(): boolean
			return props.open:get()
		end,
		on_click = function()
			props.open:set(not props.open:peek())
		end,
		ui.div { mr = ui.raw(4, "background.rs pill_leading"), shrink = 0, color = "$warning", Icon { name = "clock-circle", size = 14 } },
		ui.text { function(): string
			return tostring(#items:get())
		end, id = "pills-waiting-count", shrink = 0, weight = "$font.weight.semibold", color = "$warning", tabular = true, wrap = false },
		ui.text { function(): string
			return waiting_words(items:get())
		end, id = "pills-waiting-words", literal = true, shrink = 0, color = "$warning", wrap = false },
		ui.show(function(): boolean
			return alive:get()
		end, ui.row {
			id = "pills-waiting-fresh",
			items = "center",
			gap = ui.raw(4, "background.rs pill_frame"),
			min_w = 0,
			shrink = 1,
			overflow = "hidden",
			max_w = ui.tween(ui.computed(function(): number
				return if wide:get() then FRESH_W else 0
			end), { duration = if quick then 1 else 240, ease = "out(3)" }),
			opacity = ui.tween(ui.computed(function(): number
				return if wide:get() then 1 else 0
			end), { duration = if quick then 120 else 150 }),
			ui.div { w = 1, h = 12, mx = ui.raw(4, "the mockup's rule"), shrink = 0, bg = "$border.strong" },
			ui.text { of_fresh(function(item: any): string
				return item.title or locale.s("ds.pills.untitled")
			end), literal = true, wrap = false, truncate = true, min_w = 0, max_w = ui.raw(220, "the mockup's asker"), shrink = 0, weight = "$font.weight.medium", color = "$text.primary" },
			ui.text { of_fresh(function(item: any): string
				return locale.s(`ds.pills.fresh_{kind_of(item)}`)
			end), literal = true, wrap = false, shrink = 0, color = "$text.secondary" },
			ui.text { of_fresh(fresh_words), literal = true, wrap = false, truncate = true, min_w = 0, max_w = ui.raw(400, "the mockup's question"), shrink = 1, color = "$text.secondary" },
		}),
		ui.show(function(): boolean
			local here = props.elsewhere_of()
			for _, it in items:get() do
				if it.kind == "plan_approval" and it.chat == here then
					return true
				end
			end
			return false
		end, ui.text { locale.t("ds.pills.approve_plan"), id = "pills-waiting-plan-here", ml = ui.raw(4, "the mockup's waiting pill"), wrap = false }),
	}
	return ui.presence(function(): string
		return if #items:get() > 0 then "waiting" else ""
	end, function(which: string): Node
		if which == "" then
			return ui.div { w = 0, h = 0 }
		end
		return ui.popover(Menu.panel {
			class = "pills-waiting-panel",
			open = props.open,
			anchor = pill,
			w = function(): number
				return math.min(CARD_W, props.width())
			end,
			placement = { side = "top", align = "start", offset = 8 },
			build = function(): Node
				return P.while_open(props.open, function(): Node
					return Card {
						items = items,
						on_close = function()
							props.open:set(false)
						end,
					}
				end)
			end,
		})
	end, { enter = { opacity = { 0, 1 }, y = { 4, 0 }, duration = 150 }, exit = { opacity = 0, duration = 120 }, shrink = 1, min_w = 0 })
end

return M
