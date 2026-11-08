-- Settings → Usage: what this device's agents spent, the shipping app's Usage
-- page (crates/ui usage.rs) in this design: over a window, in dollars or
-- tokens, a bar for each hour or day, the totals, and a breakdown by model or
-- by period. Plan limits stay on Providers' cards (ds/provider_card).
--
-- The figures are the engine's, formatted by the host (src/usage.rs): nothing
-- here adds anything up.

local ds = require("@ds")
local usage = require("@account/usage")
local search = require("@app/screens/settings/search")

local M = {}

M.ID = "usage"
M.TITLE = "Usage"

search.index(M.ID, ui.t("settings.usage.title"), ui.t("settings.usage.terms"), {
	{ key = "spend", title = ui.t("settings.usage.spend.title"), terms = ui.t("settings.usage.terms") },
	{ key = "breakdown", title = ui.t("settings.usage.breakdown.title"), terms = ui.t("settings.usage.breakdown.terms") },
})

local CHART_H = 112
-- gpui's value axis: its labels' column, ticks aimed at.
local AXIS_W, TICKS = 56, 4
-- An agent's colour in the sums (usage.rs `provider_color`): Claude's brand, Grok's grey, the rest the text's.
local BRAND: { [string]: string } = { ["claude-code"] = "#d97757" }
local HARNESS: { [string]: string } = { ["claude-code"] = "Claude Code", codex = "Codex", grok = "Grok" }
local MARKS: { [string]: string } = { ["claude-code"] = "claude", codex = "openai", grok = "grok" }

local days = ui.state("settings_usage_days", 30)
local metric = ui.state("settings_usage_metric", "cost")
local by = ui.state("settings_usage_by", "model")
local data = ui.state("settings_usage_data", nil :: any)
local failed = ui.state("settings_usage_failed", false)
local loading = ui.state("settings_usage_loading", false)

local function load()
	local asked = days:peek()
	failed:set(false)
	loading:set(true)
	usage.summary(asked):after(function(summary: any)
		if days:peek() == asked then
			data:set(summary)
			loading:set(false)
		end
	end, function()
		if days:peek() == asked then
			failed:set(data:peek() == nil)
			loading:set(false)
		end
	end)
end

-- A period's name, as on a bar: "3 PM", or "Oct 5".
function M.period_label(start: number, hourly: boolean): string
	local at = math.floor(start)
	if hourly then
		local hour = string.gsub(os.date("%I %p", at) :: any, "^0", "")
		return hour
	end
	local t = os.date("*t", at) :: any
	return ui.t("time.month_day", { m = tostring(t.month), day = tostring(t.day) }):get()
end

-- The value axis's top and its ticks, rounded up to a readable step (usage.rs `nice_scale`).
function M.nice_scale(peak: number, target: number): (number, { number })
	if peak <= 0 or peak ~= peak or peak == math.huge then
		return 1, { 0 }
	end
	local raw = peak / target
	local magnitude = 10 ^ math.floor(math.log10(raw))
	local step = 10 * magnitude
	for _, m in { 1, 2, 5, 10 } do
		if m * magnitude >= raw then
			step = m * magnitude
			break
		end
	end
	local max = math.ceil(peak / step - 1e-9) * step
	local ticks = {}
	for i = 0, math.floor(max / step + 0.5) do
		table.insert(ticks, i * step)
	end
	return max, ticks
end

-- A tick's words: dollars, or tokens as the figures shorten them.
function M.tick_label(v: number, cost: boolean): string
	if cost then
		return if v == math.floor(v) then `${math.floor(v)}` else string.format("$%.2f", v)
	end
	for _, unit in { { 1e9, "B" }, { 1e6, "M" }, { 1e3, "K" } } do
		if v >= unit[1] then
			local n = v / unit[1]
			return (if n == math.floor(n) then tostring(math.floor(n)) else string.format("%.1f", n)) .. unit[2]
		end
	end
	return tostring(math.floor(v))
end

local function figure(text: string | () -> string): Node
	return ui.text { text, text_size = "$font.size.sm", line_height = "$font.line.sm", color = "$text.primary", tabular = true }
end

local function value_of(p: any): number
	return if metric:get() == "cost" then p.cost_n else p.tokens_n
end

local function text_of(item: any): string
	return if metric:get() == "cost" then item.cost else item.tokens
end

