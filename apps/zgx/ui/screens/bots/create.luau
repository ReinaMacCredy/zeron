local ds = require("@ds")
local locale = require("@ds/locale")
local motion = require("@ds/motion")
local avatar = require("@app/screens/bots/avatar")
local store = require("@app/screens/bots/store")
local templates = require("@app/screens/bots/templates")
local chat_store = require("@app/screens/chat/store")

local M = {}

local W, H, RADIUS = 780, 488, 20
local SCRIM_MS, IN_MS, OUT_MS, STEP_MS = 180, 220, 140, 180
local EASE = "cubicBezier(.2, .8, .2, 1)"
local LEFT_W = 372

local asked = ui.state("bots_create_asked", "")

function M.open(from: string)
	asked:set(from)
end

local function window_size(): { width: number, height: number }
	local window = (ui :: any).window
	local ok, v = pcall(function(): any
		return window.size:get()
	end)
	local w = if ok and type(v) == "table" and type(v.width) == "number" then v.width else 0
	local h = if ok and type(v) == "table" and type(v.height) == "number" then v.height else 0
	return { width = math.max(1, w), height = math.max(1, h) }
end

local function trimmed(s: string): string
	return string.match(s, "^%s*(.-)%s*$") or ""
end

local function arrives(from: { [string]: any }, ms: number): (any) -> ()
	return function(el: any)
		local d = motion.ms(ms)
		if d > 0 then
			from.duration = d
			from.ease = "outQuad"
			anime.animate(el, from)
		end
	end
end

type Draft = {
	step: State<number>,
	back: State<boolean>,
	moved: State<boolean>,
	template: State<string>,
	query: State<string>,
	tab: State<string>,
	name: State<string>,
	glyph: State<string>,
	frame: State<string>,
	color: State<string>,
	image: State<string?>,
	said: State<string?>,
	busy: State<boolean>,
}

local function look_of(d: Draft): () -> avatar.Look
	return function(): avatar.Look
		return { glyph = d.glyph:get(), frame = d.frame:get(), color = d.color:get(), image = d.image:get() }
	end
end

