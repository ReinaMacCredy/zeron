-- gpui's `DeviceStreamView`, shared by the Device tab and the floating preview as gpui shares it.

local api = require("@app/screens/device/api")
local model = require("@app/screens/device/model")

local M = {}

local s = model.s

function M.words(err: any): string
	-- (A host action's rejection is a UiError, userdata: its words are its message.)
	local w = if type(err) == "table" or type(err) == "userdata" then err.message else err
	if w == "outdated" then
		return s("unavailable.engine")
	elseif w == "offline" then
		return ui.t("settings.engine.offline"):get()
	end
	return if type(w) == "string" and w ~= "" then w else ui.t("common.something_went_wrong"):get()
end

function M.call(method: string, params: { [string]: any }, ok: ((any) -> ())?, bad: ((string) -> ())?)
	local fine, p = pcall(api.call, method, params)
	if not fine or p == nil then
		if bad then
			bad(s("unavailable.engine"))
		end
		return
	end
	p:after(function(v: any)
		if ok then
			ok(v)
		end
	end, function(err: any)
		if bad then
			bad(M.words(err))
		end
	end)
end

function M.target_params(host: string, id: string): { [string]: any }
	local p: { [string]: any } = { deviceId = id }
	if host ~= model.LOCAL then
		p.hostId = host
	end
	return p
end

-- HID usages iOS takes for a key, and Android's own keycodes for the few
-- that aren't text (crates/proto device_hub `DeviceInput`).
local HID: { [string]: number } = { Enter = 40, Escape = 41, Backspace = 42, Tab = 43, [" "] = 44, ArrowRight = 79, ArrowLeft = 80, ArrowDown = 81, ArrowUp = 82 }
local ANDROID: { [string]: number } = { Enter = 66, Backspace = 67, Tab = 61, Escape = 111, ArrowUp = 19, ArrowDown = 20, ArrowLeft = 21, ArrowRight = 22 }
local function hid(key: string): (number?, boolean)
	if #key == 1 then
		local b = string.byte(key)
		if b >= 97 and b <= 122 then
			return b - 97 + 4, false
		elseif b >= 65 and b <= 90 then
			return b - 65 + 4, true
		elseif b >= 49 and b <= 57 then
			return b - 49 + 30, false
		elseif key == "0" then
			return 39, false
		end
	end
	return HID[key], false
end

-- What a key down on the screen sends: the inputs, in order.
function M.key_inputs(platform: string, key: string): { any }
	if platform == "android" then
		local code = ANDROID[key]
		if code then
			return { { keycode = { keycode = code } } }
		end
		if #key == 1 or utf8.len(key) == 1 then
			return { { text = { text = key } } }
		end
		return {}
	end
	local usage, shift = hid(key)
	if not usage then
		return {}
	end
	local out = {}
	if shift then
		table.insert(out, { key = { phase = "down", usage = 225 } })
	end
	table.insert(out, { key = { phase = "down", usage = usage } })
	table.insert(out, { key = { phase = "up", usage = usage } })
	if shift then
		table.insert(out, { key = { phase = "up", usage = 225 } })
	end
	return out
end

function M.sideways(orientation: string): boolean
	return orientation == "landscape_left" or orientation == "landscape_right"
end

-- A point on the turned picture, on the device's own screen (gpui's `raw_point`).
function M.raw_point(orientation: string, x: number, y: number): (number, number)
	if orientation == "landscape_left" then
		return y, 1 - x
	elseif orientation == "landscape_right" then
		return 1 - y, x
	elseif orientation == "portrait_upside_down" then
		return 1 - x, 1 - y
	end
	return x, y
end

local function is_move(input: any): boolean
	return type(input.touch) == "table" and input.touch.phase == "move"
end

local BLACK = ui.raw("#000000", "a device's screen is black behind its picture, in either theme")

local function small(text: any): Node
	return ui.text { text, wrap = true, text_size = "$font.size.sm", color = "$text.secondary" }
end

export type Screen = { node: Node, send: (input: any) -> (), device: () -> model.Device?, screen_config: () -> any }

