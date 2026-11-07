-- A composer's agent profiles (ds.Composer's `profiles`): the host's list
-- (src/agent_profiles.rs), the chat's profile and how it stands against it,
-- and applying, reverting, saving and editing them, as Zeron's picker does
-- (crates/ui/src/pickers/profiles.rs). The chat page's composer and a
-- pane's or side chat's each bind their own: a pick lands on that chat.
--
-- A chat's comparison rides its session (`session.profile`, worked out by
-- the host as the session is published: its config, the list and the
-- catalog's names). A new chat's is asked of the host (`profiles.drift`) as
-- its picks or the list change, never as its draft is typed.
--
-- Each host action is under `pcall`: a host older than this has none, and
-- reads as no profiles at all (`profiles.list` stays at its default, the
-- card says nothing more than Save current, which saves nothing; `absent`).

local store = require("@app/screens/chat/store")
local model_switch = require("@app/screens/chat/model_switch")
local route = require("@app/screens/shell/route")
local look = require("@ds/profile_look")

local M = {}

M.list = ui.shared("profiles.list", { state = "idle", profiles = {} } :: look.List)

-- The draft handed to Settings → Agent profiles by the card's More…,
-- opened there as it shows (screens/settings/agent_profiles).
M.pending = ui.state("profiles_pending_edit", nil :: { id: string?, draft: look.Draft }?)

-- A promise, or nil on a host without the action.
local function call(name: string, args: any): any
	local ok, p = pcall(ui.call, name, args)
	return if ok then p else nil
end
local function call_sync(name: string, args: any): any
	local ok, v = pcall(ui.call_sync, name, args)
	return if ok then v else nil
end

-- What's asked of the host, each by its action (a fixture stands in for
-- them as the engine would answer).
M.host = {
	-- (Raises on a host without it.)
	load = function()
		ui.invoke("profiles.load", nil)
	end,
	apply = function(args: any): any
		return call("chat.apply_profile", args)
	end,
	save = function(args: any): any
		return call("profiles.save", args)
	end,
	absorb = function(args: any): any
		return call("profiles.absorb", args)
	end,
	remove = function(args: any): any
		return call("profiles.remove", args)
	end,
	reorder = function(args: any): any
		return call("profiles.reorder", args)
	end,
	tools = function(args: any): any
		return call("profiles.tools", args)
	end,
	system_prompt = function(): any
		return call("profiles.system_prompt", nil)
	end,
	set_system_prompt = function(args: any): any
		return call("profiles.set_system_prompt", args)
	end,
	drift = function(args: any): any
		return call_sync("profiles.drift", args)
	end,
	current = function(args: any): any
		return call_sync("profiles.current", args)
	end,
}

-- The host has no profiles at all (older than this): Settings says so
-- rather than loading for good.
M.absent = ui.state("profiles_absent", false)

function M.load()
	if not pcall(M.host.load) and not M.absent:peek() then
		M.absent:set(true)
	end
end

-- Once, from the first composer: a chat's profile shows only once its list
-- is in.
local asked = ui.state("profiles_asked", false)
local function ensure()
	if not asked:peek() and M.list:peek().state == "idle" then
		asked:set(true)
		M.load()
	end
end

function M.find(id: string?): look.Profile?
	if not id then
		return nil
	end
	for _, p in M.list:get().profiles do
		if p.id == id then
			return p
		end
	end
	return nil
end

local function quietly(p: any)
	if p then
		p:after(function() end, function() end)
	end
end

export type Picks = { model: string?, effort: string?, access: string?, options: { [string]: string }?, fast: boolean? }
export type Opts = {
	-- The chat (nil: a new chat, not made yet), its session, and whether it
	-- has its agent (another agent's profile then goes through the switch).
	chat: () -> string?,
	session: () -> any?,
	committed: () -> boolean,
	-- The composer's picks, and setting them (a new chat's profile lands on
	-- them; a chat's lands on the chat).
	picks: () -> Picks,
	set_picks: (p: Picks) -> (),
	-- A new chat's profile (its id), sent with its first message.
	draft_profile: State<string?>,
	-- Whether the chat has anything to reread (model_switch.has_history),
	-- and its key for the switch dialog.
	history: () -> boolean?,
	key: string,
	-- The card again, after the switch dialog's No.
	reopen: () -> (),
	-- The keys back to the draft, after the switch dialog's Yes.
	refocus: () -> (),
	close: State<number>,
}

local function agents(): { any }
	local out = {}
	for _, h in store.catalog:get().harnesses do
		table.insert(out, { id = h.id, label = h.label, mark = h.mark, icon = h.icon, access_modes = h.access_modes })
	end
	return out
end
local function harness_of(value: string?): string?
	if not value then
		return nil
	end
	for _, m in store.catalog:peek().models do
		if m.value == value then
			return m.harness
		end
	end
	return string.match(value, "^([^/]+)/")
end

-- A profile's model as the catalog values it (the host's "<agent>/<id>").
local function value_of(harness: string, model: string): string
	for _, m in store.catalog:peek().models do
		if m.harness == harness and (m.value == model or m.value == `{harness}/{model}`) then
			return m.value
		end
	end
	return `{harness}/{model}`
end

