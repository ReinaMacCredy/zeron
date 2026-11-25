local ds = require("@ds")
local ContextMenu = require("@ds/context_menu")
local SessionRow = require("@ds/session_row")
local Tooltip = require("@ds/tooltip")
local locale = require("@ds/locale")
local motion = require("@ds/motion")
local status = require("@ds/status")
local frame = require("@app/screens/shell/frame")
local route = require("@app/screens/shell/route")
local splits = require("@app/screens/shell/split")
local tabs = require("@app/screens/shell/session_tabs")

local M = {}

local CELL, PANE, MARK, GAP = 34, 32, 16, 2
local TOGGLE_H, CHEVRON = 24, 13
local DOT = 5
local DECK_STEP, DECK_SCALE, DECK_SHOWN = 4, 0.56, 3
local HOVER_MS, FOLD_MS = 150, 200
local HOVER_FADE = "cubicBezier(.4, 0, .2, 1)"
local FOLD_EASE = "cubicBezier(.22, 1, .36, 1)"

M.hidden = ui.persist("shell_rail_tabs_hidden", false)

local function marked(id: string): boolean
	local c = tabs.chat_of(id)
	return c ~= nil and (c.agent ~= nil or c.mark ~= nil)
end

local function marks_of(key: string): { string }
	local out: { string } = {}
	for _, id in splits.panes(key) do
		if marked(id) then
			table.insert(out, id)
		end
	end
	return out
end

local function listed(pinned: boolean): { string }
	local out: { string } = {}
	for _, key in tabs.shown:get() do
		if splits.is_pinned(key) == pinned and #marks_of(key) > 0 then
			table.insert(out, key)
		end
	end
	return out
