-- Settings → Code review: the shipping app's (crates/ui/src/settings/
-- code_review.rs). The engine keeps who reviews and the instructions
-- (`GetReviewSettings`, `SetReviewSettings` through `settings.engine`); the
-- reviewers to choose from and an agent's models are zeron_proto's own list
-- (`review.reviewers`, `review.models`). The review chip runs this pick
-- unless a chat chose its own. Each pick shows at once, back as it was with
-- the engine's words when it refuses.

local host = require("@app/screens/settings/host")
local ds = require("@ds")
local search = require("@app/screens/settings/search")
local visits = require("@app/screens/settings/visits")

local M = {}

M.ID = "code_review"
M.TITLE = "Code review"

local function t(key: string): Message
	return ui.t(`settings.code_review.{key}`)
end

search.index(M.ID, t("title"), t("terms"), {
	{ key = "reviewer", title = t("reviewer.label"), section = t("reviewer.section"), terms = t("reviewer.terms") },
	{ key = "model", title = t("model.label"), section = t("reviewer.section"), terms = t("model.terms") },
	{ key = "thinking", title = t("thinking.label"), section = t("reviewer.section"), terms = t("thinking.terms") },
	{ key = "instructions", title = t("instructions.title"), section = t("instructions.title"), terms = t("instructions.terms") },
})

-- The engine's (`{ reviewer, instructions }`, nil until read), the reviewers for it, and each agent's models.
local settings = ui.state("settings_review", nil :: any)
local reviewers = ui.state("settings_review_reviewers", nil :: any)
local models = ui.state("settings_review_models", {} :: { [string]: { any } })
local unread = ui.state("settings_review_unread", nil :: string?)
local refused = ui.state("settings_review_refused", nil :: string?)

local function engine(method: string, params: { [string]: any }?): any
	return host.call("settings.engine", { method = method, params = params or {} })
end

local function says(err: any): string
	local word = if type(err) == "table" then err.message else err
	if word == "outdated" then
		return ui.t("settings.engine.outdated"):get()
	elseif word == "offline" then
		return ui.t("settings.engine.offline"):get()
	end
	return if type(word) == "string" and word ~= "" then word else ui.t("common.something_went_wrong"):get()
end

local function reviewer(): any
	local s = settings:get()
	return if s and s.reviewer then s.reviewer else { kind = "auto" }
end

local function list_for(current: any)
	host.call("review.reviewers", { current = current }):after(function(v: any)
		reviewers:set(v)
	end, function(err: any)
		unread:set(says(err))
	end)
	if current.kind == "harness" and current.harness and models:peek()[current.harness] == nil then
		local harness = current.harness
		host.call("review.models", { harness = harness }):after(function(v: any)
			local all = table.clone(models:peek())
			all[harness] = v or {}
			models:set(all)
		end, function() end)
	end
end

local function load()
	engine("GetReviewSettings"):after(function(v: any)
		settings:set(v or {})
		unread:set(nil)
		list_for((v or {}).reviewer or { kind = "auto" })
	end, function(err: any)
		unread:set(says(err))
	end)
end

M.load = load

local function store(next: { reviewer: any?, instructions: string? })
	local before = settings:peek()
	if not before then
		return
	end
	local shown = table.clone(before)
	for k, v in next :: any do
		shown[k] = v
	end
	settings:set(shown)
	list_for(shown.reviewer or { kind = "auto" })
	engine("SetReviewSettings", shown):after(function(v: any)
		if v ~= nil then
			settings:set(v)
		end
		refused:set(nil)
	end, function(err: any)
		settings:set(before)
		list_for(before.reviewer or { kind = "auto" })
		refused:set(says(err))
	end)
end

local function same(a: any, b: any): boolean
	if type(a) ~= "table" or type(b) ~= "table" then
		return a == b
	end
	for k, v in a do
		if b[k] ~= v then
			return false
		end
	end
	for k, v in b do
		if a[k] ~= v then
			return false
		end
	end
	return true
end

-- A pick of the page's own: it follows `read`, a change goes to `write`.
local function picker(kind: string, label: Message, read: () -> any, peek: () -> any, options: () -> { any }, write: (v: any) -> ()): Node
	return ui.component(function(): Node
		local chosen = ui.use_state(peek())
		ui.effect(function()
			local v = read()
			if v ~= chosen:peek() then
				chosen:set(v)
			end
		end)
		if kind == "segmented" then
			ui.effect(function()
				local v = chosen:get()
				if v ~= peek() then
					write(v)
				end
			end)
			return ds.Segmented { value = chosen, label = label, options = options() :: any }
		end
		return ds.Select {
			value = chosen,
			name = label,
			w = 196,
			options = options,
			on_change = function(v: any)
				if v ~= peek() then
					write(v)
				end
			end,
		}
	end)()
end

local function row(key: string, props: any): Node
	return search.anchor(key, ds.Settings.Row(props))
end

local function reviewer_row(): Node
	local function index(): any
		local r = reviewers:get()
		return if r then tostring(r.selected or 1) else "1"
	end
	return row("reviewer", {
		label = t("reviewer.label"),
		description = t("reviewer.hint"),
		control = picker("select", t("reviewer.label"), index, function(): any
			local r = reviewers:peek()
			return if r then tostring(r.selected or 1) else "1"
		end, function(): { any }
			local out = {}
			for i, c in (reviewers:get() or {}).choices or {} do
				table.insert(out, { value = tostring(i), label = c.label, description = c.detail })
			end
			return out
		end, function(v: any)
			local c = ((reviewers:peek() or {}).choices or {})[tonumber(v) or 0]
			if c and not same(c.choice, (settings:peek() or {}).reviewer) then
				store({ reviewer = c.choice })
			end
		end),
	})
