local Icon = require("@self/icon")
local AgentMark = require("@self/agent_mark")
local locale = require("@self/locale")
local motion = require("@self/motion")
local glass = require("@self/glass")
local ProgressBar = require("@self/progress").ProgressBar

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

local SEG_MAX = 16
local SEG_INK: { [string]: string } = {
	working = "$accent",
	readyForReview = "$zeron.tone.success.edge",
	done = "$success",
	failed = "$danger",
	waitingOnYou = "$warning",
	quiet = "$warning",
	cancelled = "$text.tertiary",
	skipped = "$text.tertiary",
}

-- A thread's message reaching the chat (the transcript counts them): the plan's working segments beat as its card comes up from them.
M.arrivals = ui.state("pills_thread_arrivals", 0)

function M.Segments(props: { states: () -> { string }, done: () -> number }): Node
	local reduced = motion.reduced_now()
	local beat = ui.use_state(1)
	local arrivals_seen: number? = nil
	ui.effect(function()
		local n = M.arrivals:get()
		local first = arrivals_seen == nil
		arrivals_seen = n
		if first or motion.reduced_now() then
			return
		end
		anime.animate(beat, { value = { 1, 2.6 }, duration = 150, ease = "out(2)" })
		ui.after(150, function()
			anime.animate(beat, { value = { 2.6, 1 }, duration = 150, ease = "out(2)" })
		end)
	end)
	local states = ui.computed(props.states)
	local places = ui.computed(function(): { number }
		local out: { number } = {}
		for i = 1, #states:get() do
			out[i] = i
		end
		return out
	end)
	return ui.presence(function(): string
		return if #states:get() > SEG_MAX then "line" else "segs"
	end, function(which: string): Node
		if which == "line" then
			return ProgressBar {
				size = "thin",
				w = 60,
				value = function(): number?
					local n = #states:peek()
					return if n > 0 then props.done() / n else 0
				end,
			}
		end
		return ui.each(places, function(i: number): Node
			return ui.presence(function(): string
				return states:get()[i] or "planned"
			end, function(st: string): Node
				return ui.div {
					class = `pills-plan-seg pills-plan-seg-{st}`,
					w = ui.raw(9, "the mockup's segment"),
					h = ui.raw(4, "the mockup's segment"),
					rounded = ui.raw(2, "the mockup's segment"),
					bg = SEG_INK[st] or "$border.strong",
					opacity = if st == "working" and not reduced then ui.loop { from = 1, to = 0.45, duration = 700, yoyo = true, fps = 30 } else nil,
					scale_y = if st == "working" then function(): number
						return beat:get()
					end else nil,
				}
			end, { shrink = 0, enter = { opacity = { 0, 1 }, duration = if reduced then motion.FADE else 220 }, exit = { opacity = 0, duration = if reduced then motion.FADE else 220 } })
		end, { direction = "row", gap = ui.raw(2, "the mockup's segments"), items = "center", shrink = 0 })
	end, { shrink = 0 })
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
