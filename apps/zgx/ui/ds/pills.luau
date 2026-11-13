local Icon = require("@self/icon")
local AgentMark = require("@self/agent_mark")
local locale = require("@self/locale")
local motion = require("@self/motion")
local glass = require("@self/glass")

local M = {}

M.HEIGHT = 28
M.ROW_GAP = 6

local FRAMES = { 189, 220, 90, 78, 45, 291, 306, 433 }
local FRAME_MS = 175
local DOT, GAP = 2.25, 1.75

export type RunState = "working" | "completed" | "stopped" | "failed"

local function lit(frame: number, dot: number): boolean
	return bit32.extract(FRAMES[frame + 1], dot) == 1
end
M.lit = lit

-- Ticks while `active` says so (a row of pills lives under every composer, most with none).
function M.clock(active: (() -> boolean)?): { frame: any, now: any }
	local frame = ui.use_state(0)
	local now = ui.use_state(os.time())
	local reduced = motion.reduced_now()
	local on = ui.computed(function(): boolean
		return not active or active()
	end)
	local function seconds()
		local t = os.time()
		if t ~= now:peek() then
			now:set(t)
		end
	end
	ui.effect(function()
		if on:get() then
			seconds()
			ui.every(if reduced then 1000 else FRAME_MS, function()
				if not reduced then
					frame:set((frame:peek() + 1) % #FRAMES)
				end
				seconds()
			end)
		end
	end)
	return { frame = frame, now = now }
end

function M.Dots(props: { frame: any, ink: any? }): Node
	local rows: { Node } = {}
	for row = 0, 2 do
		local cells: { Node } = {}
		for col = 0, 2 do
			local dot = row * 3 + col
			table.insert(cells, ui.div {
				w = ui.raw(DOT, "background.rs DOT_SIZE"),
				h = ui.raw(DOT, "background.rs DOT_SIZE"),
				rounded = "$radius.full",
				bg = props.ink or "$zeron.composer.faint",
				opacity = function(): number
					return if lit(props.frame:get(), dot) then 1 else 0
				end,
			})
		end
		table.insert(rows, ui.row { gap = ui.raw(GAP, "background.rs DOT_GAP"), table.unpack(cells) })
	end
	return ui.column { gap = ui.raw(GAP, "background.rs DOT_GAP"), shrink = 0, aria_hidden = true, table.unpack(rows) }
end

function M.Glyph(props: { state: () -> RunState, frame: any, size: number? }): Node
	local size = props.size or 12
	return ui.presence(props.state, function(state: RunState): Node
		if state == "working" then
			return M.Dots { frame = props.frame }
		elseif state == "failed" then
			return ui.div { color = "$danger", Icon { name = "close", size = 11 } }
		elseif state == "stopped" then
			return ui.div { color = "$text.secondary", Icon { name = "stop", size = 11 } }
		end
		return ui.div { color = "$success", Icon { name = "check", size = 11 } }
	end, {
		w = size,
		h = size,
		shrink = 0,
		items = "center",
		justify = "center",
		enter = { opacity = { 0, 1 }, duration = 160 },
		exit = { opacity = 0, duration = 160 },
	})
end

function M.Pill(props: { [any]: any }): Node
	local p = table.clone(props)
	local active = p.active
	p.active = nil
	p.h = M.HEIGHT
	p.px = p.px or ui.raw(10, "background.rs pill_frame")
	p.gap = p.gap or ui.raw(4, "background.rs pill_frame")
	p.shrink = p.shrink or 0
	p.items = "center"
	p.rounded = "$radius.full"
	p.border = 1
	p.border_color = function(): string
		return if active and active() then "$border.strong" else "$zeron.composer.edge"
	end
	glass.fill(p)
	p.color = function(): string
		return if active and active() then "$text.primary" else "$text.secondary"
	end
	p.text_size = ui.raw(13, "background.rs pill_frame")
	p.hover = { border_color = "$border.strong", color = "$text.primary" }
	p.pointer_events = "auto"
	p.wrap = false
	return ui.button(p)
end

-- A popover's content stays mounted once closed: this lets it go after its exit.
function M.while_open(open: any, build: () -> Node): Node
	local alive = ui.use_state(open:peek())
	local stamp = 0
	ui.effect(function()
		stamp += 1
		local mine = stamp
		if open:get() then
			alive:set(true)
		else
			ui.after(200, function()
				if mine == stamp then
					alive:set(false)
				end
			end)
		end
	end)
	return ui.presence(function(): string
		return if alive:get() then "on" else ""
	end, function(which: string): Node
		return if which == "on" then build() else ui.div {}
	end, { w_full = true })
end

function M.Chip(props: { key: any, value: any, id: string? }): Node
	return ui.row {
		id = props.id,
		shrink = 0,
		gap = ui.raw(4, "background.rs detail_chip"),
		px = ui.raw(6, "background.rs detail_chip"),
		py = ui.raw(1, "background.rs detail_chip"),
		items = "center",
		rounded = ui.raw(5, "background.rs detail_chip"),
		bg = "$zeron.secondary.bg",
		text_size = ui.raw(11, "background.rs detail_chip"),
		ui.text { props.key, color = "$zeron.composer.faint", wrap = false },
		ui.text { props.value, color = "$text.secondary", wrap = false, tabular = true },
	}
end

function M.ModelTag(props: { tag: any, mark: string?, icon: string?, id: string? }): Node
	return ui.row {
		id = props.id,
		shrink = 0,
		gap = ui.raw(4, "background.rs model_tag"),
		px = ui.raw(5, "background.rs model_tag"),
		py = ui.raw(1, "background.rs model_tag"),
		items = "center",
		rounded = ui.raw(5, "background.rs model_tag"),
		bg = "$zeron.secondary.bg",
		text_size = ui.raw(11, "background.rs model_tag"),
		color = "$text.secondary",
		if props.mark or props.icon
			then AgentMark { label = props.mark or "agent", mark = props.mark, icon = props.icon, size = 11, bare = true }
			else nil,
		ui.text { props.tag, wrap = false },
	}
end

function M.Stop(props: { id: string?, label: any, on_click: () -> () }): Node
	return ui.button {
		bg = "#00000000",
		id = props.id,
		label = props.label,
		tooltip = locale.s("ds.composer_dock.stop"),
		w = 20,
		h = 20,
		shrink = 0,
		items = "center",
		justify = "center",
		rounded = ui.raw(5, "background.rs stop_button"),
		color = "$zeron.composer.faint",
		hover = { bg = "$hover", color = "$danger" },
		on_click = props.on_click,
		Icon { name = "stop", size = 10 },
	}
end

return M
