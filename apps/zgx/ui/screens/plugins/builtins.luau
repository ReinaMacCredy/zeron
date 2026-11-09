--!strict
-- Zeron's own plugins (DECISIONS 2026-11-08): each door reads `M.on(id)`, so a switch in Settings → Plugins takes it away or brings it back at once.
-- One key each: `builtin_<id>` (ui.persist), but Keep awake's is the engine's own `keepAwakeWhileWorking` in its ui-settings.json, which it reads as it holds the Mac awake.

local M = {}

export type Builtin = { id: string, icon: string }

M.LIST = {
	{ id = "review", icon = "checklist" },
	{ id = "usage", icon = "usage-chart" },
	{ id = "home_widgets", icon = "widget" },
	{ id = "keep_awake", icon = "sun" },
	{ id = "appshots", icon = "gallery" },
	{ id = "predictions", icon = "magic-stick" },
	{ id = "browser_annotate", icon = "cursor" },
	{ id = "device", icon = "smartphone" },
	{ id = "voice", icon = "microphone" },
} :: { Builtin }

-- Commands a built-in owns: off, they have no key, do nothing, and the palette and Shortcuts leave them out.
M.COMMANDS = {
	["panel.annotate"] = "browser_annotate",
} :: { [string]: string }

local KEEP_AWAKE = "keep_awake"

local stores: { [string]: State<boolean> } = {}
for _, b in M.LIST do
	if b.id ~= KEEP_AWAKE then
		stores[b.id] = ui.persist(`builtin_{b.id}`, true)
	end
end
-- (nil until the Plugins page asks: on, as the engine takes a file without it.)
local keep_awake = ui.state("builtins_keep_awake", nil :: boolean?)

function M.on(id: string): boolean
	if id == KEEP_AWAKE then
		return keep_awake:get() ~= false
	end
	local s = stores[id]
	return s == nil or s:get() ~= false
end

function M.command_on(id: string): boolean
	local owner = M.COMMANDS[id]
	return owner == nil or M.on(owner)
end

local function host(): any
	return require("@app/screens/settings/host")
end

function M.load()
	local ok, p = pcall(host().call, "keep_awake.get")
	if ok and p ~= nil then
		p:after(function(on: any)
			if type(on) == "boolean" then
				keep_awake:set(on)
			end
		end, function(_: any) end)
	end
end

-- `failed` hears a change the host refused (Keep awake's), already put back.
function M.set(id: string, on: boolean, failed: ((err: any) -> ())?)
	if id ~= KEEP_AWAKE then
		local s = stores[id]
		if s then
			s:set(on)
		end
		return
	end
	local was = keep_awake:peek()
	keep_awake:set(on)
	local function refused(err: any)
		keep_awake:set(was)
		if failed then
			failed(err)
		end
	end
	local ok, p = pcall(host().call, "keep_awake.set", { on = on })
	if not ok or p == nil then
		refused({ message = "failed" })
		return
	end
	p:after(function(_: any) end, refused)
end

-- (Voice's door is the design system's composer's mic, a lane's too: ds can't require this module.)
local voice = ui.shared("builtins.voice", true)
ui.effect(function()
	local on = M.on("voice")
	if voice:peek() ~= on then
		voice:set(on)
	end
end)

-- Off, the host watches no chat for a prediction, so the engine makes none; the host starts on.
local told_predictions = true
ui.effect(function()
	local on = M.on("predictions")
	if on ~= told_predictions then
		told_predictions = on
		pcall(ui.invoke, "chat.predictions", on)
	end
end)

return M
