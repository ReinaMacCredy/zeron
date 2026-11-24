local FileIcon = require("@self/file_icons")
local Button = require("@self/button")
local Icon = require("@self/icon")
local IconButton = require("@self/icon_button").IconButton
local Links = require("@self/links")
local DataTable = require("@self/data_table")
local Csv = require("@self/csv")
local ReplyPicture = require("@self/reply_picture")
local motion = require("@self/motion")
local locale = require("@self/locale")

local M = {}

export type File = { path: string, name: string, size: number?, outside: boolean? }
export type Files = {
	find: (target: string) -> File | false | nil,
	check: (target: string) -> (),
	read: (target: string) -> any,
	reveal: (target: string) -> (),
	here: () -> boolean,
	shown: ((target: string) -> boolean)?,
	root: (() -> string?)?,
}
export type Item = { target: string, words: string }
export type RowProps = {
	files: Files,
	items: { Item },
	open: ((target: string) -> ())?,
	pictures: ReplyPicture.Pictures?,
	menu: ((target: string) -> { Links.Entry }?)?,
	group: string?,
}

local HEAD_LINES, CSV_ROWS = 8, 3
local LINE_MOST = 400
local READ_MOST = 2 * 1024 * 1024
local PICTURE = { png = true, jpg = true, jpeg = true, gif = true, webp = true }
local OPEN_MOVE = { duration = 200, ease = "cubicBezier(.2,.8,.2,1)" }
local HOVER_MOVE = { duration = 100, ease = "linear" }
local ROW_H, LIST_MOST = 38, 560

M.shown = {} :: { [string]: boolean }
local shown_count = 0
function M.remember(sig: string)
	if not M.shown[sig] then
		shown_count += 1
		if shown_count > 2000 then
			M.shown, shown_count = {}, 1
		end
		M.shown[sig] = true
	end
end

local function ext_of(name: string): string
	return string.lower(string.match(name, "%.([^./]+)$") or "")
end

local function kind_of(name: string): string
	local ext = ext_of(name)
	if ext == "md" or ext == "markdown" then
		return "Markdown"
	elseif ext == "txt" then
		return locale.s("ds.file_card.text")
	elseif ext == "" then
		return locale.s("ds.file_card.file")
	end
	return string.upper(ext)
end

local function bytes(n: number): string
	if n < 1024 then
		return locale.s("ds.file_viewer.bytes", { n = tostring(n) })
	elseif n < 1024 * 1024 then
		return string.format("%.0f KB", n / 1024)
	end
	local mb = n / (1024 * 1024)
	return if mb < 10 then string.format("%.1f MB", mb) else string.format("%.0f MB", mb)
end

local function base_name(words: string): string
	local bare = string.gsub(string.gsub(words, "^zeron%-file:", ""), "[:#].*$", "")
	return string.match(bare, "([^/]+)$") or words
end

local function dir_of(path: string): string
	return string.match(path, "^(.*)/[^/]+$") or ""
end

local function leaf(path: string): string
	return string.match(path, "([^/]+)/*$") or path
end

local function head(text: string, most: number): (string, number)
	local lines: { string } = {}
	local at = 1
	while #lines < most do
		local nl = string.find(text, "\n", at, true)
		if not nl then
			if at <= #text then
				table.insert(lines, string.sub(text, at))
			end
			at = #text + 1
			break
		end
		table.insert(lines, string.sub(text, at, nl - 1))
		at = nl + 1
	end
	for i, line in lines do
		if #line > LINE_MOST then
			local cut = string.gsub(string.gsub(string.sub(line, 1, LINE_MOST), "[\128-\191]*$", ""), "[\192-\255]$", "")
			lines[i] = cut .. "…"
		end
	end
	local rest = string.sub(text, at)
	local more = select(2, string.gsub(rest, "\n", ""))
	if rest ~= "" and string.sub(rest, -1) ~= "\n" then
		more += 1
	end
	return (string.gsub(table.concat(lines, "\n"), "\r", "")), more
end

local function quiet(words: string): Node
	return ui.text { words, literal = true, text_size = "$font.size.sm", line_height = "$font.line.sm", color = "$text.tertiary" }
end

local function more_of(n: number, truncated: boolean): Node?
	if truncated then
		return quiet(locale.s("ds.file_card.more_in_panel"))
	end
	return if n > 0 then quiet(locale.s("ds.file_card.more_lines", { n = n })) else nil
end

