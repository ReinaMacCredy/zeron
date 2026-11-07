-- Agent profiles as the host would answer them (src/agent_profiles.rs), for
-- the fixtures (chat_fixture.luau, shell_fixture.luau): five profiles (two
-- of Claude's, one an agent made, Codex's, one on the generic ACP agent), and
-- each action of screens/chat/composer_profiles' `host`, noted. A chat's
-- profile and its drift ride its session, worked out here from what the
-- profile sets (model, effort, mode, instructions, tools), as the host's are.
-- `store`, when given: a configure there refreshes the chat's drift.
local store = require("@app/screens/chat/store")

local M = {}

function M.install(o: { note: (string) -> (), meta: { [string]: any }, publish: (string, any) -> (), store: any?, settled: (any) -> any })
	local note, META, publish, stub, settled = o.note, o.meta, o.publish, o.store, o.settled
	local profiles = require("@app/screens/chat/composer_profiles")
	local function profile(p: any): any
		p.options = p.options or {}
		p.notes = p.notes or ""
		p.instructions = p.instructions or ""
		p.tools_off = p.tools_off or {}
		p.harness_label = p.harness_label or (if p.harness == "claude-code" then "Claude" elseif p.harness == "codex" then "Codex" else "ACP")
		p.mark = p.mark or (if p.harness == "claude-code" then "claude" elseif p.harness == "codex" then "openai" else "pi")
		return p
	end
	local LEVEL: { [string]: string } = { low = "Low", medium = "Medium", high = "High" }
	local LIST: { any } = {
		profile { id = "planner", name = "Planner", harness = "claude-code", model = "opus", model_label = "Opus", effort = "high", effort_label = "High", access = "edits", notes = "Plans before it edits" },
		profile { id = "reviewer", name = "Reviewer", harness = "claude-code", model = "sonnet", model_label = "Sonnet", instructions = "Review only.", tools_off = { "WebFetch" } },
		profile { id = "scout", name = "Scout", harness = "claude-code", access = "supervised", by_agent = true, notes = "Reads the repo for a delegate" },
		profile { id = "quick", name = "Quick", harness = "codex", model = "gpt", model_label = "GPT-5.5", effort = "medium", effort_label = "Medium" },
		profile { id = "relay", name = "Relay", harness = "acp", harness_label = "ACP" },
	}
	local function publish_list()
		profiles.list:set({ state = "ready", profiles = table.clone(LIST) })
	end
	publish_list()
	local function find(id: string?): any?
		for _, p in LIST do
			if p.id == id then
				return p
			end
		end
		return nil
	end
	local function harness_of(s: any): string
		return s.harness or "claude-code"
	end
	local function label_of(value: string?): string
		for _, m in store.catalog:peek().models do
			if m.value == value then
				return m.label
			end
		end
		return value or ""
	end
	local function same_set(a: { string }, b: { string }): boolean
		if #a ~= #b then
			return false
		end
		for _, x in a do
			if not table.find(b, x) then
				return false
			end
		end
		return true
	end
	-- What the host's `applied_value` says of `s` against its profile.
	local function applied(p: any, s: any): any
		local drift: { any } = {}
		if harness_of(s) ~= p.harness then
			drift = { { kind = "harness", from = p.harness_label, to = harness_of(s) } }
		else
			if p.model and s.model ~= p.model then
				table.insert(drift, { kind = "model", from = p.model_label, to = label_of(s.model) })
			end
			if p.effort and s.effort ~= p.effort then
				table.insert(drift, { kind = "effort", from = LEVEL[p.effort] or p.effort, to = LEVEL[s.effort or ""] or (s.effort or "") })
			end
			if p.access and s.access ~= p.access then
				table.insert(drift, { kind = "mode", from = p.access, to = s.access })
			end
			if not same_set(p.tools_off, s.tools_off or {}) then
				table.insert(drift, { kind = "tools" })
			end
			if (s.instructions or "") ~= p.instructions then
				table.insert(drift, { kind = "instructions" })
			end
		end
		local savable = false
		for _, c in drift do
			savable = savable or c.kind ~= "instructions"
		end
		return { id = p.id, name = p.name, harness = p.harness, mark = p.mark, edited = #drift > 0, savable = savable, drift = drift }
	end
	local function refresh(id: string)
		local s = META[id]
		if not s then
			return
		end
		local p = find(s.profile_id)
		publish(id, { profile = if p then applied(p, s) else ui.null })
	end
	if stub then
		local base_configure = stub.configure
		stub.configure = function(id: string, patch: any)
			base_configure(id, patch)
			refresh(id)
		end
	end
	local failing = ui.state("fixture_profiles_failing", false)
	profiles.host = {
		load = function()
			note("profiles_load")
		end,
		apply = function(args: any): any
			note(`apply_profile:{args.id}:{args.profile or "none"}{if args.note then "|note" else ""}`)
			local p = find(args.profile)
			local s = META[args.id] or {}
			if not p then
				publish(args.id, { profile_id = ui.null, instructions = "" })
			else
				local patch: { [string]: any } = { profile_id = p.id, instructions = p.instructions, tools_off = table.clone(p.tools_off) }
				if harness_of(s) ~= p.harness then
					patch.harness = p.harness
				end
				if p.model then
					patch.model = p.model
				end
				if p.effort then
					patch.effort = p.effort
				end
				if p.access then
					patch.access = p.access
				end
				publish(args.id, patch)
			end
			refresh(args.id)
			return settled(nil)
		end,
		absorb = function(args: any): any
			note(`absorb:{args.id}`)
			local p = find(args.id)
			local s = META[args.current.chat or ""]
			if p and s then
				if p.model then
					p.model = s.model
					p.model_label = label_of(s.model)
				end
				if p.effort then
					p.effort = s.effort
					p.effort_label = LEVEL[s.effort or ""] or s.effort
				end
				if p.access then
					p.access = s.access
				end
				p.tools_off = table.clone(s.tools_off or {})
				publish_list()
				refresh(args.current.chat)
			end
			return settled(nil)
		end,
		save = function(args: any): any
			note(`save_profile:{args.id or "new"}:{args.draft.name}`)
			if failing:peek() then
				return {
					after = function(_self: any, _ok: any, err: (any) -> ())
						err({ message = "Profile name must not exceed 80 characters" })
					end,
				}
			end
			local d = args.draft
			local p = find(args.id)
			if not p then
				p = { id = string.lower((string.gsub(d.name, "%s+", "-"))) }
				table.insert(LIST, p)
			end
			for k, v in d do
				p[k] = v
			end
			profile(p)
			publish_list()
			return settled(nil)
		end,
		remove = function(args: any): any
			note(`remove_profile:{args.id}`)
			for i, p in LIST do
				if p.id == args.id then
					table.remove(LIST, i)
					break
				end
			end
			publish_list()
			return settled(nil)
		end,
		reorder = function(args: any): any
			note(`reorder_profiles:{table.concat(args.ids, ",")}`)
			local next_list = {}
			for _, id in args.ids do
				table.insert(next_list, find(id))
			end
			LIST = next_list
			publish_list()
			return settled(nil)
		end,
		tools = function(args: any): any
			note(`profile_tools:{args.harness}`)
			return settled({
				groups = {
					{ id = "Threads", title = "Threads", tools = { { name = "create_thread", rule = "mcp__zeron__create_thread", locked = false }, { name = "delegate_task", rule = "mcp__zeron__delegate_task", locked = false } } },
					{ id = "Computer", title = "Computer", tools = { { name = "screenshot", rule = "mcp__zeron__screenshot", locked = true } } },
				},
				claude = if args.harness == "claude-code" then { state = "ready", builtin = { { id = "web", title = "Web", summary = "Fetch and search", rules = { "WebFetch", "WebSearch" } } }, mcp = {} } else nil,
				unlisted = {},
			})
		end,
		system_prompt = function(): any
			return settled("Answer briefly.")
		end,
		set_system_prompt = function(args: any): any
			note(`system_prompt:{args.prompt}`)
			return settled(args.prompt)
		end,
		drift = function(args: any): any
			local p = find(args.profile)
			if not p then
				return nil
			end
			local s = { harness = nil, model = args.model, effort = args.effort, access = args.access, tools_off = p.tools_off, instructions = p.instructions }
			for _, m in store.catalog:peek().models do
				if m.value == args.model then
					s.harness = m.harness
				end
			end
			return applied(p, s)
		end,
		current = function(args: any): any
			local s = if args.chat then META[args.chat] else args
			if not s then
				return nil
			end
			return {
				name = "",
				harness = harness_of(s),
				model = s.model,
				effort = s.effort,
				access = s.access,
				options = s.options or {},
				notes = "",
				instructions = s.instructions or "",
				tools_off = table.clone(s.tools_off or {}),
			}
		end,
	}
	-- The profiles the engine couldn't list, and saves it turns away.
	ui.command {
		id = "profiles_fail",
		title = "Profiles don't list",
		run = function()
			profiles.list:set({ state = "error", error = "engine offline", profiles = {} })
		end,
	}
	ui.command {
		id = "profiles_save_fails",
		title = "The engine turns a save away",
		run = function()
			failing:set(true)
		end,
	}
	-- A host older than profiles: no `profiles.load` (it raises, as an
	-- action a host lacks does), the list never asked for.
	ui.command {
		id = "profiles_old_host",
		title = "A host without profiles",
		run = function()
			profiles.host.load = function()
				error("no host action named `profiles.load`")
			end
			profiles.list:set({ state = "idle", profiles = {} })
		end,
	}
	-- An agent whose catalog names no access mode (Full access alone).
	ui.command {
		id = "catalog_full_only",
		title = "An agent with no modes listed",
		run = function()
			local c = store.catalog:peek()
			local harnesses: { any } = table.clone(c.harnesses)
			local models: { any } = table.clone(c.models)
			table.insert(harnesses, { id = "fx", label = "Fx", mark = "pi", access_modes = {} })
			table.insert(models, { value = "fx-1", label = "Fx 1", harness = "fx" })
			store.catalog:set { state = "ready", harnesses = harnesses, models = models }
		end,
	}
	-- s2 on Planner, as the host says once it's applied.
	ui.command {
		id = "s2_on_planner",
		title = "s2 runs on Planner",
		run = function()
			if not META.s2 then
				return
			end
			META.s2.profile_id = "planner"
			META.s2.model = "opus"
			META.s2.effort = "high"
			META.s2.access = "edits"
			refresh("s2")
		end,
	}
end

return M
