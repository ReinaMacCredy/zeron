local ds = require("@ds")
local motion = require("@ds/motion")
local glass = require("@ds/glass")
local material = require("@ds/material")
local Icon = require("@ds/icon")
local host = require("@app/screens/settings/host")
local arrival = require("@app/screens/shell/arrival")
local visuals = require("@app/screens/shell/whats_new_visuals")

local M = {}

local EASE = "cubicBezier(.22,1,.36,1)"
local SPRING = "cubicBezier(.34,1.56,.64,1)"
local HERO = { angle = 135, stops = { "#6d8bff", "#a78bfa", "#f0abfc" } }

M.data = ui.shared("whats_new", nil :: any)
M.dismissed = ui.state("whats_new_dismissed", false)
M.open_now = ui.state("whats_new_open", false)
M.view = ui.state("whats_new_view", "highlights")
local entry = ui.state("whats_new_entry", nil :: any)
local sprung = ui.state("whats_new_sprung", false)
local up = ui.state("whats_new_up", false)
local rise = ui.state("whats_new_rise", 0)
local shown = ui.state("whats_new_shown", 0)
local shine = ui.state("whats_new_shine", 0)

local function card_entry(): any
	local d = M.data:get()
	return d and d.card
end

local function card_visual(): string?
	local e = card_entry()
	return if e and visuals.has(e.hero_visual) then e.hero_visual else nil
end

local function seen()
	host.invoke("whats_new.seen")
end

function M.open(which: any?)
	local d = M.data:peek()
	local e = which or (d and (d.card or d.highlights))
	if not e then
		return
	end
	ui.batch(function()
		entry:set(e)
		M.view:set("highlights")
		M.open_now:set(true)
	end)
end

function M.close()
	M.open_now:set(false)
end

local function dismiss()
	M.dismissed:set(true)
	seen()
end

local was_open = M.open_now:peek()
ui.effect(function()
	local now = M.open_now:get()
	if was_open and not now then
		dismiss()
	end
	was_open = now
end)

local function spring()
	up:set(true)
	if motion.reduced_now() then
		rise:set(1)
		shown:set(1)
		shine:set(1)
		return
	end
	anime.animate(rise, { value = 1, duration = 420, ease = SPRING })
	anime.animate(shown, { value = 1, duration = 220, ease = "out(2)" })
	anime.animate(shine, { value = 1, duration = 900, delay = 420, ease = "inOut(2)" })
end

if up:peek() then
	rise:set(1)
	shown:set(1)
	shine:set(1)
elseif sprung:peek() then
	ui.after(0, spring)
end

ui.effect(function()
	if sprung:peek() or card_entry() == nil or M.dismissed:get() or arrival.waiting() then
		return
	end
	sprung:set(true)
	ui.after(if motion.reduced_now() then 0 else 500, spring)
end)

local card_in_view = ui.state("whats_new_card_in_view", false)
local card_clock = visuals.clock("whats_new_card_clock", function(): boolean
	return card_visual() ~= nil and up:get() and card_in_view:get() and not M.dismissed:get() and not M.open_now:get()
end)
local dialog_clock = visuals.clock("whats_new_dialog_clock", function(): boolean
	local e = entry:get()
	return M.open_now:get() and M.view:get() == "highlights" and e ~= nil and visuals.any(e)
end)

local function hero(props: { h: number, version: (() -> string)?, rounded: any?, shine: boolean?, visual: (() -> string?)? }): Node
	return ui.div {
		relative = true,
		w_full = true,
		h = props.h,
		overflow = "hidden",
		rounded = props.rounded,
		gradient = if props.visual then nil else HERO,
		if props.visual
			then ui.presence(props.visual, function(id: string): Node
				return ui.div {
					absolute = true,
					top = 0,
					left = 0,
					right = 0,
					bottom = 0,
					fade = { 0, 40 },
					on_visible = function(visible: boolean)
						card_in_view:set(visible)
					end,
					ui.div { w_full = true, h_full = true, bg = "$zeron.whats_new.frame", visuals.Stage { visual = id, clock = card_clock } },
				}
			end)
			else nil,
		if props.shine
			then ui.div {
				absolute = true,
				top = 0,
				bottom = 0,
				w = "60%",
				left = function(): string
					return `{-70 + 200 * shine:get()}%`
				end,
				gradient = { angle = 100, stops = { "#ffffff00", "#ffffff8c", "#ffffff00" } },
			}
			else nil,
		if props.version
			then ui.text { props.version, absolute = true, left = 12, bottom = 8, color = ui.raw("#ffffff", "white on the hero's gradient in either theme"), text_size = "$font.size.xl", weight = "$font.weight.semibold", wrap = false }
			else nil,
	}
