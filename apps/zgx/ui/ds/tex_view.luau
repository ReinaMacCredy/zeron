local PdfView = require("@self/pdf_view")
local AnnotationLayer = require("@self/annotation_layer")
local Icon = require("@self/icon")
local FadeText = require("@self/fade_text")
local locale = require("@self/locale")

local M = {}

local SETTLE = 400
local SHOWN = 4
local LINE = 16

export type Problem = { message: string, line: number?, file: string? }
export type Way = { label: Label, on_click: () -> (), primary: boolean? }
export type Props = {
	path: string,
	label: string,
	saved: () -> string,
	zoom_label: State<string>,
	page_label: State<string>,
	commands: { [string]: () -> () },
	failed: (reason: string, again: () -> ()) -> Node,
	notice: (icon: Icon.Name, title: () -> string, body: () -> string, ways: { Way }?) -> Node,
	on_line: (file: string, line: number) -> (),
	annotate: ((source: string) -> any)?,
}

M.compile = function(path: string): any
	return ui.call("tex.compile", { path = path })
end
M.sync = function(at: { [string]: any }): any
	return ui.call("tex.sync", at)
end

local function base_name(path: string): string
	return string.match(path, "[^/]+$") or path
end

local function reason(err: any): string
	local why = if type(err) == "table" then err.message or err.error elseif err ~= nil then tostring(err) else nil
	return if type(why) == "string" then why else ""
end

local function by_line(annotate: (source: string) -> any, path: string): (any, (source: string) -> ())
	local base = annotate(path)
	local made: { [string]: any } = {}
	local source = path
	local function now(): any
		local n = made[source]
		if not n then
			n = annotate(source)
			made[source] = n
		end
		return n
	end
	local notes = {
		list = function(): { any }
			local out: { any } = {}
			local seen: { [string]: boolean } = {}
			local function take(n: any)
				for _, a in n.list() do
					if not seen[a.id] then
						seen[a.id] = true
						table.insert(out, a)
					end
				end
			end
			take(base)
			for _, n in made do
				take(n)
			end
			table.sort(out, function(a: any, b: any): boolean
				return a.n < b.n
			end)
			return out
		end,
		add = function(text: string, anchor: any): string?
			return now().add(text, anchor)
		end,
		comment = base.comment,
		remove = base.remove,
		keys = base.keys,
		edit = base.edit,
		edited = base.edited,
		side = if base.side
			then function(text: string)
				now().side(text)
			end
			else nil,
		mention = if base.mention
			then function(q: any)
				now().mention(q)
			end
			else nil,
	}
	return notes, function(s: string)
		source = s
	end
end

local function problems_bar(props: Props, result: State<any>): Node
	local listed = ui.computed(function(): { any }
		local r = result:get()
		local out: { any } = {}
		for i, p in r.problems or {} do
			if i > SHOWN then
				break
			end
			table.insert(out, { key = `{i}\0{p.file or ""}\0{p.line or ""}\0{p.message}`, message = p.message, line = p.line, file = p.file or r.root or props.path })
		end
		return out
	end)
	local function count(): number
		return #(result:get().problems or {})
	end
	local function row(item: any): Node
		local line: number? = item.line
		return ui.button {
			w_full = true,
			h = 24,
			px = "$space.2",
			gap = "$space.2",
			items = "center",
			rounded = "$radius.md",
			bg = "#00000000",
			hover = if line then { bg = "$hover" } else nil,
			active = if line then { bg = "$pressed" } else nil,
			cursor = if line then "pointer" else "default",
			label = if line then locale.t("ds.tex_view.go_to_line", { line = line }) else item.message,
			on_click = function()
				if line then
					props.on_line(item.file, line)
				end
			end,
			ui.text {
				if line then locale.t("ds.tex_view.line", { line = line }) else "",
				shrink = 0,
				font = "$font.mono",
				text_size = "$font.size.xs",
				color = "$zeron.tone.danger.ink",
			},
			FadeText { item.message, grow = 1, min_w = 0, literal = true, text_size = "$font.size.sm", color = "$text.primary" },
		}
	end
	return ui.show(
		function(): boolean
			local st = result:get().state
			return count() > 0 or st == "timeout"
		end,
		ui.column {
			id = "tex-problems",
			role = "alert",
			w_full = true,
			shrink = 0,
			p = "$space.2",
			gap = "$zeron.space.hair",
			bg = "$zeron.tone.danger.wash",
			border_sides = { 0, 0, 1, 0 },
			border_color = "$zeron.tone.danger.edge",
			ui.row {
				px = "$space.2",
				gap = "$space.2",
				items = "center",
				ui.div { color = "$zeron.tone.danger.ink", Icon { name = "danger-triangle", size = 14 } },
				ui.text {
					function(): string
						if result:get().state == "timeout" then
							return locale.s("ds.tex_view.timeout")
						end
						return locale.s("ds.tex_view.errors", { n = count() })
					end,
					literal = true,
					text_size = "$font.size.sm",
					weight = "$font.weight.medium",
					color = "$text.primary",
				},
			},
			ui.each(listed, row, {
				key = function(item: any): string
					return item.key
				end,
				w_full = true,
			}),
			ui.show(
				function(): boolean
					return count() > SHOWN
				end,
				ui.text {
					function(): string
						return locale.s("ds.tex_view.more", { n = count() - SHOWN })
					end,
					literal = true,
					px = "$space.2",
					text_size = "$font.size.xs",
					color = "$text.tertiary",
				}
			),
		}
	)
end