local function Preview(d: Draft): Node
	local function heading(text: any): Node
		return ui.text {
			text,
			role = "heading",
			mt = ui.raw(26, "the mockup's preview"),
			text_size = ui.raw(20, "the mockup's preview"),
			weight = "$font.weight.semibold",
			color = "$text.primary",
			align = "center",
		}
	end
	local function blurb(text: any): Node
		return ui.text {
			text,
			mt = ui.raw(8, "the mockup's preview"),
			w = ui.raw(300, "the mockup's preview"),
			text_size = ui.raw(13.5, "the mockup's preview"),
			line_height = ui.raw(20, "the mockup's preview"),
			color = "$text.secondary",
			align = "center",
			wrap = true,
		}
	end
	local pop = arrives({ opacity = { 0, 1 }, scale = { 0.92, 1 } }, STEP_MS)
	local seen = 0
	local cluster = { { "yellow", "arch", 4, 6 }, { "green", "square", 112, 0 }, { "rust", "squircle", 8, 56 }, { "forest", "circle", 66, 32 }, { "violet", "leaf", 134, 58 } }
	return ui.column {
		id = "bots-create-preview",
		relative = true,
		w = LEFT_W,
		h_full = true,
		shrink = 0,
		items = "center",
		pt = ui.raw(110, "the mockup's preview"),
		rounded = ui.raw(14, "the mockup's preview"),
		bg = "$zeron.secondary.bg",
		ui.div { absolute = true, left = 14, top = 14, w = 12, h = 12, rounded = "$radius.md", bg = "$zeron.dialog.solid" },
		ui.div { absolute = true, right = 14, top = 14, w = 12, h = 12, rounded = "$radius.md", bg = "$zeron.dialog.solid" },
		ui.presence(function(): string
			return if d.step:get() == 1 then `1:{d.template:get()}` else "2"
		end, function(key: string): Node
			local step = d.step:peek()
			local pop = if step == 1 and seen == 1 then pop else nil
			seen = step
			if key == "2" then
				local t = templates.find(d.template:peek())
				return ui.column {
					items = "center",
					ui.div { mt = "$space.1", on_mount = pop, avatar.Avatar { id = "bots-create-avatar", look = look_of(d), size = 110 } },
					ui.input {
						id = "bots-create-name",
						label = ui.t("bots.create.name"),
						placeholder = ui.t("bots.create.name"),
						placeholder_color = "$text.tertiary",
						value = function(): string
							return d.name:get()
						end,
						on_change = function(v: string)
							d.name:set(string.sub(v, 1, 80))
						end,
						mt = ui.raw(22, "the mockup's preview"),
						mb = "$space.1",
						w = ui.raw(260, "the mockup's preview"),
						h = ui.raw(34, "the mockup's preview"),
						px = "$space.1",
						align = "center",
						text_size = ui.raw(20, "the mockup's preview"),
						weight = "$font.weight.semibold",
						color = "$text.primary",
						rounded = ui.raw(8, "the mockup's preview"),
						border = 0,
						bg = "#00000000",
						hover = { bg = "$hover" },
						focus = { border = 0, outline = 0, bg = "$hover" },
					},
					blurb(ui.t(if t.id == templates.SCRATCH then t.desc else t.desc)),
				}
			end
			local t = templates.find(d.template:peek())
			if t.id == templates.SCRATCH then
				local box: { [any]: any } = { relative = true, w = 200, h = 118, on_mount = pop }
				for _, c in cluster do
					table.insert(box, ui.div { absolute = true, left = c[3], top = c[4], avatar.Avatar { look = { glyph = "bot", color = c[1], frame = c[2] }, size = 58 } })
				end
				return ui.column { items = "center", ui.div(box), heading(ui.t("bots.create.custom")), blurb(ui.t("bots.create.custom_blurb")) }
			end
			return ui.column {
				items = "center",
				ui.div { mt = "$space.1", on_mount = pop, avatar.Avatar { look = { glyph = t.glyph, color = t.color, frame = "squircle" }, size = 110 } },
				heading(ui.t(t.name)),
				blurb(ui.t(t.desc)),
			}
		end, { items = "center" }),
	}
end

local function pill(props: { [any]: any }): Node
	props.size = "sm"
	props.shape = "pill"
	props.h = ui.raw(32, "the mockup's dialog button")
	props.px = ui.raw(14, "the mockup's dialog button")
	return ds.Button(props)
end

local function foot(...: Node): Node
	return ui.row { w_full = true, shrink = 0, justify = "end", gap = "$space.2", pt = ui.raw(14, "the mockup's dialog foot"), ... }
end

