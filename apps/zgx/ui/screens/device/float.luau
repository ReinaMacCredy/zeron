-- gpui's floating device preview (crates/ui/src/device_panel/float.rs, shell/device_tabs.rs): one per chat, over its chat.

local ds = require("@ds")
local api = require("@app/screens/device/api")
local model = require("@app/screens/device/model")
local Screen = require("@app/screens/device/screen")
local Icon = require("@ds/icon")
local IconButton = require("@ds/icon_button").IconButton

local M = {}

M.EDGE_GAP = 12
M.DEFAULT_BOX = 320
M.MIN_WIDTH = 240
M.MIN_HEIGHT = 150
local HANDLE_INSET = 8

export type Frame = { x: number, y: number, width: number, height: number }
export type Size = { width: number, height: number }
export type Target = { host: string, id: string }

M.floats = ui.state("device_floats", {} :: { [string]: Target })
M.frame = ui.persist("device_float_frame", nil :: Frame?)

M.showing = nil :: ((chat: string) -> Target?)?
M.open_in_panel = nil :: ((chat: string, host: string, id: string) -> ())?

local function round(v: number): number
	return math.floor(v + 0.5)
end

function M.source_size(platform: string?, screen: any): Size
	if screen and (screen.width or 0) > 0 and (screen.height or 0) > 0 then
		local long = math.max(screen.width, screen.height)
		local short = math.min(screen.width, screen.height)
		if screen.orientation == "landscape_left" or screen.orientation == "landscape_right" then
			return { width = long, height = short }
		end
		return { width = short, height = long }
	end
	local aspect = if platform == "android" then 9 / 20 else 9 / 19.5
	return { width = 1000, height = 1000 / aspect }
end

function M.fit_width(desired: number, source: Size, max: Size): Size
	local aspect = source.width / source.height
	local width = math.min(math.max(desired, M.MIN_WIDTH, M.MIN_HEIGHT * aspect), math.max(max.width, 1), math.max(max.height * aspect, 1))
	return { width = round(width), height = round(width / aspect) }
end

local function clamp_origin(x: number, y: number, container: Size, player: Size): (number, number)
	local gap = M.EDGE_GAP
	return math.min(math.max(x, gap), math.max(container.width - player.width - gap, gap)),
		math.min(math.max(y, gap), math.max(container.height - player.height - gap, gap))
end

function M.resolve_frame(stored: Frame?, source: Size, container: Size): Frame
	local gap = M.EDGE_GAP
	local desired = if stored then stored.width else math.min(M.DEFAULT_BOX, M.DEFAULT_BOX * source.width / source.height)
	local player = M.fit_width(desired, source, { width = container.width - gap * 2, height = container.height - gap * 2 })
	local ax = if stored then stored.x else container.width - gap - player.width
	local ay = if stored then stored.y else gap
	local x, y = clamp_origin(ax, ay, container, player)
	return { x = x, y = y, width = player.width, height = player.height }
end

function M.moved(start: Frame, dx: number, dy: number, container: Size): Frame
	local player = { width = start.width, height = start.height }
	local x, y = clamp_origin(start.x + dx, start.y + dy, container, player)
	return { x = x, y = y, width = start.width, height = start.height }
end

function M.resized(start: Frame, edge: string, dx: number, dy: number, source: Size, container: Size): Frame
	local gap = M.EDGE_GAP
	local east = edge == "right" or edge == "top-right" or edge == "bottom-right"
	local west = edge == "left" or edge == "top-left" or edge == "bottom-left"
	local north = edge == "top" or edge == "top-left" or edge == "top-right"
	local south = edge == "bottom" or edge == "bottom-left" or edge == "bottom-right"
	local right = start.x + start.width
	local bottom = start.y + start.height
	local max = {
		width = if west then right - gap elseif east then container.width - gap - start.x else container.width - gap * 2,
		height = if north then bottom - gap elseif south then container.height - gap - start.y else container.height - gap * 2,
	}
	local width = start.width + (if east then dx elseif west then -dx else 0)
	local height = start.height + (if south then dy elseif north then -dy else 0)
	local aspect = source.width / source.height
	local desired
	if (east or west) and (north or south) then
		desired = (width + height / aspect) / (1 + 1 / (aspect * aspect))
	elseif east or west then
		desired = width
	else
		desired = height * aspect
	end
	local player = M.fit_width(desired, source, max)
	local x, y = clamp_origin(if west then right - player.width else start.x, if north then bottom - player.height else start.y, container, player)
	return { x = x, y = y, width = player.width, height = player.height }
end

function M.open(chat: string, host: string, id: string)
	local now = M.floats:peek()[chat]
	if now and now.host == host and now.id == id then
		return
	end
	M.floats:set_key(chat, { host = host, id = id })
