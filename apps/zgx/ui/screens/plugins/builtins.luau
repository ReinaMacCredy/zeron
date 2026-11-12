--!strict
-- Zeron's own features (DECISIONS 2026-11-08): each door reads `M.on(id)`, so a switch (Review's in Settings → Plugins, the rest on their own Settings pages) takes it away or brings it back at once.
-- One key each: `builtin_<id>` (ui.persist), but Keep awake's is the engine's own `keepAwakeWhileWorking` in its ui-settings.json, which it reads as it holds the Mac awake.

local M = {}

export type Builtin = { id: string, icon: string }

M.LIST = {
	{ id = "review", icon = "checklist" },
} :: { Builtin }

-- Switched on the Settings page that owns each (Reina, 2026-11-12), not on Plugins.
M.SETTINGS = { "usage", "home_widgets", "keep_awake", "browser_annotate", "device", "voice" }

-- Commands a built-in owns: off, they have no key, do nothing, and the palette and Shortcuts leave them out.
M.COMMANDS = {
	["panel.annotate"] = "browser_annotate",
} :: { [string]: string }

local KEEP_AWAKE = "keep_awake"

local stores: { [string]: State<boolean> } = {}
for _, b in M.LIST do
	stores[b.id] = ui.persist(`builtin_{b.id}`, true)
end
for _, id in M.SETTINGS do
	if id ~= KEEP_AWAKE then
		stores[id] = ui.persist(`builtin_{id}`, true)
	end
end
-- (nil until General asks: on, as the engine takes a file without it.)
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

-- A Settings row's switch for one of M.SETTINGS, labelled `label`.
function M.Switch(id: string, label: Message): Node
	return ui.component(function(): Node
		local on = ui.use_state(M.on(id))
		ui.effect(function()
			local want = M.on(id)
			if on:peek() ~= want then
				on:set(want)
			end
		end)
		return ui.div {
			id = `feature-{id}`,
			require("@ds").Switch {
				value = on,
				label = label,
				on_change = function(value: boolean)
					M.set(id, value, function(err: any)
						require("@ds").toast.show {
							title = ui.t("plugins.page.couldnt_switch", { name = label:get() }),
							body = if type(err) == "table" and err.message then tostring(err.message) else nil,
							tone = "warning",
						}
					end)
				end,
			},
		}
	end)()
end

-- One of M.SETTINGS as a card of its own, its words `settings.features.<id>`.
function M.Row(id: string): Node
	local ds = require("@ds")
	local label = ui.t(`settings.features.{id}.label`)
	return ds.Settings.Group {
		ds.Settings.Row {
			label = label,
			description = ui.t(`settings.features.{id}.description`),
			control = M.Switch(id, label),
		},
	}
end

-- (Voice's door is the design system's composer's mic, a lane's too: ds can't require this module.)
local voice = ui.shared("builtins.voice", true)
ui.effect(function()
	local on = M.on("voice")
	if voice:peek() ~= on then
		voice:set(on)
	end
end)

return M