end

local function harness_models(): { any }
	local r = reviewer()
	return if r.kind == "harness" and r.harness then models:get()[r.harness] or {} else {}
end

-- gpui's `reasoning_stops`: the chosen model's levels, else its agent's.
local function stops(): { any }
	local r = reviewer()
	for _, m in harness_models() do
		if m.id == r.model and m.levels and #m.levels > 0 then
			return m.levels
		end
	end
	return (reviewers:get() or {}).levels or {}
end

local function with(patch: { [string]: any }): any
	local next = table.clone(reviewer())
	for k, v in patch do
		next[k] = v
	end
	for _, k in { "model", "reasoning" } do
		if next[k] == "" then
			next[k] = nil
		end
	end
	return next
end

function M.view(): Node
	visits.once("code_review", load)
	local function ready(): boolean
		return settings:get() ~= nil and reviewers:get() ~= nil
	end
	local function harness(): boolean
		return reviewer().kind == "harness"
	end
	return ds.Settings.Page {
		title = t("title"),
		description = t("subtitle"),
		ui.show(function(): boolean
			return unread:get() ~= nil
		end, ds.Callout {
			tone = "warning",
			title = t("unread"),
			body = function(): string
				return unread:get() or ""
			end,
			action = {
				label = ui.t("common.retry") :: any,
				on_click = function()
					visits.again("code_review", load)
				end,
			},
		}),
		ui.show(function(): boolean
			return not ready() and unread:get() == nil
		end, ui.text { t("loading"), color = "$text.secondary", text_size = "$font.size.sm" }),
		ui.show(ready, ui.column {
			w_full = true,
			gap = "$space.6",
			ds.Settings.Section {
				title = t("reviewer.section"),
				ds.Settings.Group {
					ui.column {
						w_full = true,
						reviewer_row(),
						ui.show(harness, ui.column {
							w_full = true,
							ds.Settings.Rule(),
							row("model", {
								label = t("model.label"),
								description = t("model.hint"),
								control = picker("select", t("model.label"), function(): any
									return reviewer().model or ""
								end, function(): any
									local s = settings:peek()
									return if s and s.reviewer then s.reviewer.model or "" else ""
								end, function(): { any }
									local out = { { value = "", label = t("default"):get() } }
									for _, m in harness_models() do
										table.insert(out, { value = m.id, label = m.label })
									end
									return out
								end, function(v: any)
									store({ reviewer = with({ model = v }) })
								end),
							}),
						}),
						ui.show(function(): boolean
							return harness() and #stops() > 0
						end, ui.column {
							w_full = true,
							ds.Settings.Rule(),
							row("thinking", {
								label = t("thinking.label"),
								description = t("thinking.hint"),
								stacked = true,
								control = ui.presence(function(): string
									local keys = {}
									for _, l in stops() do
										table.insert(keys, tostring(l.value))
									end
									return table.concat(keys, ",")
								end, function(_: string): Node
									local options = { { value = "", label = t("default") } }
									for _, l in stops() do
										table.insert(options, { value = l.value, label = l.label })
									end
									return picker("segmented", t("thinking.label"), function(): any
										return reviewer().reasoning or ""
									end, function(): any
										local s = settings:peek()
										return if s and s.reviewer then s.reviewer.reasoning or "" else ""
									end, function(): { any }
										return options
									end, function(v: any)
										store({ reviewer = with({ reasoning = v }) })
									end)
								end),
							}),
						}),
					},
				},
				ui.text { t("override"), wrap = true, px = "$space.2", text_size = "$font.size.sm", color = "$text.secondary" },
				ui.show(function(): boolean
					return refused:get() ~= nil
				end, ui.text {
					function(): string
						return refused:get() or ""
					end,
					id = "code-review-refused",
					role = "alert",
					wrap = true,
					px = "$space.4",
					text_size = "$font.size.sm",
					color = "$danger",
				}),
			},
			ds.Settings.Section {
				title = t("instructions.title"),
				search.anchor("instructions", ui.component(function(): Node
					local text = ui.use_state((settings:peek() or {}).instructions or "")
					ui.effect(function()
						local saved = (settings:get() or {}).instructions or ""
						if saved ~= text:peek() then
							text:set(saved)
						end
					end)
					local function dirty(): boolean
						local saved = (settings:get() or {}).instructions or ""
						return (string.match(text:get(), "^%s*(.-)%s*$") or "") ~= (string.match(saved, "^%s*(.-)%s*$") or "")
					end
					return ds.Settings.Group {
						ui.column {
							w_full = true,
							gap = "$space.2",
							p = "$space.4",
							ui.text { t("instructions.hint"), wrap = true, text_size = "$font.size.sm", color = "$text.secondary" },
							ds.TextArea {
								id = "code-review-instructions",
								value = text,
								name = t("instructions.title"),
								placeholder = t("instructions.placeholder"),
								rows = 3,
								max_rows = 14,
							},
							ui.row {
								w_full = true,
								justify = "end",
								ds.Button {
									t("instructions.save"),
									id = "code-review-instructions-save",
									size = "sm",
									disabled = function(): boolean
										return not dirty()
									end,
									on_click = function()
										if dirty() then
											store({ instructions = string.match(text:peek(), "^%s*(.-)%s*$") or "" })
										end
									end,
								},
							},
						},
					}
				end)()),
			},
		}),
	}
end

return M
