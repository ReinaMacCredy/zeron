local Menu = require("@ds/menu")
local motion = require("@ds/motion")

local M = {}

local RADIUS, ROW = ui.raw(12, "the Bots menus' corner"), ui.raw(31, "the Bots menus' row")
local OPEN_MS, CLOSE_MS, SWITCH_MS = 140, 90, 160
local EASE = "cubicBezier(.2, .8, .2, 1)"

export type Entry = {
	id: string?,
	label: any?,
	icon: any?,
	danger: boolean?,
	confirm: any?,
	value: (() -> string)?,
	checked: (() -> boolean)?,
	switch: (() -> boolean)?,
	keep_open: boolean?,
	disabled: boolean?,
	on_select: (() -> ())?,
	separator: boolean?,
	submenu: Sub?,
}
export type Sub = { title: any?, width: number?, entries: () -> { Entry } }

local function Switch(on: () -> boolean): Node
	local at = ui.computed(function(): number
		return if on() then 1 else 0
	end)
	local ms = math.max(motion.ms(SWITCH_MS), 1)
	return ui.div {
		relative = true,
		w = ui.raw(26, "the mockup's switch"),
		h = ui.raw(15, "the mockup's switch"),
		shrink = 0,
		rounded = "$radius.full",
		bg = "$border.strong",
		aria_hidden = true,
		ui.div {
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			bottom = 0,
			rounded = "$radius.full",
			bg = "$info",
			opacity = ui.tween(at, { duration = ms, ease = "outQuad" }),
		},
		ui.div {
			absolute = true,
			left = 2,
			top = 2,
			w = ui.raw(11, "the mockup's switch knob"),
			h = ui.raw(11, "the mockup's switch knob"),
			rounded = "$radius.full",
			bg = ui.raw("#ffffff", "the mockup's switch knob"),
			x = ui.tween(at, { from = 0, to = 11, duration = ms, ease = EASE }),
		},
	}
end

local function value(words: () -> string): Node
	return ui.text { words, literal = true, shrink = 0, text_size = "$font.size.sm", color = "$text.secondary" }
end

local function rule(): Node
	return ui.div {
		h = 1,
		shrink = 0,
		mx = ui.raw(4, "the mockup's menu rule"),
		my = ui.raw(5, "the mockup's menu rule"),
		bg = "$border.subtle",
		aria_hidden = true,
	}
end

local panel: (props: { [any]: any }) -> { [any]: any }

local list: (entries: { Entry }, title: any?, close: () -> ()) -> Node

local function trail_of(e: Entry): { Node }
	local trail: { Node } = {}
	if e.value then
		table.insert(trail, value(e.value))
	end
	if e.switch then
		table.insert(trail, Switch(e.switch))
	end
	return trail
end

local function row(e: Entry, label: any, more: { [any]: any }): { [any]: any }
	local ink = if e.disabled then "$text.disabled" elseif e.danger then "$danger" else "$text.primary"
	local p = Menu.look(more)
	p.id = e.id
	p.h = ROW
	p.label = label
	p.disabled = e.disabled
	if e.disabled then
		p.cursor = "default"
		p.hover = {}
		p.active = {}
	elseif e.danger then
		p.hover = { bg = "$zeron.menu.danger" }
		p.active = { bg = "$zeron.menu.danger" }
	end
	table.insert(p, if e.icon then Menu.glyph(e.icon, if e.disabled or e.danger then ink else nil) else Menu.glyph_space())
	table.insert(p, ui.text { label, truncate = true, grow = 1, min_w = 0, text_size = "$font.size.md", color = ink })
	for _, t in trail_of(e) do
		table.insert(p, t)
	end
	return p
end

local SubRow = ui.component(function(props: { entry: Entry, close: () -> () }): Node
	local e = props.entry
	local sub = e.submenu :: Sub
	local open = ui.use_state(false)
	local trigger = row(e, e.label, {
		role = "menuitem",
		focus = { bg = "$hover", outline = 0 },
		bg = function(): string
			return if open:get() then "$hover" else "#00000000"
		end,
	})
	table.insert(trigger, Menu.chevron())
	return ui.submenu(panel {
		open = open,
		label = e.label,
		w = ui.raw(sub.width or 210, "the mockup's submenu"),
		trigger = ui.menu_item(trigger :: any),
		list(sub.entries(), sub.title, props.close),
	}) :: any
end)

local ItemRow = ui.component(function(props: { entry: Entry, close: () -> () }): Node
	local e = props.entry
	local armed = ui.use_state(false)
	local label = if e.confirm
		then function(): string
			local words = if armed:get() then e.confirm else e.label
			return if type(words) == "string" then words elseif type(words) == "function" then words() else words:get()
		end
		else e.label
	local state = e.switch or e.checked
	local p = row(e, label, {
		role = "menuitem",
		focusable = not e.disabled,
		checked = state,
		on_click = if e.disabled
			then nil
			else function()
				if e.confirm and not armed:peek() then
					armed:set(true)
					return
				end
				if not e.keep_open then
					props.close()
				end
				if e.on_select then
					e.on_select()
				end
			end,
	})
	if e.checked and not e.switch then
		table.insert(p, Menu.check(e.checked))
	end
	return ui.row(p)
end)

