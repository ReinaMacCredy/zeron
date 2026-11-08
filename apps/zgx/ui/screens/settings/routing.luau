--!strict
-- Settings → Routing, as gpui's (crates/ui settings/routing.rs and
-- mission_delegation.rs): which agent and model a delegated child runs on
-- when the agent passes a role and no model, what happens when that agent
-- is out of quota, Claude Code's own sub-agents, what the agents are told,
-- and delegate_task's limits. All the engine's (Get/SetRoutingSettings,
-- Get/SetDelegationSettings, through ENGINE_METHODS): a change goes at once,
-- and what the engine saved is what shows. The quota and the fallback lines
-- are PreviewRoutes' (host `settings.routing_preview`, read as gpui reads
-- it); the agents offered are the pickers' (host `routing.harnesses`), each
-- one's models `review.models`.
--
-- gpui lays the roles out as a table; here each role is a row with its
-- picks under its name, so the page keeps Settings' measure.

local ds = require("@ds")
local host = require("@app/screens/settings/host")
local search = require("@app/screens/settings/search")
local visits = require("@app/screens/settings/visits")
local compaction = require("@app/screens/settings/compaction")

local M = {}

M.ID = "routing"
M.TITLE = "Routing"

local ROLES = { "worker", "validator", "research", "design", "general" }
local POLICIES = { "native", "nextInList", "wait", "ask" }
local DEPTH_MAX, CHILDREN_MAX = 5, 32

search.index(M.ID, ui.t("settings.routing.title"), ui.t("settings.routing.terms"), {
	{ key = "roles", title = ui.t("settings.routing.roles"), terms = ui.t("settings.routing.roles_terms") },
	{ key = "out_of_quota", title = ui.t("settings.routing.out_of_quota.label"), section = ui.t("settings.routing.quota_section"), terms = ui.t("settings.routing.out_of_quota.terms") },
	{ key = "fallback", title = ui.t("settings.routing.fallback.label"), section = ui.t("settings.routing.quota_section"), terms = ui.t("settings.routing.fallback.terms") },
	{ key = "mid_run", title = ui.t("settings.routing.mid_run.label"), section = ui.t("settings.routing.quota_section"), terms = ui.t("settings.routing.mid_run.terms") },
	{ key = "native", title = ui.t("settings.routing.native.label"), section = ui.t("settings.routing.native_section"), terms = ui.t("settings.routing.native.terms") },
	{ key = "guidance", title = ui.t("settings.routing.guidance.label"), section = ui.t("settings.routing.native_section"), terms = ui.t("settings.routing.guidance.terms") },
	{ key = "depth", title = ui.t("settings.routing.depth.label"), section = ui.t("settings.routing.delegation"), terms = ui.t("settings.routing.depth.terms") },
	{ key = "children", title = ui.t("settings.routing.children.label"), section = ui.t("settings.routing.delegation"), terms = ui.t("settings.routing.children.terms") },
})

local settings = ui.state("settings_routing", nil :: any)
local unread = ui.state("settings_routing_unread", nil :: string?)
local refused = ui.state("settings_routing_refused", nil :: string?)
local harnesses = ui.state("settings_routing_harnesses", {} :: { any })
local models = ui.state("settings_routing_models", {} :: { [string]: { any } })
local preview = ui.state("settings_routing_preview", nil :: any)
local delegation = ui.state("settings_routing_delegation", nil :: any)
local delegation_error = ui.state("settings_routing_delegation_error", nil :: string?)

local function engine(method: string, params: any?): any
	return host.call("settings.engine", { method = method, params = params or {} })
end

local function load_preview()
	local ok, p = pcall(host.call, "settings.routing_preview")
	if ok and p ~= nil then
		p:after(function(v: any)
			preview:set(v)
		end, function() end)
	end
end

local function load()
	engine("GetRoutingSettings"):after(function(v: any)
		settings:set(v)
		unread:set(nil)
	end, function(err: any)
		unread:set(compaction.says(err))
	end)
	engine("GetDelegationSettings"):after(function(v: any)
		delegation:set(v)
		delegation_error:set(nil)
	end, function(err: any)
		delegation_error:set(compaction.says(err))
	end)
	local ok, p = pcall(host.call, "routing.harnesses")
	if ok and p ~= nil then
		p:after(function(list: any)
			harnesses:set(if type(list) == "table" then list else {})
			for _, h in harnesses:peek() do
				host.call("review.models", { harness = h.id }):after(function(m: any)
					local all = table.clone(models:peek())
					all[h.id] = if type(m) == "table" then m else {}
					models:set(all)
				end, function() end)
			end
		end, function() end)
	end
	load_preview()