-- For a composer: ds.Composer's `profiles`, and the dialog to mount.
function M.bind(opts: Opts): ({ [string]: any }, Node)
	ensure()
	local function args_of(p: Picks, profile: string?): { [string]: any }
		return { model = p.model, effort = p.effort, access = p.access, options = p.options, fast = p.fast, profile = profile }
	end
	local function harness(): string?
		local s = opts.session()
		if opts.chat() and s and s.harness then
			return harness_of(opts.picks().model) or s.harness
		end
		return harness_of(opts.picks().model)
	end
	-- How the chat stands against its profile (asked again only as the
	-- session, the picks or the list change).
	local function applied_now(): look.Applied?
		if opts.chat() then
			local s = opts.session()
			return if s then s.profile else nil
		end
		local id = opts.draft_profile:get()
		local p = M.find(id)
		if not p then
			return nil
		end
		local got = M.host.drift(args_of(opts.picks(), id))
		if type(got) == "table" then
			return got
		end
		return { id = p.id, name = p.name, harness = p.harness, mark = p.mark, icon = p.icon, edited = false, savable = false, drift = {} }
	end
	local applied_c = ui.computed(applied_now)
	local function applied(): look.Applied?
		return applied_c:get()
	end

	local pending: string? = nil
	local function land(id: string?, ask: model_switch.Ask?)
		local chat = opts.chat()
		if chat then
			quietly(M.host.apply({ id = chat, profile = id, note = if ask then ask.note else nil, note_hint = if ask then ask.note_hint else nil }))
			return
		end
		if not id then
			opts.draft_profile:set(nil)
			return
		end
		local p = M.find(id)
		if not p then
			return
		end
		-- (gpui's new chat: the profile's agent, its model or the agent's own,
		-- and what else it sets.)
		local now = opts.picks()
		local picks: Picks = { model = now.model, effort = now.effort, access = now.access, options = now.options, fast = now.fast }
		if p.model then
			picks.model = value_of(p.harness, p.model)
		elseif harness_of(now.model) ~= p.harness then
			for _, m in store.catalog:peek().models do
				if m.harness == p.harness then
					picks.model = m.value
					break
				end
			end
		end
		if p.model or harness_of(now.model) ~= p.harness then
			picks.options = {}
			picks.fast = false
		end
		if p.effort then
			picks.effort = p.effort
		end
		if p.access then
			picks.access = p.access
		end
		local options = p.options or {}
		if next(options) then
			local own: { [string]: string } = {}
			local fast = false
			for k, v in options do
				if v == "on" and (k == "fastMode" or k == "fast_mode") or (k == "fast" and v == "true") or ((k == "serviceTier" or k == "speed") and v == "fast") then
					fast = true
				elseif k ~= "fastMode" and k ~= "fast_mode" and k ~= "fast" and k ~= "serviceTier" and k ~= "speed" then
					own[k] = v
				end
			end
			picks.options = own
			picks.fast = fast
		end
		ui.batch(function()
			opts.set_picks(picks)
			opts.draft_profile:set(id)
		end)
	end
	local take, dialog = model_switch.use(`{opts.key}_profile`, {
		-- (A chat with no model still has its agent, "<agent>/": gpui asks
		-- whatever the model, pickers/profiles.rs `apply_profile`.)
		current = function(): string?
			local h = harness()
			return opts.picks().model or (if h then `{h}/` else nil)
		end,
		history = function(): boolean?
			if not opts.chat() or not opts.committed() then
				return nil
			end
			return opts.history()
		end,
		land = function(_value: string, ask: model_switch.Ask?)
			local id = pending
			pending = nil
			land(id, ask)
			-- (Once the dialog and its modal have gone.)
			ui.after(350, opts.refocus)
		end,
		back = opts.reopen,
	})
	local function apply(id: string?)
		local p = M.find(id)
		if p and opts.chat() and opts.committed() then
			-- Another agent, or another model, on a chat with history asks first.
			local now = opts.picks().model
			local moves = harness() ~= p.harness or (p.model ~= nil and now ~= value_of(p.harness, p.model))
			if moves then
				pending = id
				if take(if p.model then value_of(p.harness, p.model) else `{p.harness}/`) then
					return
				end
				pending = nil
			end
		end
		land(id, nil)
	end
	local function current_args(): { [string]: any }
		local chat = opts.chat()
		if chat then
			return { chat = chat }
		end
		return args_of(opts.picks(), opts.draft_profile:peek())
	end
	local function current(): look.Draft
		local got = M.host.current(current_args())
		if type(got) == "table" then
			-- (The host's answer is read-only: a draft of its own.)
			return {
				name = got.name or "",
				harness = got.harness,
				model = got.model,
				effort = got.effort,
				access = got.access,
				options = table.clone(got.options or {}),
				notes = got.notes or "",
				instructions = got.instructions or "",
				tools_off = table.clone(got.tools_off or {}),
			}
		end
		local p = opts.picks()
		local d = look.blank(harness() or "claude-code")
		d.model = if p.model then (string.match(p.model, "^[^/]+/(.+)$") or p.model) else nil
		d.effort = p.effort
		d.access = p.access
		d.options = table.clone(p.options or {})
		return d
	end
	return {
		list = function(): look.List
			return M.list:get()
		end,
		applied = applied,
		-- The agent the chat runs on, its mark on the chip's profile half
		-- (gpui's harness icon there, not the profile's).
		agent = function(): any?
			local h = harness()
			for _, a in store.catalog:get().harnesses do
				if a.id == h then
					return a
				end
			end
			return nil
		end,
		can_save = function(): boolean
			return harness() ~= "acp"
		end,
		switching = function(): string?
			return if opts.chat() and opts.committed() then harness() else nil
		end,
		agents = agents,
		models = function(): { any }
			return store.catalog:get().models
		end,
		apply = apply,
		save_drift = function(id: string)
			quietly(M.host.absorb({ id = id, current = current_args() }))
		end,
		current = current,
		save = function(id: string?, draft: look.Draft): any
			return M.host.save({ id = id, draft = draft })
		end,
		more = function(id: string?, draft: look.Draft)
			M.pending:set({ id = id, draft = draft })
			route.settings("agent-profiles")
		end,
		on_open = M.load,
		close_with = opts.close,
	}, dialog
end

return M
