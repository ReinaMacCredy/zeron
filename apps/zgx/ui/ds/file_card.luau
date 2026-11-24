local FileIcon = require("@self/file_icons")
local Button = require("@self/button")
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
}
export type Item = { target: string, words: string }
export type RowProps = {
	files: Files,
	items: { Item },
	open: ((target: string) -> ())?,
	pictures: ReplyPicture.Pictures?,
	group: string?,
}

local HEAD_LINES, CSV_ROWS = 12, 3
local LINE_MOST = 400
local READ_MOST = 2 * 1024 * 1024
local PICTURE = { png = true, jpg = true, jpeg = true, gif = true, webp = true }
local OPEN_MOVE = { duration = 200, ease = "cubicBezier(.2,.8,.2,1)" }

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

local Preview = ui.component(function(props: { row: RowProps, item: Item, file: File, measured: State<number> }): Node
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
			min_w = 0,
			gap = "$space.2",
			pt = "$space.2",
			px = "$space.3",
			pb = "$zeron.space.snug",
			rounded = ui.raw(10, "the mockup's file preview corner"),
			border = 1,
			border_color = "$border",
			ui.row {
				w_full = true,
				min_w = 0,
				gap = "$space.1",
				items = "center",
				ui.text { file.name, literal = true, truncate = true, grow = 1, min_w = 0, text_size = "$font.size.sm", color = "$text.tertiary" },
				if open
					then Button {
						tone = "ghost",
						size = "xs",
						class = "md-file-open",
						on_click = function()
							open(item.target)
						end,
						locale.t("ds.file_card.open_in_panel"),
					}
					else nil,
				ui.show(function(): boolean
					return not file.outside and files.here()
				end, Button {
					tone = "ghost",
					size = "xs",
					class = "md-file-reveal",
					on_click = function()
						files.reveal(item.target)
					end,
					locale.t("ds.links.reveal"),
				}),
			},
			body(),
		},
	}
end)

M.Row = ui.component(function(props: RowProps): Node
	local files = props.files
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
	local function card(item: Item): Node
		local target = item.target
		local function name(): string
			local f = file_of(target)
			return if f then f.name else base_name(item.words)
		end
		local function is_open(): boolean
			return opened() == target
		end
		local function faded(): number
			return if gone(target) then 0.6 else 1
		end
		return ui.button {
			class = "md-file-card",
			label = name,
			expanded = is_open,
			disabled = function(): boolean
				return gone(target)
			end,
			dim = false,
			min_w = 216,
			max_w = "100%",
			gap = "$zeron.space.snug",
			items = "center",
			pt = "$space.2",
			pb = "$space.2",
			pl = "$zeron.space.snug",
			pr = "$space.3",
			rounded = ui.raw(10, "the mockup's file card corner"),
			border = 1,
			border_color = function(): string
				return if is_open() then "$border.strong" else "$border"
			end,
			bg = function(): string
				return if is_open() then "$zeron.secondary.bg" else "#00000000"
			end,
			hover = { bg = "$hover" },
			on_click = function()
				if gone(target) then
					return
				end
				files.check(target)
				open_at:set(if open_at:peek() == target then nil else target)
			end,
			ui.row {
				w = 28,
				h = 28,
				shrink = 0,
				items = "center",
				justify = "center",
				rounded = ui.raw(7, "the mockup's file icon tile"),
				bg = "$zeron.secondary.bg",
				opacity = faded,
				aria_hidden = true,
				FileIcon { name = name, size = 16 },
			},
			ui.column {
				min_w = 0,
				items = "start",
				opacity = faded,
				ui.text { name, literal = true, truncate = true, text_size = "$font.size.md", line_height = "$font.line.md", weight = "$font.weight.medium", color = "$text.primary" },
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
					truncate = true,
					text_size = "$font.size.sm",
					line_height = "$font.line.sm",
					color = function(): string
						return if gone(target) then "$warning" else "$text.tertiary"
					end,
				},
			},
		}
	end
	local cards: { Node } = {}
	local by_target: { [string]: Item } = {}
	for _, item in props.items do
		by_target[item.target] = item
		table.insert(cards, card(item))
	end
	return ui.column {
		class = "md-file-cards",
		w_full = true,
		min_w = 0,
		ui.row { w_full = true, min_w = 0, flex_wrap = true, gap = "$space.2", cards },
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
				return Preview { row = props, item = item, file = file, measured = measured }
			end, { w_full = true, min_w = 0 }),
		},
	}
end)

return M
