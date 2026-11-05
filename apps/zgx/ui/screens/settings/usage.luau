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
local HARNESS: { [string]: string } = { ["claude-code"] = "Claude Code", codex = "Codex", grok = "Grok" }

local days = ui.state("settings_usage_days", 30)
local metric = ui.state("settings_usage_metric", "cost")
local by = ui.state("settings_usage_by", "model")
local data = ui.state("settings_usage_data", nil :: any)
local failed = ui.state("settings_usage_failed", false)

local function load()
	local asked = days:peek()
	failed:set(false)
	usage.summary(asked):after(function(summary: any)
		if days:peek() == asked then
			data:set(summary)
		end
	end, function()
		if days:peek() == asked then
			failed:set(data:peek() == nil)
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

local function spend(): Node
	local periods = ui.computed(function(): { any }
		local d = data:get()
		return if d then d.periods else {}
	end)
	local function hourly(): boolean
		local d = data:get()
		return d ~= nil and d.hourly
	end
	local function peak(): number
		local p = 0
		for _, period in periods:get() do
			p = math.max(p, value_of(period))
		end
		return p
	end
	local chart_w = ui.use_state(0)
	local GAP = (ui.theme.get("zeron.space.hair") :: number?) or 2
	local function width(): number
		local n = #periods:get()
		return if n == 0 then 0 else math.max(1, (chart_w:get() - GAP * (n - 1)) / n)
	end
	local function edge(last: boolean): string
		local list = periods:get()
		local p = list[if last then #list else 1]
		return if p then M.period_label(p.start, hourly()) else ""
	end
	local function caption(text: () -> string): Node
		return ui.text { text, text_size = "$font.size.xs", line_height = "$font.line.xs", color = "$text.tertiary" }
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
							ui.text {
								function(): string
									local d = data:get()
									if d then
										return text_of(d)
									end
									return if failed:get() then ui.t("settings.usage.failed"):get() else ui.t("settings.usage.reading"):get()
								end,
								text_size = "$font.size.xl",
								line_height = "$font.line.xl",
								weight = "$font.weight.semibold",
								color = "$text.primary",
								tabular = true,
							},
							caption(function(): string
								local d = data:get()
								if not d then
									return " "
								end
								local line = ui.t("settings.usage.sessions", { n = d.sessions }):get()
								if d.unpriced > 0 then
									line ..= ` · {ui.t("settings.usage.unpriced", { n = d.unpriced }):get()}`
								end
								return line
							end),
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
							ds.IconButton { icon = "refresh", label = ui.t("settings.usage.refresh"), on_click = load },
						},
					},
					ui.column {
						w_full = true,
						px = "$space.4",
						pb = "$space.4",
						gap = "$space.2",
						ui.row {
							w_full = true,
							h = CHART_H,
							items = "end",
							on_resize = function(size: any)
								if math.abs(chart_w:peek() - size.w) >= 0.5 then
									chart_w:set(size.w)
								end
							end,
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
						ui.row {
							w_full = true,
							justify = "between",
							caption(function(): string
								return edge(false)
							end),
							caption(function(): string
								return edge(true)
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