M.View = ui.component(function(props: Props): Node
	local name = base_name(props.path)
	local pdf = ui.use_state(nil :: string?)
	local busy = ui.use_state(false)
	local result = ui.use_state({ state = "", problems = {} } :: any)
	local asked = 0
	local function run()
		asked += 1
		local mine = asked
		busy:set(true)
		M.compile(props.path):after(function(r: any)
			if mine ~= asked then
				return
			end
			ui.batch(function()
				busy:set(false)
				if r.state ~= "superseded" then
					result:set(r)
					if r.pdf then
						pdf:set(r.pdf)
					end
				end
			end)
		end, function(err: any)
			if mine ~= asked then
				return
			end
			ui.batch(function()
				busy:set(false)
				result:set({ state = "failed", problems = { { message = reason(err) } } })
			end)
		end)
	end
	local first = true
	ui.effect(function()
		props.saved()
		if first then
			first = false
			run()
		else
			ui.after(SETTLE, run)
		end
	end)

	local view = ui.ref()
	local scrolled = ui.use_state(0)
	local pages: { [number]: Ref } = {}
	local layer: any = nil
	local pick_source: (string) -> () = function() end
	if props.annotate then
		local notes
		notes, pick_source = by_line(props.annotate, props.path)
		layer = AnnotationLayer.new {
			name = `tex.{props.path}`,
			notes = notes,
			view = view,
			holder_at = function(y: number): (string?, Ref?)
				for i, ref in pages do
					local el = ref:peek()
					if el and el.y and el.height and y >= el.y and y < el.y + el.height then
						return `page.{i}`, ref
					end
				end
				return nil, nil
			end,
			holder = function(key: string): Ref?
				return pages[tonumber(string.sub(key, 6)) or 0]
			end,
			scroll = function(): number
				return scrolled:get()
			end,
		}
	end
	local function pick(p: PdfView.Pick)
		local shown = pdf:peek()
		if not shown or not layer then
			return
		end
		M.sync({ pdf = shown, page = p.page, x = p.x, y = p.y, x2 = p.x2, y2 = p.y2 }):after(function(r: any)
			if pdf:peek() ~= shown or type(r.text) ~= "string" or not string.find(r.text, "%S") then
				return
			end
			pick_source(if r.end_line > r.line then `{r.file}:{r.line}-{r.end_line}` else `{r.file}:{r.line}`)
			local b = p.box
			local tall = math.max(b.h, LINE)
			local at = { x = b.x, y = b.y + b.h / 2 - tall / 2, w = math.max(b.w, 1), h = tall }
			layer.offer({ text = r.text, top = at, bottom = at })
		end, function() end)
	end

	local function shows(): string
		local st = result:get().state
		if st == "missing" or st == "not_document" or st == "no_root" then
			return st
		end
		if pdf:get() then
			return "pdf"
		end
		return if st == "" then "waiting" else "no_pdf"
	end
	local body = ui.presence(shows, function(which: string): Node
		if which == "pdf" then
			return ui.overlay {
				ref = view,
				w_full = true,
				h_full = true,
				relative = true,
				PdfView.View {
					path = function(): string
						return pdf:get() or ""
					end,
					label = props.label,
					zoom_label = props.zoom_label,
					page_label = props.page_label,
					commands = props.commands,
					failed = props.failed,
					offset = scrolled,
					on_pick = if layer then pick else nil,
					page_layer = if layer
						then function(i: number, ref: Ref): Node?
							pages[i] = ref
							ui.on_cleanup(function()
								if pages[i] == ref then
									pages[i] = nil
								end
							end)
							return layer.Here(`page.{i}`)
						end
						else nil,
				},
				if layer then layer.Overlay() else nil,
			}
		elseif which == "missing" then
			return props.notice("document", function(): string
				return locale.s("ds.tex_view.missing.title", { name = name })
			end, function(): string
				return locale.s("ds.tex_view.missing.body")
			end, { { label = locale.t("common.try_again"), on_click = run } })
		elseif which == "not_document" then
			return props.notice("document", function(): string
				return locale.s("ds.tex_view.not_document.title", { name = name })
			end, function(): string
				return locale.s("ds.tex_view.not_document.body")
			end)
		elseif which == "no_root" then
			return props.notice("danger-triangle", function(): string
				return locale.s("ds.tex_view.no_root.title")
			end, function(): string
				return locale.s("ds.tex_view.no_root.body", { root = base_name(result:peek().root or "") })
			end)
		elseif which == "no_pdf" then
			return props.notice("danger-triangle", function(): string
				return locale.s("ds.tex_view.no_pdf.title", { name = name })
			end, function(): string
				return locale.s("ds.tex_view.no_pdf.body")
			end, { { label = locale.t("common.try_again"), on_click = run } })
		end
		return props.notice("document", function(): string
			return locale.s("ds.tex_view.waiting.title", { name = name })
		end, function(): string
			return locale.s("ds.tex_view.waiting.body")
		end)
	end, { w_full = true, h_full = true })

	return ui.column {
		w_full = true,
		h_full = true,
		problems_bar(props, result),
		ui.overlay {
			w_full = true,
			grow = 1,
			h = 0,
			relative = true,
			body,
			ui.show(
				busy,
				ui.row {
					id = "tex-compiling",
					role = "status",
					absolute = true,
					top = "$space.2",
					right = "$space.2",
					h = 24,
					px = "$space.2",
					gap = "$space.1",
					items = "center",
					rounded = "$radius.full",
					bg = "$surface.raised",
					border = 1,
					border_color = "$border.subtle",
					shadow = "$shadow.sm",
					pointer_events = "none",
					ui.div { color = "$text.tertiary", Icon { name = "refresh", size = 12 } },
					ui.text { locale.t("ds.tex_view.compiling"), text_size = "$font.size.xs", color = "$text.secondary" },
				}
			),
		},
	}
end)

return M