local function Goal(d: Draft, next: () -> ()): Node
	local rows = ui.computed(function(): { templates.Template }
		local q = string.lower(d.query:get())
		local out = {}
		for _, t in templates.LIST do
			local words = string.lower(locale.s(t.name) .. " " .. locale.s(t.tag))
			if q == "" or string.find(words, q, 1, true) then
				table.insert(out, t)
			end
		end
		return out
	end)
	return ui.column {
		w_full = true,
		grow = 1,
		min_h = 0,
		ui.column {
			w_full = true,
			grow = 1,
			min_h = 0,
			rounded = ui.raw(12, "the mockup's template list"),
			border = 1,
			border_color = "$border.strong",
			ui.row {
				w_full = true,
				h = ui.raw(34, "the mockup's template search"),
				shrink = 0,
				px = ui.raw(10, "the mockup's template search"),
				gap = "$space.2",
				items = "center",
				border_sides = { 0, 0, 1, 0 },
				border_color = "$border.strong",
				color = "$text.secondary",
				ds.Icon { name = "search", size = 15 },
				ui.input {
					id = "bots-create-search",
					grow = 1,
					min_w = 0,
					h = 20,
					px = 0,
					border = 0,
					bg = "#00000000",
					focus = { border = 0, outline = 0 },
					label = ui.t("bots.create.search"),
					placeholder = ui.t("bots.create.search"),
					placeholder_color = "$text.tertiary",
					text_size = ui.raw(13.5, "the mockup's template search"),
					color = "$text.primary",
					value = function(): string
						return d.query:get()
					end,
					on_change = function(v: string)
						d.query:set(v)
					end,
				},
			},
			ui.scroll {
				id = "bots-create-templates",
				role = "listbox",
				label = ui.t("bots.create.templates"),
				w_full = true,
				grow = 1,
				min_h = 0,
				p = "$space.1",
				ui.each(rows, function(t: templates.Template): Node
					local function on(): boolean
						return d.template:get() == t.id
					end
					return ui.button {
						id = `bots-create-template-{t.id}`,
						role = "option",
						selected = on,
						w_full = true,
						h = ui.raw(32, "the mockup's template row"),
						shrink = 0,
						px = "$space.2",
						gap = ui.raw(10, "the mockup's template row"),
						items = "center",
						rounded = ui.raw(7, "the mockup's template row"),
						cursor = "pointer",
						text_size = ui.raw(13.5, "the mockup's template row"),
						bg = function(): string
							return if on() then "$zeron.selected.flat.bg" else "#00000000"
						end,
						hover = { bg = "$hover" },
						on_click = function()
							d.template:set(t.id)
						end,
						if t.id == templates.SCRATCH
							then ui.div { w = 17, h = 17, items = "center", justify = "center", color = "$text.primary", ds.Icon { name = "plus", size = 16 } }
							else avatar.Avatar { look = { glyph = t.glyph, color = t.color, frame = "squircle" }, size = 17 },
						ui.text { ui.t(t.name), shrink = 0, weight = "$font.weight.medium", color = "$text.primary" },
						ui.text { ui.t(t.tag), grow = 1, min_w = 0, truncate = true, text_size = ui.raw(12.5, "the mockup's template row"), color = "$text.secondary" },
					}
				end, {
					key = function(t: templates.Template): string
						return t.id
					end,
				}),
				ui.show(function(): boolean
					return #rows:get() == 0
				end, ui.text { id = "bots-create-no-templates", ui.t("bots.create.no_templates"), w_full = true, align = "center", py = "$space.5", text_size = "$font.size.sm", color = "$text.tertiary" }),
			},
		},
		foot(pill { ui.t("bots.create.next"), id = "bots-create-next", tone = "primary", on_click = next }),
	}
end

local function Seg(id: string, word: string, on: () -> boolean, pick: () -> ()): Node
	return ui.button {
		id = id,
		role = "tab",
		selected = on,
		h = ui.raw(28, "the mockup's dialog tab"),
		px = ui.raw(11, "the mockup's dialog tab"),
		items = "center",
		rounded = ui.raw(8, "the mockup's dialog tab"),
		cursor = "pointer",
		text_size = ui.raw(13.5, "the mockup's dialog tab"),
		bg = function(): string
			return if on() then "$zeron.secondary.bg" else "#00000000"
		end,
		color = function(): string
			return if on() then "$text.primary" else "$text.secondary"
		end,
		hover = { color = "$text.primary" },
		on_click = pick,
		ui.text { ui.t(word) },
	}
end

local GLYPH_COLUMNS = 10

