local motion = require("@self/motion")
local Button = require("@self/button")
local locale = require("@self/locale")

export type Phase = "open" | "cancel" | "send"
export type Props = {
	text: State<string>,
	from: { w: number, h: number },
	phase: () -> Phase,
	on_cancel: () -> (),
	on_send: () -> (),
	on_done: (phase: Phase) -> (),
}

local OPEN_MS, CANCEL_MS, SEND_MS = 220, 160, 200
local OPEN_EASE, CANCEL_EASE = "cubicBezier(0.2, 0.8, 0.2, 1)", "cubicBezier(0.4, 0, 1, 1)"
local BAR_H = 42
local BAR_FADE_MS, BAR_DELAY_MS = 120, 80

local function lerp(a: number, b: number, t: number): number
	return a + (b - a) * t
end

return ui.component(function(props: Props): Node
	local field = ui.ref()
	local opened = ui.use_state(0)
	local sent = ui.use_state(0)
	local full_w = ui.use_state(props.from.w)
	local text_h = ui.use_state(props.from.h)
	local twin = ui.use_state { w = props.from.w, h = props.from.h }
	local pad_top = (ui.theme.get("zeron.space.snug") :: number?) or 10
	local function natural_h(): number
		return pad_top + text_h:get() + BAR_H
	end
	local moving: any = nil
	local function run(state: State<number>, to: number, ms: number, ease: string, done: (() -> ())?)
		if moving then
			moving:pause()
			moving = nil
		end
		local d = motion.ms(ms)
		if d <= 0 then
			state:set(to)
			if done then
				done()
			end
			return
		end
		local this: any
		this = anime.animate(state, {
			value = { state:peek(), to },
			duration = d,
			ease = ease,
			onComplete = function()
				if moving == this then
					moving = nil
					if done then
						done()
					end
				end
			end,
		})
		moving = this
	end
	local bar_o = ui.use_state(0)
	local function fade_bar(to: number, delay: number)
		anime.animate(bar_o, { value = { bar_o:peek(), to }, duration = motion.ms(BAR_FADE_MS), delay = motion.ms(delay), ease = "linear" })
	end
	local was: Phase? = nil
	ui.effect(function()
		local phase = props.phase()
		if phase == was then
			return
		end
		local before = was
		was = phase
		if phase == "open" then
			if before == "send" then
				run(sent, 0, OPEN_MS, OPEN_EASE)
			else
				run(opened, 1, OPEN_MS, OPEN_EASE)
			end
			fade_bar(1, BAR_DELAY_MS)
		elseif phase == "cancel" then
			fade_bar(0, 0)
			run(opened, 0, CANCEL_MS, CANCEL_EASE, function()
				props.on_done("cancel")
			end)
		elseif phase == "send" then
			local el = field:peek()
			if el then
				el:blur()
			end
			fade_bar(0, 0)
			run(sent, 1, SEND_MS, OPEN_EASE, function()
				props.on_done("send")
			end)
		end
	end)
	ui.on_mount(function()
		local el = field:peek()
		if el then
			el:focus()
			local n = utf8.len(props.text:peek()) or #props.text:peek()
			el:set_selection(n, n)
		end
	end)
	local function empty(): boolean
		return string.match(props.text:get(), "^%s*$") ~= nil
	end
	local function send()
		if not empty() and props.phase() == "open" then
			props.on_send()
		end
	end
	local function measure(extra: { [string]: any }): { [string]: any }
		extra.absolute = true
		extra.opacity = 0
		extra.pointer_events = "none"
		extra.aria_hidden = true
		extra.selectable = false
		return extra
	end
	return ui.column {
		id = "message-edit",
		w_full = true,
		items = "end",
		relative = true,
		on_resize = function(size: { w: number, h: number })
			if math.abs(full_w:peek() - size.w) >= 0.5 then
				full_w:set(size.w)
			end
		end,
		ui.text(measure {
			function(): string
				return props.text:get() .. " "
			end,
			top = 0,
			left = 0,
			w = function(): number
				return math.max(1, full_w:get() - 32)
			end,
			wrap = true,
			text_size = "$zeron.prose.size",
			line_height = "$zeron.prose.line",
			on_resize = function(size: { w: number, h: number })
				if math.abs(text_h:peek() - size.h) >= 0.5 then
					text_h:set(size.h)
				end
			end,
		}),
		ui.column(measure {
			top = 0,
			right = 0,
			max_w = function(): number
				return full_w:get() * 0.8
			end,
			px = "$space.4",
			py = "$zeron.space.snug",
			on_resize = function(size: { w: number, h: number })
				twin:set { w = size.w, h = size.h }
			end,
			ui.text {
				function(): string
					return props.text:get()
				end,
				wrap = true,
				text_size = "$zeron.prose.size",
				line_height = "$zeron.prose.line",
			},
		}),
		ui.column {
			id = "message-edit-box",
			w = function(): number | string
				local o, s = opened:get(), sent:get()
				if o >= 1 and s <= 0 then
					return "100%"
				end
				local open_w = lerp(props.from.w, full_w:get(), o)
				return lerp(open_w, twin:get().w, s)
			end,
			h = function(): number
				local o, s = opened:get(), sent:get()
				local open_h = lerp(props.from.h, natural_h(), o)
				return lerp(open_h, twin:get().h, s)
			end,
			overflow = "hidden",
			rounded = "$zeron.prose.bubble.radius",
			bg = "$zeron.prose.bubble.bg",
			border = 1,
			border_color = function(): string
				return if opened:get() > 0.5 and sent:get() < 0.5 then "$border" else "#00000000"
			end,
			ui.textarea {
				ref = field,
				value = props.text,
				label = locale.t("ds.transcript.edit_field"),
				read_only = function(): boolean
					return props.phase() ~= "open"
				end,
				w_full = true,
				h = function(): number
					local o = opened:get()
					local open_h = math.max(text_h:get(), lerp(props.from.h, natural_h(), o) - pad_top - BAR_H * o)
					return lerp(open_h, math.max(0, twin:get().h - 2 * pad_top), sent:get())
				end,
				shrink = 0,
				mt = "$zeron.space.snug",
				px = "$space.4",
				py = 0,
				border = 0,
				rounded = 0,
				bg = "#00000000",
				focus = { border = 0, outline = 0 },
				wrap = true,
				text_size = "$zeron.prose.size",
				line_height = "$zeron.prose.line",
				color = "$text.primary",
				caret_color = "$zeron.composer.caret",
				caret_width = ui.theme.get("zeron.composer.caret_width"),
				on_key = function(e: any): boolean
					if e.key == "Enter" and not e.shift and not e.alt then
						send()
						return true
					end
					if e.key == "Escape" then
						if props.phase() == "open" then
							props.on_cancel()
						end
						return true
					end
					return false
				end,
			},
			ui.row {
				h = function(): number
					return BAR_H * (1 - sent:get())
				end,
				shrink = 0,
				opacity = function(): number
					return bar_o:get()
				end,
				justify = "end",
				items = "center",
				gap = ui.raw(6, "the mockup's Cancel to Send"),
				px = "$space.2",
				pt = ui.raw(6, "the mockup's 6 over the buttons"),
				pb = "$space.2",
				Button {
					tone = "ghost",
					size = "sm",
					on_click = function()
						if props.phase() == "open" then
							props.on_cancel()
						end
					end,
					ui.text { locale.t("common.cancel") },
				},
				Button {
					tone = "primary",
					size = "sm",
					disabled = empty,
					on_click = send,
					ui.text { locale.t("ds.transcript.edit_send") },
				},
			},
		},
	}
end)