local function csv_body(text: string, ext: string, truncated: boolean): Node
	local shown, more = head(text, CSV_ROWS + 1)
	local rows = Csv.parse(shown, if ext == "tsv" then "\t" else ",")
	local columns: { DataTable.Column } = {}
	for _, label in rows[1] or {} do
		table.insert(columns, { label = label })
	end
	return ui.column {
		w_full = true,
		min_w = 0,
		gap = "$space.1",
		DataTable {
			columns = columns,
			rows = math.max(#rows - 1, 0),
			hover = false,
			cell = function(r: number, c: number): Node
				return ui.text { (rows[r + 1] or {})[c] or "", literal = true, truncate = true, text_size = "$font.size.sm" }
			end,
		},
		more_of(more, truncated),
	}
end

local function text_body(text: string, truncated: boolean): Node
	local shown, more = head(text, HEAD_LINES)
	return ui.column {
		w_full = true,
		min_w = 0,
		gap = "$space.1",
		ui.text {
			shown,
			literal = true,
			wrap = true,
			font = "$font.mono",
			text_size = "$zeron.prose.code.size",
			line_height = "$zeron.prose.code.line",
			color = "$text.primary",
		},
		more_of(more, truncated),
	}
end

local Preview = ui.component(function(props: { row: RowProps, item: Item, file: File, measured: State<number>, close: () -> () }): Node
	local files, item, file = props.row.files, props.item, props.file
	local ext = ext_of(file.name)
	local picture = PICTURE[ext] == true
	local got = ui.use_state(nil :: any)
	if not picture and (file.size or 0) <= READ_MOST then
		files.read(item.target):after(function(r: any)
			got:set(if type(r) == "table" then r else false)
		end, function()
			got:set(false)
			files.check(item.target)
		end)
	end
	local function body(): Node
		if picture then
			local pictures = props.row.pictures
			if file.outside or not pictures or pictures.resolve(file.path) == nil then
				return quiet(locale.s("ds.file_card.more_in_panel"))
			end
			local entry = { src = file.path, alt = file.name }
			return ReplyPicture.Single {
				pictures = pictures,
				src = file.path,
				alt = file.name,
				group = `{props.row.group or "reply"}:file`,
				set = function(): { ReplyPicture.Entry }
					return { entry }
				end,
				open_link = props.row.open,
			}
		end
		if (file.size or 0) > READ_MOST then
			return quiet(locale.s("ds.file_card.more_in_panel"))
		end
		return ui.presence(function(): string
			local r = got:get()
			if r == nil then
				return "wait"
			elseif r == false or type(r.text) ~= "string" or r.encoding == "binary" or r.encoding == "unsupported" then
				return "none"
			end
			return "text"
		end, function(k: string): Node
			if k == "wait" then
				return ui.div { h = "$font.line.sm" }
			elseif k == "none" then
				return quiet(locale.s("ds.file_card.more_in_panel"))
			end
			local r = got:peek()
			if ext == "csv" or ext == "tsv" then
				return csv_body(r.text, ext, r.truncated == true)
			end
			return text_body(r.text, r.truncated == true)
		end, { w_full = true, min_w = 0 })
	end
	local open = props.row.open
	return ui.column {
		class = "md-file-preview",
		w_full = true,
		shrink = 0,
		pt = "$space.2",
		on_resize = function(size: { w: number, h: number })
			if math.abs(size.h - props.measured:peek()) >= 0.5 then
				props.measured:set(size.h)
			end
		end,
		ui.column {
			w_full = true,
			max_w = LIST_MOST,
			min_w = 0,
			rounded = ui.raw(12, "the mockup's file list corner"),
			border = 1,
			border_color = "$border",
			overflow = "hidden",
			ui.row {
				w_full = true,
				min_w = 0,
				h = ROW_H,
				gap = "$space.2",
				items = "center",
				pl = "$space.3",
				pr = "$zeron.space.tight",
				ui.row { shrink = 0, aria_hidden = true, FileIcon { name = file.name, size = 14 } },
				ui.text { file.name, literal = true, truncate = true, grow = 1, min_w = 0, text_size = "$font.size.sm", weight = "$font.weight.medium", color = "$text.primary" },
				if open
					then Button {
						tone = "ghost",
						size = "xs",
						class = "md-file-open",
						on_click = function()
							open(item.target)
						end,
						Icon { name = "split-columns", size = 14 },
						locale.t("ds.file_card.open_in_pane"),
					}
					else nil,
				IconButton {
					icon = "close-thin",
					size = "md",
					class = "md-file-close",
					label = locale.t("ds.file_card.close_preview"),
					on_click = props.close,
				},
			},
			ui.div { w_full = true, h = 1, bg = "$border" },
			ui.column { w_full = true, min_w = 0, p = "$space.3", body() },
		},
	}
end)

M.Row = ui.component(function(props: RowProps): Node
	local files, open, menu = props.files, props.open, props.menu
	local open_at = ui.use_state(nil :: string?)
	local laid = ui.use_state(nil :: string?)
	local measured = ui.use_state(0)
	local last: { [string]: File } = {}
	local function file_of(target: string): File?
		local f = files.find(target)
		if f then
			last[target] = f
			return f
		end
		return last[target]
	end
	local function gone(target: string): boolean
		return files.find(target) == false
	end
	local function opened(): string?
		local at = open_at:get()
		return if at and not gone(at) then at else nil
	end
	ui.effect(function()
		local on = opened()
		if on then
			laid:set(on)
		elseif laid:peek() then
			ui.after(math.max(motion.ms(OPEN_MOVE.duration), motion.FADE) + 40, function()
				if open_at:peek() == nil or gone(open_at:peek() :: string) then
					laid:set(nil)
				end
			end)
		end
	end)
	local function peek(target: string)
		if gone(target) then
			return
		end
		files.check(target)
		open_at:set(if open_at:peek() == target then nil else target)
	end
	local function folder(target: string): string
		local f = file_of(target)
		local dir = if f then dir_of(f.path) else ""
		if f and f.outside then
			return dir
		end
		local root = if files.root then files.root() else nil
		return if dir ~= "" then dir elseif root then leaf(root) else ""
	end
	local first = props.items[1].target
	local function line(item: Item): Node
		local target = item.target
		local over = ui.use_state(false)
		local focused = ui.use_state(false)
		local function focus()
			focused:set(true)
		end
		local function blur()
			focused:set(false)
		end
		local function name(): string
			local f = file_of(target)
			return if f then f.name else base_name(item.words)
		end
		local function faded(): number
			return if gone(target) then 0.6 else 1
		end
		local function peeked(): boolean
			return opened() == target
		end
		local function showing(): boolean
			return files.shown ~= nil and not gone(target) and files.shown(target)
		end
		local function acting(): boolean
			return not gone(target) and (over:get() or focused:get() or peeked() or showing())
		end
		return ui.row {
			class = "md-file-line",
			w_full = true,
			min_w = 0,
			h = ROW_H,
			items = "center",
			pr = "$space.3",
			bg = function(): string
				return if showing() then "$zeron.secondary.bg" elseif over:get() and not gone(target) then "$hover" else "#00000000"
			end,
			on_hover = function(inside: boolean)
				over:set(inside)
			end,
			context_menu = if open or menu
				then Links.menu_for(target, function(): { Links.Entry }?
					local own = if menu then menu(target) else nil
					if own ~= nil then
						return own
					end
					return Links.other(target, if open
						then function()
							open(target)
						end
						else nil)
				end)
				else nil,
			ui.button {
				class = "md-file-card",
				label = name,
				disabled = function(): boolean
					return gone(target)
				end,
				dim = false,
				grow = 1,
				min_w = 0,
				h = ROW_H,
				gap = "$zeron.space.snug",
				items = "center",
				justify = "start",
				pl = "$space.3",
				pr = "$space.2",
				bg = "#00000000",
				hover = { bg = "#00000000" },
				active = { bg = "#00000000" },
				on_click = function()
					if gone(target) then
						return
					end
					if open then
						files.check(target)
						open(target)
					else
						peek(target)
					end
				end,
				ui.row { shrink = 0, opacity = faded, aria_hidden = true, FileIcon { name = name, size = 16 } },
				ui.text { name, literal = true, truncate = true, min_w = 0, opacity = faded, text_size = "$font.size.md", line_height = "$font.line.md", weight = "$font.weight.medium", color = "$text.primary" },
				ui.text {
					function(): string
						local here = folder(target)
						return if here ~= folder(first) then here else ""
					end,
					literal = true,
					truncate = true,
					min_w = 0,
					max_w = 180,
					text_size = "$font.size.sm",
					line_height = "$font.line.sm",
					color = "$text.tertiary",
				},
			},
			ui.row {
				relative = true,
				shrink = 0,
				min_w = 52,
				h = ROW_H,
				items = "center",
				justify = "end",
				ui.text {
					function(): string
						if gone(target) then
							return locale.s("ds.file_card.gone")
						end
						local f = file_of(target)
						local kind = kind_of(if f then f.name else name())
						if f and f.size then
							return locale.s("ds.file_card.detail", { kind = kind, size = bytes(f.size) })
						end
						return kind
					end,
					literal = true,
					class = "md-file-detail",
					text_size = "$font.size.sm",
					line_height = "$font.line.sm",
					color = function(): string
						return if gone(target) then "$warning" else "$text.tertiary"
					end,
					opacity = motion.tween(ui.computed(function(): number
						return if acting() then 0 else 1
					end), HOVER_MOVE),
				},
				ui.row {
					absolute = true,
					top = 0,
					bottom = 0,
					right = ui.raw(-4, "the buttons' glyphs end where the detail's words do"),
					items = "center",
					gap = "$zeron.space.hair",
					hidden = function(): boolean
						return gone(target)
					end,
					opacity = motion.tween(ui.computed(function(): number
						return if acting() then 1 else 0
					end), HOVER_MOVE),
					IconButton {
						icon = "eye",
						size = "md",
						class = "md-file-peek",
						label = function(): string
							return locale.s("ds.file_card.quick_look_file", { name = name() })
						end,
						tooltip = locale.s("ds.file_card.quick_look"),
						lit = peeked,
						on_focus = focus,
						on_blur = blur,
						on_click = function()
							peek(target)
						end,
					},
					if open
						then IconButton {
							icon = "split-columns",
							size = "md",
							class = "md-file-pane",
							label = function(): string
								return locale.s("ds.file_card.open_file_in_pane", { name = name() })
							end,
							tooltip = locale.s("ds.file_card.open_in_pane"),
							lit = showing,
							on_focus = focus,
							on_blur = blur,
							on_click = function()
								files.check(target)
								open(target)
							end,
						}
						else nil,
				},
			},
		}
	end
	local lines: { Node } = {}
	local by_target: { [string]: Item } = {}
	for n, item in props.items do
		by_target[item.target] = item
		if n > 1 then
			table.insert(lines, ui.div { h = 1, bg = "$border", ml = ui.raw(38, "under the names, past the icons") })
		end
		table.insert(lines, line(item))
	end
	local many = #props.items > 1
	return ui.column {
		class = "md-file-cards",
		w_full = true,
		min_w = 0,
		ui.column {
			class = "md-file-list",
			w_full = true,
			max_w = LIST_MOST,
			min_w = 0,
			rounded = ui.raw(12, "the mockup's file list corner"),
			border = 1,
			border_color = "$border",
			overflow = "hidden",
			if many
				then ui.row {
					class = "md-file-head",
					w_full = true,
					min_w = 0,
					h = 32,
					items = "center",
					gap = "$space.2",
					pl = "$space.3",
					pr = "$space.1",
					ui.row { shrink = 0, color = "$text.tertiary", aria_hidden = true, Icon { name = "folder", size = 14 } },
					ui.text {
						function(): string
							return leaf(folder(first))
						end,
						literal = true,
						truncate = true,
						min_w = 0,
						text_size = "$font.size.sm",
						weight = "$font.weight.medium",
						color = "$text.primary",
					},
					ui.text { locale.t("ds.file_card.count", { n = #props.items }), shrink = 0, text_size = "$font.size.sm", color = "$text.tertiary" },
					ui.div { grow = 1 },
					ui.show(function(): boolean
						local f = file_of(first)
						return f ~= nil and not f.outside and not gone(first) and files.here()
					end, Button {
						tone = "ghost",
						size = "xs",
						class = "md-file-reveal",
						on_click = function()
							files.reveal(first)
						end,
						locale.t("ds.links.reveal"),
					}),
				}
				else nil,
			if many then ui.div { w_full = true, h = 1, bg = "$border" } else nil,
			lines,
		},
		ui.column {
			w_full = true,
			min_w = 0,
			overflow = "hidden",
			h = motion.tween(ui.computed(function(): number
				return if opened() then measured:get() else 0
			end), OPEN_MOVE),
			opacity = if motion.reduced_now()
				then ui.tween(ui.computed(function(): number
					return if opened() then 1 else 0
				end), { duration = motion.FADE })
				else nil,
			ui.presence(function(): string?
				return laid:get()
			end, function(target: string): Node
				local item = by_target[target]
				local file = file_of(target)
				if not item or not file then
					return ui.div {}
				end
				return Preview {
					row = props,
					item = item,
					file = file,
					measured = measured,
					close = function()
						open_at:set(nil)
					end,
				}
			end, { w_full = true, min_w = 0 }),
		},
	}
end)

return M