local function Icons(d: Draft): Node
	local grid: { [any]: any } = { id = "bots-create-glyphs", role = "radiogroup", label = ui.t("bots.create.glyphs"), w_full = true, max_h = ui.raw(138, "the mockup's glyph grid"), gap = "$space.1" }
	for i = 1, #avatar.GLYPHS, GLYPH_COLUMNS do
		local row: { [any]: any } = { w_full = true, gap = "$space.1", shrink = 0 }
		for j = i, i + GLYPH_COLUMNS - 1 do
			local g = avatar.GLYPHS[j]
			if g then
				local function on(): boolean
					return d.image:get() == nil and d.glyph:get() == g
				end
				table.insert(row, ui.button {
					id = `bots-create-glyph-{g}`,
					role = "radio",
					checked = on,
					label = g,
					basis = 0,
					grow = 1,
					min_w = 0,
					px = 0,
					h = ui.raw(30, "the mockup's glyph"),
					items = "center",
					justify = "center",
					rounded = ui.raw(7, "the mockup's glyph"),
					cursor = "pointer",
					bg = function(): string
						return if on() then "$text.primary" else "#00000000"
					end,
					color = function(): string
						return if on() then "$surface.overlay" else "$text.primary"
					end,
					hover = { bg = "$hover" },
					on_click = function()
						ui.batch(function()
							d.glyph:set(g)
							d.image:set(nil)
						end)
					end,
					ds.Icon { name = g, size = 17 },
				})
			else
				table.insert(row, ui.div { basis = 0, grow = 1 })
			end
		end
		table.insert(grid, ui.row(row))
	end
	local frames: { [any]: any } = {
		id = "bots-create-frames",
		role = "radiogroup",
		label = ui.t("bots.create.frames"),
		w_full = true,
		shrink = 0,
		justify = "between",
		mt = ui.raw(10, "the mockup's frames"),
		pt = ui.raw(14, "the mockup's frames"),
		px = ui.raw(10, "the mockup's frames"),
		pb = "$space.1",
		border_sides = { 1, 0, 0, 0 },
		border_color = "$border.strong",
	}
	for _, f in avatar.FRAMES do
		local function on(): boolean
			return d.frame:get() == f
		end
		local c = avatar.corners(f, 26)
		table.insert(frames, ui.button {
			id = `bots-create-frame-{f}`,
			role = "radio",
			checked = on,
			label = f,
			w = 40,
			h = 40,
			items = "center",
			justify = "center",
			rounded = ui.raw(9, "the mockup's frame"),
			cursor = "pointer",
			bg = function(): string
				return if on() then "$hover" else "#00000000"
			end,
			hover = { bg = "$hover" },
			on_click = function()
				d.frame:set(f)
			end,
			ui.div {
				w = 26,
				h = 26,
				min_w = 26,
				shrink = 0,
				rounded_tl = ui.raw(c[1], "the frame's corner"),
				rounded_tr = ui.raw(c[2], "the frame's corner"),
				rounded_br = ui.raw(c[3], "the frame's corner"),
				rounded_bl = ui.raw(c[4], "the frame's corner"),
				border = 3,
				border_color = ui.raw(avatar.hex("violet"), "the mockup's frame swatch"),
				bg = function(): any
					return if on() then ui.raw("#111111", "the avatar's screen") else "#00000000"
				end,
			},
		})
	end
	local colors: { [any]: any } = {
		id = "bots-create-colors",
		role = "radiogroup",
		label = ui.t("bots.create.colors"),
		w_full = true,
		shrink = 0,
		justify = "between",
		items = "center",
		pt = ui.raw(11, "the mockup's colors"),
	}
	for _, name in avatar.COLORS do
		local function on(): boolean
			return d.color:get() == name
		end
		table.insert(colors, ui.button {
			id = `bots-create-color-{name}`,
			role = "radio",
			checked = on,
			label = name,
			w = 28,
			h = 28,
			p = 0,
			items = "center",
			justify = "center",
			rounded = "$radius.full",
			cursor = "pointer",
			bg = "#00000000",
			hover = { bg = "#00000000" },
			border = 2,
			border_color = function(): string
				return if on() then "$text.primary" else "#00000000"
			end,
			on_click = function()
				d.color:set(name)
			end,
			ui.div { w = 20, h = 20, rounded = "$radius.full", bg = ui.raw(avatar.hex(name), "a Bot colour") },
		})
	end
	return ui.column { w_full = true, grow = 1, min_h = 0, ui.scroll(grid), ui.row(frames), ui.row(colors) }