end

function M.Card(): Node
	return ui.show(function(): boolean
		return card_entry() ~= nil and up:get() and not M.dismissed:get() and not M.open_now:get()
	end, ui.column {
		id = "whats-new-card",
		role = "button",
		label = function(): string
			local e = card_entry()
			return ui.t("shell.whats_new.open_card", { version = if e then e.version else "" }):get()
		end,
		shrink = 0,
		mx = "$space.2",
		mb = "$space.2",
		relative = true,
		rounded = "$radius.xl",
		overflow = "hidden",
		border = 1,
		border_color = "$border",
		bg = function(): string
			return if material.see_through() then "$zeron.composer.tint" else "$zeron.composer.solid"
		end,
		shadow = "$zeron.menu.shadow",
		cursor = "pointer",
		opacity = function(): number
			return shown:get()
		end,
		y = function(): number
			return 14 * (1 - rise:get())
		end,
		hover = { border_color = "$border.strong" },
		on_click = function()
			M.open(card_entry())
		end,
		ui.switch(function(): string
			return if card_visual() then "visual" else "gradient"
		end, {
			visual = hero { h = 96, shine = true, visual = card_visual },
			gradient = hero {
				h = 76,
				shine = true,
				version = function(): string
					local e = card_entry()
					return if e then e.version else ""
				end,
			},
		}),
		ui.column {
			px = "$space.3",
			pt = "$space.2",
			pb = "$space.3",
			gap = ui.raw(2, "the title over its one line, as the mockup's card"),
			ui.text {
				function(): string
					local e = card_entry()
					local f = e and e.features[1]
					return if f then f.title else ""
				end,
				text_size = "$font.size.sm",
				weight = "$font.weight.semibold",
				color = "$text.primary",
				min_w = 0,
				truncate = true,
			},
			ui.text {
				function(): string
					local e = card_entry()
					local f = e and e.features[1]
					return if f then f.description else ""
				end,
				text_size = "$font.size.xs",
				color = "$text.secondary",
				min_w = 0,
				truncate = true,
			},
		},
		ui.button {
			id = "whats-new-card-close",
			label = ui.t("shell.whats_new.dismiss"),
			absolute = true,
			right = 6,
			top = 6,
			w = 20,
			h = 20,
			rounded = "$radius.full",
			items = "center",
			justify = "center",
			bg = ui.raw("#00000040", "a scrim on the hero's gradient"),
			color = ui.raw("#ffffff", "white on the hero's gradient in either theme"),
			hover = { bg = ui.raw("#0000008c", "a scrim on the hero's gradient") },
			on_click = dismiss,
			Icon { name = "close", size = 10 },
		},
	})
end

local Enter = ui.component(function(props: { index: number, [number]: Child }): Node
	local t = ui.use_state(if motion.reduced_now() then 1 else 0)
	local children: { any } = {}
	for _, c in ipairs(props :: any) do
		table.insert(children, c)
	end
	return ui.column {
		w_full = true,
		gap = "$space.2",
		opacity = function(): number
			return t:get()
		end,
		y = function(): number
			return 10 * (1 - t:get())
		end,
		on_mount = function()
			if t:peek() < 1 then
				anime.animate(t, { value = 1, duration = 360, delay = 80 * props.index, ease = EASE })
			end
		end,
		table.unpack(children),
	}
end)

local function dot(ink: string): Node
	return ui.div { w = 6, h = 6, mt = ui.raw(6, "the dot on the first line's middle"), shrink = 0, rounded = "$radius.full", bg = ink }
end

local function improvement(text: string, ink: string): Node
	return ui.row { w_full = true, gap = "$space.2", items = "start", dot(ink), ui.text { text, text_size = "$font.size.sm", color = "$text.secondary", wrap = true, grow = 1, min_w = 0 } }
end