end
local held = ui.computed(function(): { string }
	return listed(true)
end)
local rest = ui.computed(function(): { string }
	return listed(false)
end)
local all = ui.computed(function(): { string }
	local out = table.clone(held:get())
	table.move(rest:get(), 1, #rest:get(), #out + 1, out)
	return out
end)

local tops = ui.computed(function(): { [string]: number }
	local out: { [string]: number } = {}
	local y = 0
	local rest = rest:get()
	for _, key in all:get() do
		local n = #marks_of(key)
		local rule = if #held:get() > 0 and rest[1] == key then 9 else 0
		out[key] = y + rule
		y += rule + (if n <= 1 then CELL else n * PANE + (n - 1) + 2) + GAP
	end
	return out
end)
local deck_size = ui.computed(function(): number
	return math.min(#all:get(), DECK_SHOWN)
end)

local function count(): number
	local n = 0
	for _, list in { held:get(), rest:get() } do
		for _, key in list do
			n += #marks_of(key)
		end
	end
	return n
end

local function urgent(): string?
	local best: string? = nil
	for _, list in { held:get(), rest:get() } do
		for _, key in list do
			for _, id in marks_of(key) do
				local c = tabs.chat_of(id)
				local kind = if c then c.status else nil
				if kind and SessionRow.URGENCY[kind] and (not best or SessionRow.URGENCY[kind] < SessionRow.URGENCY[best]) then
					best = kind
				end
			end
		end
	end
	return if best and SessionRow.URGENCY[best] <= SessionRow.URGENCY.working then best else nil
end

local function fade(target: any): any
	return ui.tween(target, { duration = math.max(motion.ms(HOVER_MS), 1), ease = HOVER_FADE })
end

local function fill(bg: any, shown: any, rounded: any): Node
	return ui.div {
		absolute = true,
		left = 0,
		right = 0,
		top = 0,
		bottom = 0,
		rounded = rounded,
		bg = bg,
		opacity = fade(shown),
		pointer_events = "none",
	}
end

local Mark = ui.component(function(props: { chat: string, key: string, h: number, rounded: any }): Node
	local id = props.chat
	local over = ui.use_state(false)
	local function chat(): any
		return tabs.chat_of(id) or {}
	end
	local on = ui.computed(function(): number
		return if route.current:get().name == "session" and splits.focused:get() == id then 1 else 0
	end)
	local wash = ui.computed(function(): number
		return if over:get() and on:get() == 0 then 1 else 0
	end)
	local function title(): string
		return chat().title or ""
	end
	return ui.button {
		id = `rail-tab-{id}`,
		label = title,
		tooltip = Tooltip { title },
		tooltip_placement = { side = "right", align = "center", offset = 6 },
		relative = true,
		w = CELL,
		h = props.h,
		shrink = 0,
		p = 0,
		items = "center",
		justify = "center",
		rounded = props.rounded,
		bg = "#00000000",
		hover = { bg = "#00000000" },
		active = { bg = "$pressed" },
		cursor = "pointer",
		on_hover = function(inside: boolean)
			over:set(inside)
		end,
		on_click = function()
			route.open_session(id)
		end,
		on_aux_click = function(e: any)
			if e.button == "middle" then
				tabs.close(props.key)
			end
		end,
		fill(function(): string
			return frame.row_token("tile")
		end, ui.computed(function(): number
			return if splits.is_pinned(props.key) then 1 else 0
		end), props.rounded),
		ui.div {
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			bottom = 0,
			rounded = props.rounded,
			bg = frame.card_fill,
			border = 1,
			border_color = function(): string
				return frame.row_token("rim")
			end,
			-- (Folded into the deck, every mark is a card of its own, so the ones under it don't show through.)
			opacity = fade(ui.computed(function(): number
				return if M.hidden:get() then 1 else on:get()
			end)),
			pointer_events = "none",
		},
		fill("$hover", wash, props.rounded),
		SessionRow.mark(function(): any
			local c = chat()
			return { agent = c.mark or c.agent, harness = c.harness, agent_icon = c.agent_icon }
		end, function(): boolean
			return true
		end, MARK),
		ui.presence(function(): string
			local kind = chat().status
			return if kind and SessionRow.URGENCY[kind] and not M.hidden:get() then kind else ""
		end, function(which: string): Node
			if which == "" then
				return ui.div {}
			end
			return ui.div { id = `rail-tab-{id}-state`, w = CELL, items = "center", tabs.state_mark(which, DOT, true) }
		end, { absolute = true, left = 0, right = 0, bottom = ui.raw(3, "under the mark"), pointer_events = "none" }),
	}
end)

local RADIUS, JOIN = "$radius.xl", ui.raw(4, "where a split's marks meet")

local function menu_cell(key: string): Node
	return ContextMenu {
		label = ui.t("panel.tab.actions"),
		ring = "$radius.xl",
		shrink = 0,
		items = function(): { any }
			return tabs.menu_items(key)
		end,
		ui.presence(function(): string
			return table.concat(marks_of(key), "\0")
		end, function(which: string): Node
			local ids = string.split(which, "\0")
			if which == "" then
				return ui.div {}
			end
			if #ids == 1 then
				return Mark { chat = ids[1], key = key, h = CELL, rounded = RADIUS }
			end
			local box: { [any]: any } = {
				id = `rail-tab-group-{key}`,
				rounded = RADIUS,
				border = 1,
				border_color = function(): string
					return frame.row_token("rim")
				end,
				overflow = "hidden",
			}
			for i, id in ids do
				table.insert(box, Mark { chat = id, key = key, h = PANE, rounded = JOIN })
				if i < #ids then
					table.insert(box, ui.div { h = 1, w_full = true, bg = function(): string
						return frame.row_token("rim")
					end })
				end
			end
			return ui.column(box)
		end, { shrink = 0 }),
	}
end

local Cell = ui.component(function(props: { key: string }): Node
	local key = props.key
	local folded = ui.use_state(if M.hidden:peek() then 1 else 0)
	local function rank(): number
		return (table.find(all:get(), key) or 1) - 1
	end
	ui.effect(function()
		local want = if M.hidden:get() then 1 else 0
		local k = rank()
		anime.animate(folded, { value = want, duration = math.max(motion.ms(FOLD_MS), 1), delay = motion.ms(k * 25), ease = FOLD_EASE })
	end)
	return ui.column {
		shrink = 0,
		items = "center",
		pointer_events = function(): string
			return if M.hidden:get() then "none" else "auto"
		end,
		y = function(): number
			return folded:get() * (rank() * DECK_STEP - (tops:get()[key] or 0))
		end,
		scale = function(): number
			return 1 - folded:get() * (1 - DECK_SCALE)
		end,
		opacity = function(): number
			return if rank() < DECK_SHOWN then 1 else 1 - folded:get()
		end,
		ui.show(function(): boolean
			return #held:get() > 0 and rest:get()[1] == key
		end, ui.div {
			id = "rail-tabs-rule",
			h = 1,
			w = 18,
			my = ui.raw(4, "round the rule"),
			shrink = 0,
			bg = function(): string
				return frame.row_token("rule")
			end,
			opacity = function(): number
				return 1 - folded:get()
			end,
		}),
		menu_cell(key),
	}
end)

local function cell(key: string): Node
	return Cell { key = key }
end

local function each(list: any): Node
	return ui.each(list, cell, {
		key = function(key: string): string
			return key
		end,
		direction = "column",
		gap = ui.raw(GAP, "between the rail's tab marks"),
		animate = motion.layout(motion.DRAG_SLIDE),
	})
end

local function toggle(): Node
	local over = ui.use_state(false)
	local wash = ui.computed(function(): number
		return if over:get() then 1 else 0
	end)
	local function name(): string
		return if M.hidden:get() then locale.s("shell.rail.show_tabs", { n = count() }) else locale.s("shell.rail.hide_tabs")
	end
	return ui.button {
		id = "rail-tabs-toggle",
		label = name,
		tooltip = Tooltip { name },
		tooltip_placement = { side = "right", align = "center", offset = 6 },
		expanded = function(): boolean
			return not M.hidden:get()
		end,
		relative = true,
		direction = "column",
		w = CELL,
		min_h = TOGGLE_H,
		shrink = 0,
		p = 0,
		py = ui.raw(4, "the toggle's room round its chevron"),
		items = "center",
		justify = "center",
		rounded = "$radius.lg",
		bg = "#00000000",
		hover = { bg = "#00000000" },
		active = { bg = "$pressed" },
		cursor = "pointer",
		color = ui.tween(wash, { from = "$text.secondary", to = "$text.primary", duration = math.max(motion.ms(HOVER_MS), 1), ease = HOVER_FADE }),
		on_hover = function(inside: boolean)
			over:set(inside)
		end,
		on_click = ui.toggle(M.hidden),
		fill("$hover", wash, "$radius.lg"),
		ui.div {
			rotate = motion.tween(ui.computed(function(): number
				return if M.hidden:get() then -90 else 0
			end), { duration = FOLD_MS, ease = FOLD_EASE }),
			ds.Icon { name = "lucide-chevron-down", size = CHEVRON },
		},
	}
end

M.Zone = ui.component(function(): Node
	return ui.column {
		id = "rail-tabs",
		role = "group",
		label = function(): string
			return locale.s("shell.rail.tabs")
		end,
		grow = 1,
		min_h = 0,
		w = CELL,
		items = "center",
		gap = ui.raw(4, "the toggle over its marks"),
		ui.show(function(): boolean
			return #held:get() + #rest:get() > 0
		end, toggle()),
		ui.show(function(): boolean
			return #held:get() + #rest:get() > 0
		end, ui.column {
			relative = true,
			grow = 1,
			min_h = 0,
			w = CELL,
			ui.scroll {
				id = "rail-tabs-list",
				grow = 1,
				min_h = 0,
				w = CELL,
				scrollbar = false,
				ui.column {
					w = CELL,
					each(all),
				},
			},
			ui.show(function(): boolean
				return M.hidden:get()
			end, ui.div {
				id = "rail-tabs-deck",
				absolute = true,
				left = 0,
				right = 0,
				top = 0,
				h = function(): number
					return ui.raw((deck_size:get() - 1) * DECK_STEP + CELL, "the deck")
				end,
				cursor = "pointer",
				on_click = function()
					M.hidden:set(false)
				end,
			}),
			ui.presence(function(): string
				return if M.hidden:get() then "count" else ""
			end, function(which: string): Node
				if which == "" then
					return ui.div {}
				end
				return ui.row {
					id = "rail-tabs-count",
					w = CELL,
					justify = "center",
					items = "center",
					gap = ui.raw(3, "the count and its dot"),
					ui.text {
						function(): string
							return tostring(count())
						end,
						color = "$text.primary",
						text_size = ui.raw(11, "the rail's count"),
						weight = "$font.weight.semibold",
						tabular = true,
					},
					ui.presence(function(): string
						return urgent() or ""
					end, function(kind: string): Node
						if kind == "" then
							return ui.div {}
						end
						return ui.div { id = "rail-tabs-urgent", status.StatusDot { kind = kind :: any, size = 7 } }
					end),
				}
			end, {
				absolute = true,
				left = 0,
				right = 0,
				top = function(): number
					return ui.raw((deck_size:get() - 1) * DECK_STEP + CELL * (1 + DECK_SCALE) / 2 + 4, "under the deck")
				end,
				items = "center",
				pointer_events = "none",
				enter = motion.transition({ opacity = { 0, 1 }, scale = { 0.6, 1 }, duration = FOLD_MS, delay = 120, ease = FOLD_EASE }),
				exit = motion.transition({ opacity = 0, duration = 100 }),
			}),
		}),
	}
end)

return M
