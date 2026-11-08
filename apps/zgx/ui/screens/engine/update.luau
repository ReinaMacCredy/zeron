-- An engine older than the one on disk (src/engine_update.rs says, as
-- shared state `engine.update`: `{ stale, ours }`): one this app started is
-- started again on its own while nothing runs in it; while something does,
-- the app rail's Update entry offers it ("Restart to update"). One this app
-- didn't start is never touched: the entry says so, and how to update it.
--
--   Update.Watch()      what starts an idle engine of this app's again:
--                       mounted once, always (the shell's root), nothing
--                       drawn
--   Update.which()      what there is to say: "ours" or "theirs", the rail's
--                       (screens/shell/rail), "old", Notice's, or ""
--   Update.restart()    the rail's press on "ours"
--   Update.Notice()     "old"'s chip, above the sidebar's footer (nothing, and
--                       no room, when there's nothing to say)
--
-- Busy: a chat working or waiting on an answer, or a terminal open in a
-- panel (starting the engine again ends its shells).
--
-- The beta also asks whether the engine serves this interface at all
-- (`engine.interface`: `{ supported }`). It runs beside the Zeron app, whose
-- engine it never restarts: one that can't serve this needs the Zeron app
-- updated, and the chip says so.

local sessions = require("@account/sessions")
local Icon = require("@ds/icon")
local FadeText = require("@ds/fade_text")
local locale = require("@ds/locale")

local M = {}

local update = ui.shared("engine.update", { stale = false, ours = false })
local interface = ui.shared("engine.interface", { supported = true })

-- An engine that doesn't serve this interface (the beta's check).
local function unsupported(): boolean
	local i = interface:get()
	return i ~= nil and i.supported == false
end
-- A restart asked of this stale engine already (once each: the engine that
-- comes back is a new one, and says again).
local asked = ui.state("engine_update_asked", false)
-- (Its module loads the first time it's asked for, from a timer: nothing more at launch. The flag
-- starts over with the script, as the module it says is loaded does: a build never loads it.)
M.prompt_loaded = ui.state("update_prompt_loaded", false)
M.prompt_loaded:set(false)
local prompt_module: any = nil
function M.show_prompt(prompt: any)
	prompt_module = prompt
	M.prompt_loaded:set(true)
end

-- Whether anything runs in the engine now.
local function busy(): boolean
	local now = sessions.now:get()
	for _, c in now.chats or {} do
		if c.status == "working" or c.status == "input" then
			return true
		end
	end
	local ok, panel = pcall(function(): any
		return (require :: any)("@app/screens/panel/state")
	end)
	if ok and panel and panel.sessions then
		for _, s in panel.sessions:get() do
			for _, t in s.tabs do
				if t.kind == "terminal" then
					return true
				end
			end
		end
	end
	return false
end

local function stale(): boolean
	local u = update:get()
	return (u ~= nil and u.stale == true) or unsupported()
end

local function ours(): boolean
	local u = update:get()
	return u ~= nil and u.ours == true
end

-- The host's restart (a test stands in for it).
function M.call_restart(): any
	return ui.call("engine.restart", {})
end

local function restart()
	asked:set(true)
	M.call_restart():after(function() end, function(err: any)
		print(`zeron-zgx: the engine didn't restart: {err}`)
		asked:set(false)
	end)
end

M.Watch = ui.component(function(): Node
	-- Idle and this app's: started again at once, nothing shown.
	ui.effect(function()
		if not stale() then
			if asked:peek() then
				asked:set(false)
			end
			return
		end
		if ours() and not busy() and not asked:peek() then
			restart()
		end
	end)
	return ui.div {
		w = 0,
		h = 0,
		absolute = true,
		ui.presence(function(): string?
			return if M.prompt_loaded:get() and prompt_module then "prompt" else nil
		end, function(): Node
			return prompt_module.Prompt()
		end),
	}
end)

-- What there is to say, if anything: one this app may restart, or one it may not.
function M.which(): string
	-- Not one for this interface.
	if unsupported() then
		return "old"
	end
	if not stale() or asked:get() then
		return ""
	end
	if not ours() then
		return "theirs"
	end
	return if busy() then "ours" else ""
end
M.restart = restart

-- An engine too old for this interface, once restarting didn't bring a newer
-- one: the way back to Stable is About & updates.
M.Notice = ui.component(function(): Node
	return ui.show(function(): boolean
		return M.which() == "old"
	end, ui.row {
		id = "engine-update",
		role = "button",
		label = ui.t("engine.update.old"),
		tooltip = locale.s("engine.update.old_tip"),
		w_full = true,
		h = ui.raw(30, "a sidebar row's height (ds/sidebar's SLOT)"),
		shrink = 0,
		px = "$space.2",
		gap = "$zeron.space.tight",
		items = "center",
		rounded = "$radius.lg",
		bg = "$zeron.tone.neutral.wash",
		hover = { bg = "$zeron.tone.neutral.edge" },
		color = "$zeron.tone.neutral.ink",
		cursor = "pointer",
		on_click = function()
			local ok, route = pcall(function(): any
				return (require :: any)("@app/screens/shell/route")
			end)
			if ok and route then
				(route :: any).settings("about")
			end
		end,
		ui.div { shrink = 0, y = 0.5, Icon { name = "info-circle", size = 12 } },
		FadeText {
			ui.t("engine.update.short_old"),
			grow = 1,
			min_w = 0,
			text_size = "$font.size.xs",
			weight = "$font.weight.medium",
		},
	})
end)

return M