end

local function Upload(d: Draft): Node
	local function take(files: { any })
		local f = files[1]
		if not f then
			return
		end
		d.said:set(nil)
		store.image(f.path):after(function(url: any)
			if type(url) == "string" then
				d.image:set(url)
			end
		end, function(err: any)
			local why = if type(err) == "table" or type(err) == "userdata" then tostring(err.message) else tostring(err)
			d.said:set(if string.find(why, "too_large", 1, true) then "bots.create.too_large" else "bots.create.invalid")
		end)
	end
	return ui.column {
		id = "bots-create-drop",
		w_full = true,
		grow = 1,
		min_h = 0,
		items = "center",
		justify = "center",
		gap = ui.raw(6, "the mockup's drop zone"),
		rounded = ui.raw(12, "the mockup's drop zone"),
		border = 1,
		border_dash = { 4, 3 },
		border_color = "$border.strong",
		color = "$text.secondary",
		on_file_drop = take,
		ds.Icon { name = "gallery", size = 20 },
		ui.text { ui.t("bots.create.drop"), weight = "$font.weight.medium", text_size = ui.raw(13.5, "the mockup's drop zone"), color = "$text.primary" },
		ui.text { ui.t("bots.create.kinds"), text_size = "$font.size.xs", color = "$text.secondary" },
		ui.div {
			mt = ui.raw(6, "the mockup's drop zone"),
			ds.Button {
				ui.t("bots.create.choose"),
				id = "bots-create-choose",
				tone = "secondary",
				size = "sm",
				on_click = function()
					ui.open_files({
						title = locale.s("bots.create.choose"),
						filters = { { locale.s("bots.create.images"), { "png", "jpg", "jpeg", "gif", "webp" } } },
					}):after(take)
				end,
			},
		},
		ui.show(function(): boolean
			return d.said:get() ~= nil
		end, ui.text {
			id = "bots-create-said",
			function(): string
				return locale.s(d.said:get() or "bots.create.invalid")
			end,
			role = "alert",
			mt = "$space.1",
			text_size = "$font.size.xs",
			color = "$danger",
		}),
	}
end