end
M.load = load

local function deep(v: any): any
	if type(v) ~= "table" then
		return v
	end
	local out = {}
	for k, x in v do
		out[k] = deep(x)
	end
	return out
end

-- A change to the engine at once; what it saved stands, a refusal puts the
-- last back with its words under the page's line.
local function save(edit: (any) -> ())
	local before = settings:peek()
	if before == nil then
		return
	end
	local next = deep(before)
	next.roles = next.roles or {}
	next.fallback = next.fallback or {}
	edit(next)
	settings:set(next)
	engine("SetRoutingSettings", next):after(function(saved: any)
		settings:set(if type(saved) == "table" then saved else next)
		refused:set(nil)
		load_preview()
	end, function(err: any)
		settings:set(before)
		refused:set(compaction.says(err))
	end)
end

local function save_delegation(field: string, n: number)
	local before = delegation:peek()
	if before == nil or before[field] == n then
		return
	end
	local next = table.clone(before)
	next[field] = n
	delegation:set(next)
	engine("SetDelegationSettings", next):after(function(saved: any)
		if type(saved) == "table" and saved.maxDepth then
			delegation:set(saved)
		end
		delegation_error:set(nil)
	end, function(err: any)
		delegation:set(before)
		delegation_error:set(compaction.says(err))
	end)
end

local function route_of(role: string): any
	local s = settings:get()
	return (s and s.roles and s.roles[role]) or { kind = "off" }
end

local function harness_of(id: string?): any
	for _, h in harnesses:get() do
		if h.id == id then
			return h
		end
	end
	return nil
end

local function label_of(id: string?): string
	local h = harness_of(id)
	return if h then h.label else (id or "")
end

local function target_label(t: any): string
	if t.model then
		for _, m in models:get()[t.harness] or {} do
			if m.id == t.model then
				return `{label_of(t.harness)} {m.label}`
			end
		end
		return `{label_of(t.harness)} {t.model}`
	end
	return label_of(t.harness)
end

