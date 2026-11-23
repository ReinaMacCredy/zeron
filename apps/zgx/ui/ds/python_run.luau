local Icon = require("@self/icon")
local control = require("@self/control")
local status = require("@self/status")
local locale = require("@self/locale")
local Lightbox = require("@self/lightbox")

export type Result = {
	state: string,
	output: string?,
	cut: boolean?,
	figures: { string }?,
	ms: number?,
	exit: number?,
	error: string?,
}
export type Runner = {
	can: () -> boolean,
	result: (code: string) -> Result?,
	run: (code: string) -> (),
	stop: (code: string) -> (),
	clear: (code: string) -> (),
}

local M = {}

local LANGS: { [string]: boolean } = { python = true, py = true, python3 = true }

function M.runs(lang: string?): boolean
	return LANGS[string.lower(lang or "")] == true
end

local function ghost(props: { [any]: any }): Node
	props.h = 24
	props.px = "$space.2"
	props.gap = "$space.1"
	props.items = "center"
	props.rounded = ui.raw(control.nest("$radius.xl", 1, 5), "concentric with the code block, as Copy")
	props.bg = "#00000000"
	props.color = "$text.tertiary"
	props.text_size = "$font.size.xs"
	props.hover = { bg = "$hover", color = "$text.primary" }
	props.active = { bg = "$pressed" }
	props.focus = { border = 0 }
	return ui.button(props)
end

local function running(r: Result?): boolean
	return r ~= nil and r.state == "running"
end

function M.Controls(runner: Runner, code: string, live: (() -> boolean)?): Node
	local function now(): Result?
		return runner.result(code)
	end
	return ui.row {
		gap = "$space.1",
		items = "center",
		ui.show(function(): boolean
			local r = now()
			return r ~= nil and not running(r)
		end, ghost {
			id = "md-code-python-clear",
			label = locale.t("ds.python_run.clear"),
			on_click = function()
				runner.clear(code)
			end,
			ui.text(function()
				return locale.s("ds.python_run.clear")
			end),
		}),
		ui.show(function(): boolean
			return running(now())
		end, ghost {
			id = "md-code-python-stop",
			label = locale.t("ds.python_run.stop"),
			tooltip = locale.s("ds.python_run.stop"),
			on_click = function()
				runner.stop(code)
			end,
			Icon { name = "stop", size = 12 },
			ui.text(function()
				return locale.s("ds.python_run.stop_label")
			end),
		}),
		ui.show(function(): boolean
			return not running(now()) and runner.can() and not (live and live())
		end, ghost {
			id = "md-code-python-run",
			label = locale.t("ds.python_run.run"),
			tooltip = locale.s("ds.python_run.run"),
			on_click = function()
				runner.run(code)
			end,
			Icon { name = "action-play", size = 12 },
			ui.text(function()
				return locale.s("ds.markdown.run")
			end),
		}),
	}
end

local function seconds(ms: number): string
	if ms < 1000 then
		return `{math.floor(ms)} ms`
	end
	return `{string.format("%.1f", ms / 1000)} s`
end

local function said(r: Result): string
	if r.state == "running" then
		return locale.s("ds.python_run.running")
	elseif r.state == "stopped" then
		return locale.s("ds.python_run.stopped")
	elseif r.state == "timed_out" then
		return locale.s("ds.python_run.timed_out")
	elseif r.state == "failed" then
		return locale.s("ds.python_run.failed", { reason = r.error or "" })
	end
	return locale.s("ds.python_run.done", { time = seconds(r.ms or 0), code = tostring(r.exit or 0) })
end

local function dot(r: Result): Node
	local ink = if r.state == "running"
		then "$accent"
		elseif r.state == "done" and r.exit == 0 then "$success"
		elseif r.state == "done" or r.state == "timed_out" or r.state == "failed" then "$danger"
		else "$text.tertiary"
	local props = { w = 6, h = 6, shrink = 0, rounded = "$radius.full", bg = ink }
	if r.state ~= "running" then
		return ui.div(props)
	end
	return status.looping(props :: any, function(el)
		return anime.animate(el, { opacity = { 1, 0.45 }, duration = 500, ease = "inOutSine", loop = true, alternate = true })
	end, 1000)
end

local function figure(src: string, name: string): Node
	local ok, w, h = pcall(ui.image_size, src)
	local known = ok and type(w) == "number" and type(h) == "number" and w > 0 and h > 0
	local iw, ih = if known then w :: number else 0, if known then h :: number else 0
	local function open()
		Lightbox.open({ { src = src, name = name, w = iw, h = ih } }, 1, `python:{src}`)
	end
	return ui.div {
		w_full = true,
		max_w = ui.raw(if known then iw / 2 else 480, "a figure is saved at 2x"),
		aspect = if known then iw / ih else 1.5,
		cursor = "zoom_in",
		role = "button",
		label = locale.t("ds.markdown.view_named", { name = name }),
		focusable = true,
		on_click = open,
		ui.image { src = src, aria_hidden = true, w_full = true, h_full = true, fit = "contain" },
	}
end

local function body(r: Result): Node
	local column: { [any]: any } = { w_full = true, min_w = 0, border = 1, border_sides = { 1, 0, 0, 0 }, border_color = "$zeron.prose.code.edge" }
	local output = r.output or ""
	if output ~= "" or r.cut then
		local text = string.gsub(output, "\n$", "")
		if r.cut then
			text ..= "\n" .. locale.s("ds.python_run.cut")
		end
		table.insert(column, ui.scroll_x {
			w_full = true,
			fade_x = { 16, 32 },
			fade_overflow = true,
			ui.text {
				text,
				id = "md-code-python-output",
				literal = true,
				font = "$font.mono",
				text_size = "$zeron.prose.code.size",
				line_height = "$zeron.prose.code.line",
				color = "$text.secondary",
				pl = "$space.4",
				pr = "$space.5",
				pt = "$space.3",
			},
		})
	end
	for i, src in r.figures or {} do
		local name = locale.s("ds.python_run.figure", { n = tostring(i) })
		table.insert(column, ui.column {
			w_full = true,
			min_w = 0,
			px = "$space.4",
			pt = "$space.3",
			gap = "$space.1",
			figure(src, name),
			ui.text { name, literal = true, text_size = "$font.size.xs", color = "$text.tertiary" },
		})
	end
	table.insert(column, ui.column {
		id = "md-code-python-status",
		w_full = true,
		min_w = 0,
		gap = "$space.1",
		px = "$space.4",
		pt = "$space.3",
		pb = "$space.3",
		text_size = "$font.size.xs",
		color = "$text.tertiary",
		ui.row { items = "center", gap = "$space.2", dot(r), ui.text { said(r), literal = true, wrap = true, min_w = 0 } },
		ui.text { locale.s("ds.python_run.sandbox"), literal = true, wrap = true, w_full = true, min_w = 0 },
	})
	return ui.column(column)
end

function M.Output(runner: Runner, code: string): Node
	local shown: { [string]: Result } = {}
	return ui.presence(function(): string?
		local r = runner.result(code)
		if not r then
			return nil
		end
		local key = `{r.state}:{r.ms or ""}:{r.exit or ""}:{#(r.output or "")}:{#(r.figures or {})}:{r.error or ""}`
		shown = { [key] = r }
		return key
	end, function(key: string): Node
		return body(shown[key])
	end, { w_full = true, min_w = 0 })
end

return M