list = function(entries: { Entry }, title: any?, close: () -> ()): Node
	local rows: { [any]: any } = { p = ui.raw(5, "the mockup's menu inset"), gap = 0 }
	if title then
		table.insert(
			rows,
			ui.row {
				w_full = true,
				px = "$space.2",
				pt = ui.raw(6, "the mockup's menu title"),
				pb = ui.raw(4, "the mockup's menu title"),
				shrink = 0,
				ui.text { title, text_size = "$font.size.xs", color = "$text.secondary" },
			}
		)
	end
	for _, e in entries do
		if e.separator then
			table.insert(rows, rule())
		elseif e.submenu then
			table.insert(rows, SubRow { entry = e, close = close })
		else
			table.insert(rows, ItemRow { entry = e, close = close })
		end
	end
	rows.on_key = function(e: any): boolean
		if e.key == "Escape" then
			close()
			return true
		end
		return false
	end
	return Menu.Group(rows)
end

panel = function(props: { [any]: any }): { [any]: any }
	local open = props.shown or props.open
	props.shown = nil
	local shown = ui.computed(function(): number
		return if open:get() then 1 else 0
	end)
	local ms = math.max(motion.ms(OPEN_MS), 1)
	props.rounded = RADIUS
	props.origin = { 0, 0 }
	props.opacity = ui.tween(shown, { duration = ms, ease = EASE })
	props.scale = ui.tween(shown, { from = 0.96, to = 1, duration = ms, ease = EASE })
	return Menu.panel(props)
end

type PopupProps = {
	open: any,
	anchor: Node,
	label: any,
	title: any?,
	width: number?,
	placement: any?,
	items: () -> { Entry },
}

-- The menu stays up CLOSE_MS after it is told to close, fading out, as the
-- mockup's does; it takes no pointer while it leaves.
M.Popup = ui.component(function(props: PopupProps): Node
	local open = props.open
	local held = ui.use_state(false)
	-- (Fading out: told to close, the overlay still up.)
	local leaving = false
	Menu.hold_while(open)
	ui.effect(function()
		if open:get() then
			held:set(true)
			return
		end
		local ms = motion.ms(CLOSE_MS)
		if ms <= 0 then
			held:set(false)
			return
		end
		leaving = held:peek()
		ui.after(ms, function()
			leaving = false
			if not open:peek() then
				held:set(false)
			end
		end)
	end)
	-- Closed by the overlay itself (a click outside, Tab): nothing to fade.
	ui.effect(function()
		if not held:get() and open:peek() then
			open:set(false)
		elseif held:get() and not open:peek() and not leaving then
			-- (Opened by the overlay itself: a press on its anchor after Escape closed it.)
			open:set(true)
		end
	end)
	return ui.menu(panel {
		open = held,
		shown = open,
		pointer_events = function(): string
			return if open:get() then "auto" else "none"
		end,
		anchor = props.anchor,
		label = props.label,
		placement = props.placement or { side = "bottom", align = "start", offset = 4 },
		autofocus = false,
		w = ui.raw(props.width or 228, "the mockup's menu"),
		build = function(): Node
			return list(props.items(), props.title, function()
				open:set(false)
			end)
		end,
	})
end)

M.Context = ui.component(function(props: { items: () -> { Entry }, label: any, focus: any?, open: any?, [any]: any }): Node
	local open = props.open or ui.use_state(false)
	local at = ui.use_state({ x = 0, y = 0 })
	local box: { [any]: any } = {}
	for k, v in props do
		if k ~= "items" and k ~= "label" and k ~= "children" and k ~= "focus" and k ~= "open" then
			box[k] = v
		end
	end
	box.relative = true
	box.context_menu = function(e: any): { Node }?
		local el = props.focus and props.focus:get()
		if el then
			el:focus()
		end
		ui.batch(function()
			open:set(false)
			at:set({ x = e.local_x, y = e.local_y })
		end)
		ui.after(0, function()
			open:set(true)
		end)
		return nil
	end
	table.insert(box, ui.div {
		absolute = true,
		left = function(): number
			return at:get().x
		end,
		top = function(): number
			return at:get().y
		end,
		w = 1,
		h = 1,
		pointer_events = "none",
		M.Popup {
			open = open,
			anchor = ui.div { w = 1, h = 1 },
			label = props.label,
			placement = { side = "right", align = "start", offset = 0 },
			items = props.items,
		},
	})
	return ui.div(box)
end)

return M