-- A pick of the page's own: it follows `read`, a change goes to `write`
-- (Code review's way).
local function picker(name: Message | (() -> string), w: number, read: () -> any, options: () -> { any }, write: (v: any) -> ()): Node
	return ui.component(function(): Node
		local chosen = ui.use_state(read())
		ui.effect(function()
			local v = read()
			if v ~= chosen:peek() then
				chosen:set(v)
			end
		end)
		return ds.Select {
			value = chosen,
			name = name :: any,
			w = w,
			options = options,
			on_change = function(v: any)
				write(v)
			end,
		}
	end)()
end

local function role_harness(role: string): Node
	return picker(function(): string
		return ui.t("settings.routing.harness_for", { role = role }):get()
	end, 180, function(): any
		local r = route_of(role)
		return if r.kind == "target" then r.harness else r.kind
	end, function(): { any }
		local out: { any } = {
			{ value = "off", label = ui.t("settings.routing.off"):get() },
			{ value = "sameAsParent", label = ui.t("settings.routing.same_as_parent"):get() },
		}
		for _, h in harnesses:get() do
			table.insert(out, { value = h.id, label = h.label })
		end
		return out
	end, function(v: any)
		save(function(s: any)
			if v == "off" or v == "sameAsParent" then
				s.roles[role] = { kind = v }
			else
				s.roles[role] = { kind = "target", harness = v }
			end
		end)
	end)
end

local DEFAULT = "__default"

local function role_model(role: string): Node
	return picker(function(): string
		return ui.t("settings.routing.model_for", { role = role }):get()
	end, 160, function(): any
		return route_of(role).model or DEFAULT
	end, function(): { any }
		local out: { any } = { { value = DEFAULT, label = ui.t("settings.routing.default"):get() } }
		for _, m in models:get()[route_of(role).harness] or {} do
			table.insert(out, { value = m.id, label = m.label })
		end
		return out
	end, function(v: any)
		save(function(s: any)
			local r = s.roles[role]
			if r and r.kind == "target" then
				r.model = if v == DEFAULT then nil else v
				r.reasoning = nil
			end
		end)
	end)
end

-- The chosen model's levels, else its agent's (gpui's `reasoning_stops`).
local function levels_of(r: any): { any }
	for _, m in models:get()[r.harness] or {} do
		if m.id == r.model and m.levels and #m.levels > 0 then
			return m.levels
		end
	end
	local h = harness_of(r.harness)
	return (h and h.levels) or {}
end

local function role_reasoning(role: string): Node
	return ui.show(function(): boolean
		return #levels_of(route_of(role)) > 0
	end, picker(function(): string
		return ui.t("settings.routing.reasoning_for", { role = role }):get()
	end, 120, function(): any
		return route_of(role).reasoning or DEFAULT
	end, function(): { any }
		local out: { any } = { { value = DEFAULT, label = ui.t("settings.routing.default"):get() } }
		for _, l in levels_of(route_of(role)) do
			table.insert(out, { value = l.value, label = l.label })
		end
		return out
	end, function(v: any)
		save(function(s: any)
			local r = s.roles[role]
			if r and r.kind == "target" then
				r.reasoning = if v == DEFAULT then nil else v
			end
		end)
	end))
end

local function quota_of(harness: string?): any
	local p = preview:get()
	return p and p.quota and harness and p.quota[harness]
end

-- The routed agent's quota: a bar with "Codex 42% · resets 3:45 PM", or
-- that it reports none.
local function quota_cell(role: string): Node
	local function q(): any
		local r = route_of(role)
		return if r.kind == "target" then quota_of(r.harness) else nil
	end
	return ui.show(function(): boolean
		return route_of(role).kind == "target"
	end, ui.column {
		class = "routing-quota",
		w = 150,
		gap = "$space.1",
		ui.show(function(): boolean
			return q() ~= nil
		end, ds.ProgressBar {
			w_full = true,
			size = "thin",
			value = function(): number
				local x = q()
				return if x then math.clamp(x.fraction or 0, 0, 1) else 0
			end,
			tone = function(): string
				local x = q()
				local f = if x then x.fraction or 0 else 0
				return if f >= 1 then "danger" elseif f >= 0.8 then "warning" else "accent"
			end,
			label = ui.t("settings.routing.quota") :: any,
		} :: any),
		ui.text {
			function(): string
				local r = route_of(role)
				local x = q()
				if not x then
					local h = harness_of(r.harness)
					return ui.t("settings.routing.no_quota", { name = if h then h.short else (r.harness or "") }):get()
				end
				local percent = math.floor((x.fraction or 0) * 100 + 0.5)
				local reset = if x.resets_at then ds.ProviderCard.reset_words(x.resets_at / 1000) else nil
				return if reset then `{x.name} {percent}% · {reset}` else `{x.name} {percent}%`
			end,
			text_size = "$font.size.xs",
			color = "$text.tertiary",
			truncate = true,
		},
	})
end

local function role_row(role: string): Node
	local function row_preview(): any
		local p = preview:get()
		return p and p.rows and p.rows[role]
	end
	return ui.column {
		id = `routing-role-{role}`,
		w_full = true,
		ds.Settings.Row {
			label = role,
			badge = ui.show(function(): boolean
				local r = row_preview()
				return r ~= nil and r.fell_back == true
			end, ds.Badge { ui.t("settings.routing.fell_back"), tone = "accent" }),
			description = function(): string
				local r = row_preview()
				if r and r.line then
					return r.line
				end
				local route = route_of(role)
				if route.kind == "target" and quota_of(route.harness) == nil and preview:get() ~= nil then
					return ui.t("settings.routing.no_quota_line"):get()
				end
				return ui.t(`settings.routing.role.{role}`):get()
			end,
			stacked = true,
			control = ui.row {
				w_full = true,
				gap = "$space.2",
				items = "center",
				wrap = true,
				role_harness(role),
				ui.switch(function(): string
					return route_of(role).kind
				end, {
					off = ui.text { ui.t("settings.routing.off_line"), text_size = "$font.size.sm", color = "$text.tertiary" },
					sameAsParent = ui.text { ui.t("settings.routing.parent_line"), text_size = "$font.size.sm", color = "$text.tertiary" },
					target = ui.row {
						gap = "$space.2",
						items = "center",
						role_model(role),
						ui.text {
							function(): string
								local r = row_preview()
								return (r and r.multiplier) or ""
							end,
							text_size = "$font.size.xs",
							color = "$text.tertiary",
						},
						role_reasoning(role),
					},
				}),
				ui.div { grow = 1 },
				quota_cell(role),
			},
		},
	}
end

local function policy_select(): Node
	return picker(ui.t("settings.routing.out_of_quota.label"), 260, function(): any
		local s = settings:get()
		return (s and s.outOfQuota) or "native"
	end, function(): { any }
		local out: { any } = {}
		for _, p in POLICIES do
			local label = ui.t(`settings.routing.policy.{p}`):get()
			if p == "native" then
				label = ui.t("settings.routing.recommended", { label = label }):get()
			end
			table.insert(out, { value = p, label = label, description = ui.t(`settings.routing.policy.{p}_detail`):get() })
		end
		return out
	end, function(v: any)
		save(function(s: any)
			s.outOfQuota = v
		end)
	end)
end

local function same_target(a: any, b: any): boolean
	return a.harness == b.harness and a.model == b.model and a.reasoning == b.reasoning
end

-- The fallback list: each in order with its Remove, then Add with the
-- agents' models not in it yet.
local function fallback_list(): Node
	local adding = ui.state("settings_routing_fallback_add", "")
	local function candidates(): { any }
		local s = settings:get()
		local listed = (s and s.fallback) or {}
		local out: { any } = {}
		for _, h in harnesses:get() do
			local ms = models:get()[h.id] or {}
			local options: { any } = {}
			if #ms == 0 then
				table.insert(options, { harness = h.id })
			end
			for _, m in ms do
				table.insert(options, { harness = h.id, model = m.id })
			end
			for _, t in options do
				local there = false
				for _, f in listed do
					there = there or same_target(f, t)
				end
				if not there then
					table.insert(out, { value = `{t.harness}/{t.model or ""}`, label = target_label(t), target = t })
				end
			end
		end
		return out
	end
	return ui.column {
		id = "routing-fallback",
		w_full = true,
		gap = "$space.2",
		ui.each(ui.computed(function(): { any }
			local s = settings:get()
			local out = {}
			for i, f in (s and s.fallback) or {} do
				table.insert(out, { i = i, target = f })
			end
			return out
		end), function(item: any): Node
			return ui.row {
				w_full = true,
				gap = "$space.2",
				items = "center",
				ui.text { tostring(item.i), w = 16, text_size = "$font.size.sm", color = "$text.tertiary" },
				ui.text { target_label(item.target), grow = 1, text_size = "$font.size.sm", color = "$text.primary" },
				ds.IconButton {
					icon = "close",
					size = "sm",
					tone = "quiet",
					label = ui.t("settings.routing.fallback.remove", { name = target_label(item.target) }):get(),
					on_click = function()
						save(function(s: any)
							table.remove(s.fallback, item.i)
						end)
					end,
				},
			}
		end, { key = function(item: any): string
			return `{item.i}:{item.target.harness}/{item.target.model or ""}`
		end, w_full = true, gap = "$space.1" }),
		ui.show(function(): boolean
			return #candidates() > 0
		end, ds.Select {
			value = adding,
			name = ui.t("settings.routing.fallback.add") :: any,
			placeholder = ui.t("settings.routing.fallback.add"),
			w = 220,
			options = candidates,
			on_change = function(v: any)
				for _, c in candidates() do
					if c.value == v then
						save(function(s: any)
							table.insert(s.fallback, c.target)
						end)
					end
				end
				adding:set("")
			end,
		}),
	}
end

local function mid_run(): Node
	return ui.component(function(): Node
		local chosen = ui.use_state(((settings:peek() or {}) :: any).midRun or "after30s")
		ui.effect(function()
			local v = ((settings:get() or {}) :: any).midRun or "after30s"
			if v ~= chosen:peek() then
				chosen:set(v)
			end
		end)
		ui.effect(function()
			local v = chosen:get()
			if settings:peek() and v ~= (settings:peek().midRun or "after30s") then
				save(function(s: any)
					s.midRun = v
				end)
			end
		end)
		return ds.Segmented {
			value = chosen,
			label = ui.t("settings.routing.mid_run.label"),
			options = {
				{ value = "ask", label = ui.t("settings.routing.mid_run.ask") },
				{ value = "after10s", label = ui.t("settings.routing.mid_run.after10s") },
				{ value = "after30s", label = ui.t("settings.routing.mid_run.after30s") },
			} :: any,
		}
	end)()
end

local function native_select(): Node
	return picker(ui.t("settings.routing.native.label"), 200, function(): any
		local s = settings:get()
		return (s and s.native) or "sameModelOnly"
	end, function(): { any }
		return {
			{ value = "sameModelOnly", label = ui.t("settings.routing.native.same_model"):get() },
			{ value = "always", label = ui.t("settings.routing.native.always"):get() },
			{ value = "off", label = ui.t("settings.routing.native.off"):get() },
		}
	end, function(v: any)
		save(function(s: any)
			s.native = v
		end)
	end)
end

local function stepper(field: string, label: Message, max: number): Node
	return ui.component(function(): Node
		local n = ui.use_state(((delegation:peek() or {}) :: any)[field] or 1)
		ui.effect(function()
			local v = ((delegation:get() or {}) :: any)[field]
			if v and v ~= n:peek() then
				n:set(v)
			end
		end)
		return ui.div { id = `routing-{field}`, ds.NumberStepper {
			value = n,
			min = 1,
			max = max,
			step = 1,
			name = label,
			w = 110,
			on_change = function(v: number)
				save_delegation(field, v)
			end,
		} }
	end)()
end

local function row(key: string, props: any): Node
	return search.anchor(key, ds.Settings.Row(props))
end

local function body(): Node
	return ui.column {
		w_full = true,
		gap = "$space.6",
		ui.show(function(): boolean
			return refused:get() ~= nil
		end, ui.column { id = "routing-refused", w_full = true, ds.Callout {
			tone = "danger",
			title = ui.t("settings.routing.couldnt_save") :: any,
			body = function(): string
				return refused:get() or ""
			end,
		} }),
		search.anchor("roles", ds.Settings.Section {
			title = ui.t("settings.routing.roles"),
			description = ui.t("settings.routing.roles_note"),
			ds.Settings.Group((function(): { any }
				local rows: { any } = {}
				for i, r in ROLES do
					if i > 1 then
						table.insert(rows, ds.Settings.Rule())
					end
					table.insert(rows, role_row(r))
				end
				return rows
			end)()),
		}),
		ds.Settings.Section {
			title = ui.t("settings.routing.quota_section"),
			ds.Settings.Group {
				row("out_of_quota", {
					label = ui.t("settings.routing.out_of_quota.label"),
					description = ui.t("settings.routing.out_of_quota.description"),
					control = policy_select(),
				}),
				row("fallback", {
					label = ui.t("settings.routing.fallback.label"),
					description = ui.t("settings.routing.fallback.description"),
					stacked = true,
					control = fallback_list(),
				}),
				row("mid_run", {
					label = ui.t("settings.routing.mid_run.label"),
					description = ui.t("settings.routing.mid_run.description"),
					control = mid_run(),
				}),
			},
		},
		ds.Settings.Section {
			title = ui.t("settings.routing.native_section"),
			ds.Settings.Group {
				row("native", {
					label = ui.t("settings.routing.native.label"),
					description = ui.t("settings.routing.native.description"),
					control = native_select(),
				}),
				row("guidance", {
					label = ui.t("settings.routing.guidance.label"),
					description = ui.t("settings.routing.guidance.description"),
					stacked = true,
					control = ui.text {
						id = "routing-guidance",
						function(): string
							local p = preview:get()
							if p == nil then
								return ui.t("settings.routing.loading"):get()
							end
							return if (p.guidance or "") ~= "" then p.guidance else ui.t("settings.routing.guidance.none"):get()
						end,
						w_full = true,
						text_size = "$font.size.xs",
						font = "$font.mono",
						color = "$text.secondary",
					},
				}),
			},
		},
		ds.Settings.Section {
			title = ui.t("settings.routing.delegation"),
			ui.show(function(): boolean
				return delegation:get() ~= nil
			end, ds.Settings.Group {
				row("depth", {
					label = ui.t("settings.routing.depth.label"),
					description = ui.t("settings.routing.depth.description"),
					control = stepper("maxDepth", ui.t("settings.routing.depth.label"), DEPTH_MAX),
				}),
				row("children", {
					label = ui.t("settings.routing.children.label"),
					description = ui.t("settings.routing.children.description"),
					control = stepper("maxRunningChildren", ui.t("settings.routing.children.label"), CHILDREN_MAX),
				}),
			}),
			ui.show(function(): boolean
				return delegation_error:get() ~= nil
			end, ui.text {
				id = "routing-delegation-error",
				function(): string
					return delegation_error:get() or ""
				end,
				text_size = "$font.size.sm",
				color = "$zeron.tone.danger.ink",
			}),
		},
	}
end

function M.view(): Node
	visits.once(M.ID, load)
	return ds.Settings.Page {
		title = ui.t("settings.routing.title"),
		description = ui.t("settings.routing.description"),
		ui.switch(function(): string
			if settings:get() ~= nil then
				return "ready"
			end
			return if unread:get() ~= nil then "error" else "loading"
		end, {
			loading = ui.text { ui.t("settings.routing.loading"), color = "$text.tertiary" },
			error = ds.Callout {
				tone = "danger",
				title = ui.t("settings.routing.couldnt_load") :: any,
				body = function(): string
					return unread:get() or ""
				end,
				action = { label = ui.t("common.retry") :: any, on_click = function()
					visits.again(M.ID, load)
				end },
			},
			ready = body(),
		}),
	}
end

return M