end

function M.close(chat: string)
	if M.floats:peek()[chat] then
		M.floats:set_key(chat, nil)
	end
end

function M.of(chat: string): Target?
	return M.floats:get()[chat]
end

-- A tab's More → Float device over chat: floating closes the pane, the tab stays in the strip.
function M.float_tab(chat: string, host: string, id: string)
	M.open(chat, host, id)
	local ok, view = pcall(function(): any
		return (require :: any)("@app/screens/shell/view")
	end)
	if ok and view and view.set_files then
		view.set_files(false)
	end
end

ui.effect(function()
	local hub = api.hub:get()
	local all = M.floats:get()
	if type(hub) ~= "table" or hub.outdated or hub.sessions == nil then
		return
	end
	local gone = {}
	for chat, t in all do
		if not model.session(hub, chat, t.host, t.id) then
			table.insert(gone, chat)
		end
	end
	if #gone > 0 then
		ui.after(0, function()
			for _, chat in gone do
				M.close(chat)
			end
		end)
	end
end)

local EDGES = { "top", "bottom", "left", "right", "top-left", "top-right", "bottom-left", "bottom-right" }
local EDGE_CURSOR = {
	top = "ns_resize",
	bottom = "ns_resize",
	left = "ew_resize",
	right = "ew_resize",
	["top-left"] = "nwse_resize",
	["bottom-right"] = "nwse_resize",
	["top-right"] = "nesw_resize",
	["bottom-left"] = "nesw_resize",
}

local function edge_box(edge: string): { [string]: any }
	local b: { [string]: any } = { absolute = true }
	if edge == "top" or edge == "bottom" then
		b.left, b.right, b.h = 0, 0, 8
		b[edge] = -4
	elseif edge == "left" or edge == "right" then
		b.top, b.bottom, b.w = 0, 0, 8
		b[edge] = -4
	else
		local v, h = string.match(edge, "^(%a+)-(%a+)$")
		b.w, b.h = 16, 16
		b[v :: string] = -8
		b[h :: string] = -8
	end
	return b
end