function M.make(props: { key: string, host: string, id: string, visible: (() -> boolean)?, on_error: (string) -> (), node_id: string?, rounded: any?, area: State<{ w: number, h: number }>?, three_d: (() -> boolean)? }): Screen
	local key, host, id = props.key, props.host, props.id
	local function hub(): any
		return api.hub:get() or {}
	end
	local function frame(): any
		return api.frames:get()[key]
	end
	local function device(): model.Device?
		return model.device(hub(), host, id)
	end
	local function screen_config(): any
		return (hub().screens or {})[model.key(host, id)]
	end
	-- How the picture is turned (gpui's `display_rotation`): a screen whose
	-- size the device already gives on its side is shown as it comes.
	local rotation = ui.computed(function(): string
		local sc = screen_config()
		if sc == nil or (sc.width or 0) > (sc.height or 0) then
			return "portrait"
		end
		return sc.orientation or "portrait"
	end)
	-- (Its frames only while it shows: hidden, nothing streams or decodes.)
	local visible = props.visible or function(): boolean
		return true
	end
	-- (The stream follows the platform alone: re-asked on every hub push, its own subscription's push asked it again.)
	local platform = ui.computed(function(): string?
		local d = device()
		return if d then d.platform else nil
	end)
	ui.effect(function()
		if visible() then
			api.stream(key, host, id, platform:get(), rotation:peek())
		else
			api.unstream(key)
		end
	end)
	-- One input at a time, in order, the moves queued behind one merged into
	-- the newest (gpui's `send_inputs`): the engine answers each call apart.
	local queue: { any } = {}
	local sending = false
	local function pump()
		if sending then
			return
		end
		local input = table.remove(queue, 1)
		if input == nil then
			return
		end
		sending = true
		local p = M.target_params(host, id)
		p.input = input
		M.call("SendHubDeviceInput", p, function()
			sending = false
			pump()
		end, function(why: string)
			sending = false
			props.on_error(why)
			pump()
		end)
	end
	ui.on_cleanup(function()
		table.clear(queue)
		api.unstream(key)
	end)
	local function send(input: any)
		local last = queue[#queue]
		if last and is_move(last) and is_move(input) then
			queue[#queue] = input
		else
			table.insert(queue, input)
		end
		pump()
	end
	ui.effect(function()
		api.orient(key, rotation:get())
	end)
	local three_d = props.three_d or function(): boolean
		return false
	end
	local function send_all(touches: { any })
		for _, t in touches do
			send({ touch = t })
		end
	end
	local name = ui.computed(function(): string?
		local d = device()
		return if d then d.name else nil
	end)
	ui.effect(function()
		api.three_d(key, three_d(), { platform = platform:get(), name = name:get(), host = host, id = id })
	end)
	ui.effect(function()
		if three_d() then
			send_all(api.screen3d(key, screen_config()))
		end
	end)
	ui.on_cleanup(function()
		api.three_d(key, false)
	end)
	local screen = ui.ref()
	local down = false
	local function point(e: any): (number, number)
		local w = screen.width or 1
		local h = screen.height or 1
		local x = math.clamp((e.local_x or 0) / math.max(w, 1), 0, 1)
		local y = math.clamp((e.local_y or 0) / math.max(h, 1), 0, 1)
		return M.raw_point(rotation:peek(), x, y)
	end
	local function aspect(): number
		local f = frame()
		local sc = screen_config()
		local w = (sc and sc.width) or 9
		local h = (sc and sc.height) or 19.5
		if M.sideways(rotation:get()) then
			w, h = h, w
		end
		if f and f.w and f.h then
			w, h = f.w, f.h
		end
		return w / math.max(h, 1)
	end
	-- The biggest box of the picture's shape inside `area` (gpui's `fit_device_frame`): a box
	-- clamped on one side alone would letterbox the picture and put touches off it.
	local function fit(): (number, number)
		local a = (props.area :: any):get()
		if three_d() then
			return a.w, a.h
		end
		local r = aspect()
		if a.h * r <= a.w then
			return a.h * r, a.h
		end
		return a.w, a.w / r
	end
	local node = ui.div {
		id = props.node_id or "device-screen",
		role = "img",
		label = function(): string
			local d = device()
			return if d then `{s("stream.screen")}: {d.name}` else s("stream.screen")
		end,
		ref = screen,
		focusable = true,
		max_w = "100%",
		max_h = "100%",
		h_full = props.area == nil,
		aspect = if props.area then nil else aspect,
		w = if props.area
			then function(): number
				local w = fit()
				return w
			end
			else nil,
		h = if props.area
			then function(): number
				local _, h = fit()
				return h
			end
			else nil,
		rounded = function(): any
			return if three_d() then 0 else props.rounded or "$radius.lg"
		end,
		overflow = "hidden",
		bg = function(): any
			return if three_d() then "transparent" else BLACK
		end,
		cursor = "pointer",
		on_press = function(e: any)
			down = true
			if three_d() then
				send_all(api.pointer3d(key, "begin", e.local_x or 0, e.local_y or 0, e.alt == true))
				return
			end
			local x, y = point(e)
			send({ touch = { phase = "begin", x = x, y = y } })
		end,
		on_pointer_move = function(e: any)
			if down then
				if three_d() then
					send_all(api.pointer3d(key, "move", e.local_x or 0, e.local_y or 0))
					return
				end
				local x, y = point(e)
				send({ touch = { phase = "move", x = x, y = y } })
			end
		end,
		on_release = function(e: any)
			if down then
				down = false
				if three_d() then
					send_all(api.pointer3d(key, "end", e.local_x or 0, e.local_y or 0))
					return
				end
				local x, y = point(e)
				send({ touch = { phase = "end", x = x, y = y } })
			end
		end,
		on_wheel = function(e: any): boolean
			return three_d() and api.wheel3d(key, { dx = e.dx, dy = e.dy, x = e.local_x, y = e.local_y, ctrl = e.ctrl })
		end,
		on_pinch = function(e: any): boolean
			return three_d() and api.pinch3d(key, e.scale, e.local_x or 0, e.local_y or 0)
		end,
		on_key = function(e: any): boolean
			if e.cmd or e.ctrl then
				return false
			end
			local d = device()
			local inputs = M.key_inputs(if d then d.platform else "ios", e.key)
			for _, input in inputs do
				send(input)
			end
			return #inputs > 0
		end,
		ui.presence(function(): string
			local f = frame()
			if not (f and f.seq) then
				return "none"
			end
			return if three_d() then "3d" else "frame"
		end, function(which: string): Node
			if which == "3d" then
				return ui.gpu_view { id = "device-3d", view = api.view3d(key), label = s("stream.screen"), w_full = true, h_full = true }
			end
			if which == "none" then
				return ui.column {
					w_full = true,
					h_full = true,
					items = "center",
					justify = "center",
					small(ui.t("device.stream.connecting")),
				}
			end
			return ui.image {
				src = function(): string
					return api.frame_src(key)
				end,
				w_full = true,
				h_full = true,
				fit = "contain",
			}
		end, { w_full = true, h_full = true }),
	}
	return { node = node, send = send, device = device, screen_config = screen_config }
end

return M