local function Customize(d: Draft, back: () -> (), make: () -> ()): Node
	return ui.column {
		w_full = true,
		grow = 1,
		min_h = 0,
		ui.row {
			role = "tablist",
			label = ui.t("bots.create.customize"),
			w_full = true,
			shrink = 0,
			items = "center",
			gap = ui.raw(2, "the mockup's dialog tabs"),
			mb = ui.raw(10, "the mockup's dialog tabs"),
			Seg("bots-create-tab-icons", "bots.create.icons", function(): boolean
				return d.tab:get() == "icons"
			end, function()
				d.tab:set("icons")
			end),
			Seg("bots-create-tab-upload", "bots.create.upload", function(): boolean
				return d.tab:get() == "upload"
			end, function()
				d.tab:set("upload")
			end),
			ui.div { grow = 1 },
			ds.IconButton {
				id = "bots-create-shuffle",
				icon = "refresh",
				label = ui.t("bots.create.shuffle"),
				size = "sm",
				tone = "quiet",
				on_click = function()
					ui.batch(function()
						d.glyph:set(avatar.GLYPHS[math.random(1, #avatar.GLYPHS)])
						d.color:set(avatar.COLORS[math.random(1, #avatar.COLORS)])
						d.frame:set(avatar.FRAMES[math.random(1, #avatar.FRAMES)])
						d.image:set(nil)
					end)
				end,
			},
		},
		ui.presence(function(): string
			return d.tab:get()
		end, function(tab: string): Node
			return if tab == "upload" then Upload(d) else Icons(d)
		end, { w_full = true, grow = 1, min_h = 0 }),
		ui.show(function(): boolean
			return d.said:get() ~= nil and d.tab:get() ~= "upload"
		end, ui.text {
			id = "bots-create-failed",
			function(): string
				return locale.s(d.said:get() or "bots.create.failed")
			end,
			role = "alert",
			pt = "$space.2",
			text_size = "$font.size.xs",
			color = "$danger",
		}),
		foot(
			pill { ui.t("bots.create.back"), id = "bots-create-back", tone = "secondary", on_click = back },
			pill { ui.t("bots.create.create"), id = "bots-create-make", tone = "primary", busy = d.busy, on_click = make }
		),
	}
end

local Body = ui.component(function(props: { close: () -> (), phase: State<number> }): Node
	local d: Draft = {
		step = ui.use_state(1),
		back = ui.use_state(false),
		moved = ui.use_state(false),
		template = ui.use_state(templates.SCRATCH),
		query = ui.use_state(""),
		tab = ui.use_state("icons"),
		name = ui.use_state(""),
		glyph = ui.use_state("bot"),
		frame = ui.use_state("squircle"),
		color = ui.use_state("violet"),
		image = ui.use_state(nil :: string?),
		said = ui.use_state(nil :: string?),
		busy = ui.use_state(false),
	}
	local function next()
		local t = templates.find(d.template:peek())
		ui.batch(function()
			if t.id ~= templates.SCRATCH then
				d.name:set(locale.s(t.name))
				d.glyph:set(t.glyph)
				d.color:set(t.color)
			end
			d.back:set(false)
			d.moved:set(true)
			d.step:set(2)
		end)
	end
	local function back()
		ui.batch(function()
			d.back:set(true)
			d.moved:set(true)
			d.said:set(nil)
			d.step:set(1)
		end)
	end
	local function make()
		if d.busy:peek() then
			return
		end
		local t = templates.find(d.template:peek())
		local name = trimmed(d.name:peek())
		local defaults = chat_store.new_defaults:peek()
		local model = defaults.model
		if model == nil then
			local first = chat_store.catalog:peek().models[1]
			model = first and first.value
		end
		local project = defaults.project
		d.busy:set(true)
		d.said:set(nil)
		store.create({
			name = if name == "" then locale.s("bots.create.default_name") else name,
			avatar = { glyph = d.glyph:peek(), frame = d.frame:peek(), color = d.color:peek(), image = d.image:peek() },
			template = t.id,
			instructions = if t.id == templates.SCRATCH then nil else locale.s(t.desc),
			picks = { model = model, effort = defaults.effort, access = defaults.access, fast = defaults.fast },
			project = if project ~= nil and project ~= "none" then project else nil,
			device = defaults.device,
		}):after(function(made: any)
			d.busy:set(false)
			props.close()
			if type(made) == "table" and type(made.chat) == "string" then
				store.go_chat(made.chat)
			end
		end, function(err: any)
			local why = if type(err) == "table" then err.message else err
			d.busy:set(false)
			d.said:set(if why == "no agent" then "bots.create.no_agent" else "bots.create.failed")
		end)
	end
	local phase = props.phase
	return ui.div {
		relative = true,
		w_full = true,
		h_full = true,
		items = "center",
		justify = "center",
		ui.div {
			id = "bots-create-scrim",
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			bottom = 0,
			bg = ui.raw("#0000002e", "the mockup's scrim"),
			opacity = function(): number
				return math.min(1, phase:get() * (IN_MS / SCRIM_MS))
			end,
			on_click = props.close,
		},
		ui.row {
			id = "bots-create-card",
			relative = true,
			w = W,
			h = H,
			max_w = "calc(100% - 32px)",
			max_h = "calc(100% - 32px)",
			p = ui.raw(10, "the mockup's dialog"),
			rounded = ui.raw(RADIUS, "the brief's dialog radius"),
			bg = "$zeron.dialog.solid",
			shadow = "$zeron.dialog.shadow",
			opacity = function(): number
				return phase:get()
			end,
			scale = function(): number
				return 0.97 + 0.03 * phase:get()
			end,
			y = function(): number
				return 6 * (1 - phase:get())
			end,
			Preview(d),
			ui.column {
				relative = true,
				grow = 1,
				min_w = 0,
				h_full = true,
				pt = ui.raw(22, "the mockup's dialog"),
				pr = ui.raw(12, "the mockup's dialog"),
				pb = ui.raw(8, "the mockup's dialog"),
				pl = ui.raw(26, "the mockup's dialog"),
				ui.div {
					absolute = true,
					right = 8,
					top = 6,
					ds.IconButton { id = "bots-create-close", icon = "close", label = ui.t("bots.create.close"), size = "sm", tone = "quiet", on_click = props.close },
				},
				ui.text {
					id = "bots-create-title",
					function(): string
						return locale.s(if d.step:get() == 1 then "bots.create.goal" else "bots.create.customize")
					end,
					role = "heading",
					text_size = ui.raw(16.5, "the mockup's dialog"),
					line_height = ui.raw(20, "the mockup's dialog"),
					weight = "$font.weight.semibold",
					color = "$text.primary",
				},
				ui.text {
					function(): string
						return locale.s(if d.step:get() == 1 then "bots.create.goal_blurb" else "bots.create.customize_blurb")
					end,
					mt = ui.raw(8, "the mockup's dialog"),
					mb = ui.raw(18, "the mockup's dialog"),
					text_size = ui.raw(13.5, "the mockup's dialog"),
					line_height = ui.raw(20, "the mockup's dialog"),
					color = "$text.secondary",
					wrap = true,
				},
				ui.presence(function(): number
					return d.step:get()
				end, function(step: number): Node
					return ui.column {
						w_full = true,
						grow = 1,
						min_h = 0,
						on_mount = function(el: any)
							if d.moved:peek() then
								arrives({ opacity = { 0, 1 }, x = { if d.back:peek() then -10 else 10, 0 } }, STEP_MS)(el)
							end
						end,
						if step == 1 then Goal(d, next) else Customize(d, back, make),
					}
				end, { w_full = true, grow = 1, min_h = 0 }),
			},
		},
	}
end)

M.Dialog = ui.component(function(props: { from: string }): Node
	local phase = ui.use_state(0)
	local shown = ui.use_state(false)
	local moving: any = nil
	local function close()
		if asked:peek() == props.from then
			asked:set("")
		end
	end
	ui.effect(function()
		local want = asked:get() == props.from
		if moving then
			moving:cancel()
			moving = nil
		end
		if want then
			shown:set(true)
			moving = anime.animate(phase, motion.params({ value = 1, duration = IN_MS, ease = EASE }))
		elseif shown:peek() then
			moving = anime.animate(phase, motion.params({ value = 0, duration = OUT_MS, ease = "inQuad" }))
			ui.after(math.max(motion.ms(OUT_MS), 1), function()
				if asked:peek() ~= props.from then
					shown:set(false)
				end
			end)
		end
	end)
	return ui.div {
		theme = { backdrop = "#00000000" },
		ui.modal {
			open = function(): boolean
				return shown:get()
			end,
			on_close = close,
			label = ui.t("bots.create.title"),
			placement = { x = "start", y = "start", inset = 0 },
			w = function(): number
				return window_size().width
			end,
			h = function(): number
				return window_size().height
			end,
			max_w = function(): number
				return window_size().width
			end,
			max_h = function(): number
				return window_size().height
			end,
			p = 0,
			gap = 0,
			rounded = 0,
			bg = "#00000000",
			border = 0,
			shadow = "$zeron.dialog.stage",
			build = function(): Node
				return Body { close = close, phase = phase }
			end,
		},
	}
end)

return M