local function card(chat: string, target: Target, container: any): Node
	local key = `float:{chat}`
	local live = ui.use_state(nil :: Frame?)
	local hovered = ui.use_state(false)
	local gesture: { px: number, py: number, start: Frame, edge: string? }? = nil
	local grabbing = ui.use_state(false)
	local function source(): Size
		local hub = api.hub:get() or {}
		local d = model.device(hub, target.host, target.id)
		return M.source_size(if d then d.platform else nil, (hub.screens or {})[model.key(target.host, target.id)])
	end
	local function room(): Size?
		local w, h = container.width, container.height
		if not w or not h or w <= 0 or h <= 0 then
			return nil
		end
		return { width = w, height = h }
	end
	local frame = ui.computed(function(): Frame?
		local c = room()
		if not c then
			return nil
		end
		return M.resolve_frame(live:get() or M.frame:get(), source(), c)
	end)
	local function begin(edge: string?, e: any)
		local start = frame:peek()
		if not start then
			return
		end
		live:set(M.frame:peek())
		gesture = { px = e.x, py = e.y, start = start, edge = edge }
		grabbing:set(true)
	end
	local function track(e: any)
		local g = gesture
		local c = room()
		if not g or not c then
			return
		end
		local dx, dy = e.x - g.px, e.y - g.py
		local next: Frame
		if g.edge == nil then
			local at = M.moved(g.start, dx, dy, c)
			local kept = live:peek() or at
			next = { x = at.x, y = at.y, width = kept.width, height = kept.height }
		else
			next = M.resized(g.start, g.edge, dx, dy, source(), c)
		end
		local was = live:peek()
		if not was or was.x ~= next.x or was.y ~= next.y or was.width ~= next.width or was.height ~= next.height then
			live:set(next)
		end
	end
	local function finish()
		if gesture == nil then
			return
		end
		gesture = nil
		grabbing:set(false)
		local f = live:peek()
		live:set(nil)
		if f then
			M.frame:set(f)
		end
	end
	local function gestures(props: { [string]: any }, edge: string?): { [string]: any }
		props.on_press = function(e: any)
			begin(edge, e)
		end
		props.on_pointer_move = function(e: any)
			track(e)
		end
		props.on_release = function()
			finish()
		end
		return props
	end
	local screen = Screen.make {
		key = key,
		host = target.host,
		id = target.id,
		node_id = "device-float-screen",
		rounded = "$radius.xl",
		on_error = function() end,
	}
	local function shown(): number
		return if hovered:get() or grabbing:get() then 1 else 0
	end
	local function button(id: string, label: Message, run: () -> (), glyph: Node): Node
		return ui.div {
			opacity = shown,
			IconButton {
				id = id,
				label = label,
				tone = "quiet",
				w = 24,
				h = 24,
				on_click = run,
				glyph,
			},
		}
	end
	local handle = ui.row(gestures({
		id = "device-float-handle",
		absolute = true,
		top = HANDLE_INSET,
		right = HANDLE_INSET,
		h = 32,
		items = "center",
		gap = ui.raw(2, "gpui's handle pill"),
		p = ui.raw(2, "gpui's handle pill"),
		rounded = "$radius.lg",
		border = 1,
		border_color = function(): string
			return if shown() == 1 then "$border" else "#00000000"
		end,
		bg = function(): string
			return if shown() == 1 then "$surface.raised" else "#00000000"
		end,
		shadow = function(): any
			return if shown() == 1 then "$shadow.lg" else false
		end,
		cursor = function(): string
			return if grabbing:get() then "grabbing" else "grab"
		end,
		on_hover = function(inside: boolean)
			hovered:set(inside)
		end,
		button("device-float-open", ui.t("device.float.open"), function()
			if M.open_in_panel then
				M.open_in_panel(chat, target.host, target.id)
			end
		end, ds.SidebarIcon { open = ui.use_state(true), side = "right", size = 14 }),
		button("device-float-close", ui.t("device.float.close"), function()
			M.close(chat)
		end, Icon { name = "close", size = 14 }),
		ui.row {
			id = "device-float-grip",
			self = "start",
			w = 12,
			h = 12,
			justify = "end",
			ui.div {
				w = 8,
				h = 8,
				rounded = "$radius.full",
				bg = ui.raw("#ffffff40", "gpui's grip dot, white at 25% over the device's picture"),
				opacity = function(): number
					return 1 - shown()
				end,
			},
		},
	}, nil))
	local children: { Node } = {
		ui.column {
			w_full = true,
			h_full = true,
			rounded = "$radius.xl",
			overflow = "hidden",
			items = "center",
			justify = "center",
			screen.node,
		},
		ui.div {
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			bottom = 0,
			rounded = "$radius.xl",
			border = 1,
			border_color = "$border",
			pointer_events = "none",
		},
	}
	for _, edge in EDGES do
		local b = edge_box(edge)
		b.id = `device-float-resize-{edge}`
		b.cursor = EDGE_CURSOR[edge]
		table.insert(children, ui.div(gestures(b, edge)))
	end
	table.insert(children, handle)
	local props: { [any]: any } = {
		id = "device-float",
		role = "group",
		label = ui.t("device.float.label"),
		absolute = true,
		left = function(): number
			local f = frame:get()
			return if f then f.x else 0
		end,
		top = function(): number
			local f = frame:get()
			return if f then f.y else 0
		end,
		w = function(): number
			local f = frame:get()
			return if f then f.width else 0
		end,
		h = function(): number
			local f = frame:get()
			return if f then f.height else 0
		end,
		hidden = function(): boolean
			return frame:get() == nil
		end,
		pointer_events = "auto",
		rounded = "$radius.xl",
		bg = ui.raw("#000000e6", "gpui's float card, black at 90% behind the device's picture in either theme"),
		shadow = "$shadow.lg",
	}
	for _, c in children do
		table.insert(props, c)
	end
	return ui.column(props)
end

function M.View(props: { chat: () -> string?, shown: () -> boolean }): Node
	local container = ui.ref()
	local function visible(): (string?, Target?)
		local chat = props.chat()
		if not chat or not props.shown() then
			return nil, nil
		end
		local t = M.of(chat)
		if not t then
			return nil, nil
		end
		local Builtins = require("@app/screens/plugins/builtins")
		if not Builtins.on("device") then
			return nil, nil
		end
		local showing = if M.showing then M.showing(chat) else nil
		if showing and showing.host == t.host and showing.id == t.id then
			return nil, nil
		end
		return chat, t
	end
	return ui.div {
		id = "device-float-area",
		ref = container,
		absolute = true,
		left = 0,
		right = 0,
		top = 0,
		bottom = 0,
		overflow = "hidden",
		pointer_events = "none",
		ui.presence(function(): string
			local chat, t = visible()
			return if chat and t then `{chat}\0{model.key(t.host, t.id)}` else ""
		end, function(which: string): Node
			if which == "" then
				return ui.div {}
			end
			local chat = string.match(which, "^(.-)%z") :: string
			local t = M.floats:peek()[chat]
			return card(chat, t, container)
		end, { w_full = true, h_full = true, pointer_events = "none" }),
	}
end

return M