local function bar(now: State<any>, peak: () -> number, width: () -> number, hourly: () -> boolean): Node
	local function says(): string
		return `{M.period_label(now:get().start, hourly())}: {text_of(now:get())}`
	end
	return ui.column {
		w = width,
		shrink = 0,
		h = CHART_H,
		justify = "end",
		role = "img",
		label = says,
		tooltip = ds.Tooltip { says },
		ui.div {
			w_full = true,
			h = function(): number
				local v, p = value_of(now:get()), peak()
				if v <= 0 or p <= 0 then
					return 2
				end
				return math.max(CHART_H * 0.04, CHART_H * v / p)
			end,
			rounded = "$radius.sm",
			bg = function(): string
				return if value_of(now:get()) > 0 then "$accent" else "$zeron.secondary.bg"
			end,
		},
	}
end

local function provider_rows(): Node
	local list = ui.computed(function(): { any }
		local d = data:get()
		return if d and d.providers then d.providers else {}
	end)
	return ui.each(list, function(_p: any, _i: number, now: State<any>): Node
		local function cost(): boolean
			return metric:get() == "cost"
		end
		return ui.column {
			w_full = true,
			gap = ui.raw(2, "gpui"),
			ui.row {
				w_full = true,
				gap = ui.raw(12, "gpui"),
				items = "center",
				justify = "between",
				ui.row {
					gap = ui.raw(8, "gpui"),
					items = "center",
					min_w = 0,
					ui.div { w = 8, h = 8, shrink = 0, rounded = "$radius.full", bg = function(): string
						local h = now:get().harness
						local brand = BRAND[h]
						return if brand then ui.raw(brand, "Claude's brand, as gpui's sums") :: any else (if h == "grok" then "$text.secondary" else "$text.primary")
					end },
					ui.presence(function(): string
						return now:get().harness
					end, function(h: string): Node
						return ds.AgentMark { label = now:peek().name, seed = h, mark = MARKS[h], size = 16, bare = true }
					end),
					ui.text { function(): string
						return now:get().name
					end, literal = true, truncate = true, text_size = ui.raw(13, "gpui"), color = "$text.primary" },
					ui.text { function(): string
						return ui.t("settings.usage.sessions", { n = now:get().sessions }):get()
					end, text_size = ui.raw(11, "gpui"), color = "$text.secondary", wrap = false },
				},
				ui.text { function(): string
					return if cost() then now:get().cost else now:get().tokens
				end, shrink = 0, tabular = true, text_size = ui.raw(13, "gpui"), weight = "$font.weight.medium", color = "$text.primary" },
			},
			ui.text { function(): string
				local p = now:get()
				return if cost()
					then ui.t("settings.usage.provider_cost", { share = p.cost_share, tokens = p.tokens }):get()
					else ui.t("settings.usage.provider_tokens", { share = p.tokens_share, cost = p.cost }):get()
			end, text_size = ui.raw(12, "gpui"), color = "$text.secondary" },
		}
	end, {
		key = function(p: any): string
			return p.harness
		end,
		id = "usage-providers",
		w_full = true,
		gap = "$space.4",
	})
end