local function highlights(e: any): Node
	local items: { Node } = {}
	for i, f in e.features do
		local visual = visuals.of(f)
		table.insert(items, Enter {
			index = i - 1,
			if visual
				then visuals.Frame { visual = visual, clock = dialog_clock, h = 172 }
				elseif f.image then ui.image { src = f.image, w_full = true, h = 104, fit = "cover", rounded = "$radius.lg", border = 1, border_color = "$border" }
				else hero { h = 104, rounded = "$radius.lg" },
			ui.text { f.title, text_size = "$font.size.md", weight = "$font.weight.semibold", color = "$text.primary", wrap = true },
			ui.text { f.description, text_size = "$font.size.sm", color = "$text.secondary", wrap = true },
		})
	end
	if #e.improvements > 0 then
		local lines: { Node } = { ui.text { ui.t("shell.whats_new.improvements"), text_size = "$font.size.md", weight = "$font.weight.semibold", color = "$text.primary" } }
		for _, item in e.improvements do
			table.insert(lines, improvement(item.text, "$success"))
			local visual = visuals.of(item)
			if visual then
				table.insert(lines, ui.div { w_full = true, pl = "$space.3", visuals.Frame { visual = visual, clock = dialog_clock, h = 104 } })
			end
		end
		table.insert(items, Enter { index = #e.features, table.unpack(lines) })
	end
	return ui.column { id = "whats-new-highlights", w_full = true, gap = "$space.4", table.unpack(items) }
end

local function changelog(): Node
	local d = M.data:peek()
	local items: { Node } = {}
	for _, r in (d and d.releases) or {} do
		table.insert(items, ui.row {
			gap = "$space.2",
			items = "baseline",
			ui.text { r.version, text_size = "$font.size.md", weight = "$font.weight.semibold", color = "$text.primary" },
			ui.text { r.date, text_size = "$font.size.xs", color = "$text.tertiary" },
		})
		for _, f in r.features do
			table.insert(items, improvement(f.title, "$text.tertiary"))
		end
		for _, item in r.improvements do
			table.insert(items, improvement(item.text, "$text.tertiary"))
		end
	end
	return ui.column { id = "whats-new-changelog", w_full = true, gap = "$space.1", table.unpack(items) }
end

local Body = ui.component(function(): Node
	local phase = ui.use_state(if motion.reduced_now() then 1 else 0)
	local fade = ui.use_state(0)
	local e = entry:peek()
	local inner = ui.ref()
	local panel = glass.style({ rounded = "$zeron.dialog.radius" }, "menu")
	panel.bg = function()
		return if material.menus_see_through() then "$zeron.dialog.tint" else "$zeron.dialog.solid"
	end
	panel.shadow = "$zeron.dialog.shadow"
	return ui.column({
		w_full = true,
		max_h = 520,
		overflow = "hidden",
		bg = panel.bg,
		backdrop_blur = panel.backdrop_blur,
		border = panel.border,
		border_color = panel.border_color,
		rounded = panel.rounded,
		shadow = panel.shadow,
		opacity = function(): number
			return fade:get()
		end,
		scale = function(): number
			return 0.96 + 0.04 * phase:get()
		end,
		on_mount = function()
			if motion.reduced_now() then
				anime.animate(fade, { value = 1, duration = 150, ease = "linear" })
			else
				anime.animate(phase, { value = 1, duration = 240, ease = EASE })
				anime.animate(fade, { value = 1, duration = 160, ease = "out(2)" })
			end
		end,
		ui.row {
			shrink = 0,
			px = "$space.4",
			pt = "$space.4",
			pb = "$space.2",
			gap = "$space.2",
			items = "center",
			border_sides = { 0, 0, 1, 0 },
			border_color = "$border.subtle",
			ui.text {
				ui.t("shell.whats_new.title", { version = if e then e.version else "" }),
				heading = 2,
				text_size = "$font.size.lg",
				weight = "$font.weight.semibold",
				color = "$text.primary",
				grow = 1,
				min_w = 0,
				truncate = true,
			},
			ds.Segmented {
				id = "whats-new-view",
				value = M.view,
				label = ui.t("shell.whats_new.view"),
				options = {
					{ value = "highlights", label = ui.t("shell.whats_new.highlights") },
					{ value = "changelog", label = ui.t("shell.whats_new.changelog") },
				},
			},
		},
		ui.scroll {
			h = function(): number
				return math.min(460, inner.height or 0)
			end,
			w_full = true,
			fade = { 12, 12 },
			ui.column {
				ref = inner,
				w_full = true,
				px = "$space.4",
				pt = "$space.3",
				pb = "$space.4",
				ui.switch(function(): string
					return M.view:get()
				end, {
					highlights = if e then highlights(e) else ui.div {},
					changelog = changelog(),
				}),
			},
		},
	})
end)

function M.Dialog(): Node
	return ui.modal {
		open = M.open_now,
		label = ui.t("shell.whats_new.view") :: any,
		w = 480,
		p = 0,
		gap = 0,
		rounded = "$zeron.dialog.radius",
		bg = "#00000000",
		border = 0,
		shadow = "$zeron.dialog.stage",
		autofocus = false,
		build = function(): Node
			return Body {}
		end,
	}
end

return M