local function spend(): Node
	local periods = ui.computed(function(): { any }
		local d = data:get()
		return if d then d.periods else {}
	end)
	local function hourly(): boolean
		local d = data:get()
		return d ~= nil and d.hourly
	end
	local scale = ui.computed(function(): any
		local p = 0
		for _, period in periods:get() do
			p = math.max(p, value_of(period))
		end
		local max, ticks = M.nice_scale(p, TICKS)
		return { max = max, ticks = ticks }
	end)
	local function peak(): number
		return scale:get().max
	end
	local chart_w = ui.use_state(0)
	local GAP = (ui.theme.get("zeron.space.hair") :: number?) or 2
	local function width(): number
		local n = #periods:get()
		return if n == 0 then 0 else math.max(1, (chart_w:get() - GAP * (n - 1)) / n)
	end
	local function at(where: "first" | "middle" | "last"): string
		local list = periods:get()
		local p = list[if where == "first" then 1 elseif where == "last" then #list else math.floor(#list / 2) + 1]
		return if p then string.upper(M.period_label(p.start, hourly())) else ""
	end
	local function caption(text: () -> string): Node
		return ui.text { text, text_size = ui.raw(10, "gpui's axis"), color = "$text.tertiary" }
	end
	local function tick_y(v: number): number
		local max = scale:get().max
		return CHART_H - (if max > 0 then CHART_H * v / max else 0)
	end
	return ds.Settings.Section {
		title = ui.t("settings.usage.spend.title"),
		search.anchor(
			"spend",
			ds.Settings.Group {
				ui.column {
					w_full = true,
					-- (Not a Settings.Row: its words keep a width the three controls need.)
					ui.row {
						w_full = true,
						px = "$space.4",
						pt = "$space.4",
						pb = "$space.3",
						gap = "$space.3",
						items = "center",
						justify = "between",
						ui.column {
							grow = 1,
							min_w = 0,
							gap = ui.raw(4, "gpui"),
							ui.text {
								function(): string
									local d = data:get()
									if d then
										return text_of(d)
									end
									return if failed:get() then ui.t("settings.usage.failed"):get() else ui.t("settings.usage.reading"):get()
								end,
								id = "usage-headline",
								text_size = function(): any
									return if data:get() then ui.raw(36, "gpui's summary headline") else "$font.size.xl"
								end,
								line_height = function(): any
									return if data:get() then ui.raw(42, "gpui's summary headline") else "$font.line.xl"
								end,
								weight = "$font.weight.semibold",
								color = "$text.primary",
								tabular = true,
							},
							ui.text { function(): string
								local d = data:get()
								if not d then
									return " "
								end
								local n = ui.t("settings.usage.sessions", { n = d.sessions }):get()
								return if metric:get() == "cost" then ui.t("settings.usage.estimate", { sessions = n }):get() else n
							end, text_size = ui.raw(12, "gpui"), color = "$text.secondary" },
							ui.show(function(): boolean
								local d = data:get()
								return d ~= nil and d.unpriced > 0 and metric:get() == "cost"
							end, ui.text { function(): string
								local d = data:get()
								return ui.t("settings.usage.unpriced", { n = if d then d.unpriced else 0 }):get()
							end, text_size = ui.raw(12, "gpui"), color = "$text.secondary" }),
						},
						ui.row {
							gap = "$space.2",
							items = "center",
							shrink = 0,
							ds.Segmented {
								value = metric,
								label = ui.t("settings.usage.metric"),
								options = {
									{ value = "cost", label = ui.t("settings.usage.cost") },
									{ value = "tokens", label = ui.t("settings.usage.tokens") },
								},
							},
							ds.Select {
								value = days,
								name = ui.t("settings.usage.window"),
								size = "sm",
								w = 140,
								options = function(): { any }
									return {
										{ value = 1, label = ui.t("settings.usage.window_1"):get() },
										{ value = 7, label = ui.t("settings.usage.window_7"):get() },
										{ value = 30, label = ui.t("settings.usage.window_30"):get() },
										{ value = 90, label = ui.t("settings.usage.window_90"):get() },
									}
								end,
							},
							ui.div {
								id = "usage-refresh",
								opacity = function(): number
									return if loading:get() then 0.5 else 1
								end,
								ds.IconButton { icon = "refresh", label = ui.t("settings.usage.refresh"), on_click = load },
							},
						},
					},
					ui.column { w_full = true, px = "$space.4", pb = "$space.4", provider_rows() },
					ui.column {
						w_full = true,
						px = "$space.4",
						pb = "$space.4",
						gap = "$space.2",
						ui.text { function(): string
							local key = (if hourly() then "hourly_" else "daily_") .. metric:get()
							return ui.t(`settings.usage.chart.{key}`):get()
						end, id = "usage-chart-title", text_size = ui.raw(13, "gpui"), weight = "$font.weight.medium", color = "$text.primary" },
						ui.row {
							w_full = true,
							ui.div {
								id = "usage-axis",
								w = AXIS_W,
								h = CHART_H,
								shrink = 0,
								relative = true,
								ui.each(ui.computed(function(): { number }
									return scale:get().ticks
								end), function(v: number): Node
									return ui.text {
										M.tick_label(v, metric:peek() == "cost"),
										absolute = true,
										right = ui.raw(8, "gpui"),
										top = function(): number
											return tick_y(v) - 6
										end,
										text_size = ui.raw(10, "gpui's axis"),
										color = "$text.tertiary",
										tabular = true,
									}
								end, { key = function(v: number): string
									return `{metric:peek()}:{v}`
								end, w_full = true, h_full = true }),
							},
							ui.row {
								grow = 1,
								min_w = 0,
								h = CHART_H,
								relative = true,
								items = "end",
								on_resize = function(size: any)
									if math.abs(chart_w:peek() - size.w) >= 0.5 then
										chart_w:set(size.w)
									end
								end,
								ui.each(ui.computed(function(): { number }
									return scale:get().ticks
								end), function(v: number): Node
									return ui.div { absolute = true, left = 0, right = 0, h = 1, top = function(): number
										return tick_y(v)
									end, bg = "$border.subtle", pointer_events = "none" }
								end, { key = function(v: number): string
									return tostring(v)
								end, absolute = true, left = 0, right = 0, top = 0, bottom = 0, pointer_events = "none" }),
								ui.each(periods, function(_p: any, _i: number, now: State<any>): Node
									return bar(now, peak, width, hourly)
								end, {
									key = function(p: any): string
										return tostring(p.start)
									end,
									w_full = true,
									h_full = true,
									grow = 1,
									direction = "row",
									items = "end",
									gap = "$zeron.space.hair",
								}),
							},
						},
						ui.row {
							w_full = true,
							pl = ui.raw(AXIS_W, "gpui's value axis"),
							justify = "between",
							caption(function(): string
								return at("first")
							end),
							caption(function(): string
								return at("middle")
							end),
							caption(function(): string
								return at("last")
							end),
						},
					},
				},
			}
		),
	}
end

local function totals(): Node
	local function total(key: string, label: string): Node
		return ds.Settings.Row {
			label = ui.t(`settings.usage.totals.{label}`),
			control = figure(function(): string
				local d = data:get()
				return if d then d[key] else ""
			end),
		}
	end
	return ds.Settings.Section {
		title = ui.t("settings.usage.totals.title"),
		ds.Settings.Group {
			total("tokens", "processed"),
			total("cached", "cached"),
			total("uncached", "uncached"),
			total("output", "output"),
			total("savings", "savings"),
		},
	}
end

local function breakdown(): Node
	local rows = ui.computed(function(): { any }
		local d = data:get()
		if not d then
			return {}
		end
		if by:get() == "model" then
			return d.models
		end
		local list = {}
		for i = #d.periods, 1, -1 do
			local p = d.periods[i]
			if p.cost_n > 0 or p.tokens_n > 0 then
				table.insert(list, p)
			end
		end
		return list
	end)
	return ds.Settings.Section {
		title = ui.t("settings.usage.breakdown.title"),
		search.anchor(
			"breakdown",
			ds.Settings.Group {
				ds.Settings.Row {
					label = ui.t("settings.usage.breakdown.by"),
					control = ds.Segmented {
						value = by,
						label = ui.t("settings.usage.breakdown.by"),
						options = {
							{ value = "model", label = ui.t("settings.usage.breakdown.model") },
							{
								value = "period",
								label = function(): string
									local d = data:get()
									return if d and d.hourly then ui.t("settings.usage.breakdown.hour"):get() else ui.t("settings.usage.breakdown.day"):get()
								end,
							},
						},
					},
				},
			}
		),
		ui.show(function(): boolean
			return data:get() ~= nil and #rows:get() == 0
		end, ds.EmptyState { icon = "clock-circle", title = ui.t("settings.usage.empty"), size = "sm" }),
		ds.Settings.Each(rows, function(item: any, _i: number, now: State<any>): Node
			return ds.Settings.Row {
				label = function(): string
					local r = now:get()
					return r.name or M.period_label(r.start, data:get().hourly)
				end,
				description = if item.name
					then function(): string
						local r = now:get()
						local share = if metric:get() == "cost" then r.cost_share else r.tokens_share
						return `{HARNESS[r.harness] or r.harness} · {share}`
					end
					else nil,
				truncate = true,
				control = figure(function(): string
					return text_of(now:get())
				end),
			}
		end, {
			key = function(r: any): string
				return r.key or tostring(r.start)
			end,
		}),
	}
end

function M.view(): Node
	-- Read as the page opens and as the window changes (the bars keep the
	-- last window's until the new ones land).
	ui.effect(function()
		days:get()
		load()
	end)
	return ds.Settings.Page {
		title = ui.t("settings.usage.title"),
		description = ui.t("settings.usage.description"),
		spend(),
		totals(),
		breakdown(),
	}
end

return M
