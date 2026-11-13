local Mark = require("@self/mark")
local Icon = require("@self/icon")
local glass = require("@self/glass")
local locale = require("@self/locale")
local look = require("@self/look")

local MIN_WIDTH = 768
-- gpui gates on the conversation column less 10 (shell.rs `main_width`).
local GUTTER = 10
local PROMPT_LINES, REPLY_LINES = 3, 4
local SOURCE_BYTES = 2048
local SPACING, SLOT = 8, 10
local HEIGHT_SHARE, MAX_HEIGHT = 0.7, 640
local EDGE_FADE = 24
local HIT_WIDTH = 32
local LEFT = 16
-- gpui's band starts at OWN_SEND_TOP_INSET_PX, 10 under a title bar that zgx's list starts below.
local BAND_TOP = 10
local CARD_W = 280
local CARD_MARGIN = 8

local M = {}

function M.visible(width: number): boolean
	return width - GUTTER >= MIN_WIDTH
end

function M.height(n: number): number
	return math.max(0, n - 1) * SPACING
end

function M.window(content: number, band: number): number
	return math.min(content, math.max(math.min(band * HEIGHT_SHARE, MAX_HEIGHT), SLOT * 4))
end

function M.follow(scroll: number, window: number, content: number, top: number, bottom: number, margin: number): number
	margin = math.min(margin, math.max(0, (window - (bottom - top)) / 2))
	if bottom + margin > scroll + window then
		scroll = bottom + margin - window
	end
	if top - margin < scroll then
		scroll = top - margin
	end
	return math.clamp(scroll, 0, math.max(0, content - window))
end

function M.offset(ix: number, n: number, height: number): number
	return if n <= 1 then 0 else (math.min(ix, n) - 1) / (n - 1) * height
end

function M.at(n: number, height: number, y: number): number?
	if n == 0 then
		return nil
	end
	if n == 1 or height <= 0 then
		return 1
	end
	local progress = math.clamp(y / height, 0, 1)
	return math.min(math.floor(progress * (n - 1) + 0.5), n - 1) + 1
end

function M.width(distance: number?): number
	if distance == 0 then
		return 24
	elseif distance == 1 then
		return 16
	elseif distance == 2 then
		return 10
	end
	return 8
end

function M.active(rows: { number }, top_row: number): number?
	if #rows == 0 then
		return nil
	end
	for i = #rows, 1, -1 do
		if rows[i] <= top_row then
			return i
		end
	end
	return 1
end

export type Run = { x: string, b: boolean?, i: boolean?, c: boolean?, s: boolean?, a: string? }

local function block_runs(block: any, out: { Run })
	local t = block.t
	if t == "p" or t == "h" then
		for _, r in block.s do
			table.insert(out, { x = r.x, b = r.b or t == "h", i = r.i, c = r.c, s = r.s, a = r.a })
		end
	elseif t == "ul" or t == "ol" then
		for _, item in block.items do
			for _, child in item do
				block_runs(child, out)
			end
		end
		return
	else
		return
	end
	table.insert(out, { x = " " })
end

local function first_code_line(block: any): string?
	if block.t == "code" then
		for line in string.gmatch(block.text, "[^\n]+") do
			local trimmed = string.match(line, "^%s*(.-)%s*$") :: string
			if trimmed ~= "" then
				return trimmed
			end
		end
		return nil
	elseif block.t == "quote" then
		for _, child in block.b do
			local found = first_code_line(child)
			if found then
				return found
			end
		end
	elseif block.t == "ul" or block.t == "ol" then
		for _, item in block.items do
			for _, child in item do
				local found = first_code_line(child)
				if found then
					return found
				end
			end
		end
	end
	return nil
end

function M.preview(source: string): { Run }
	local stop = math.min(#source, SOURCE_BYTES)
	while stop > 0 and stop < #source do
		local byte = string.byte(source, stop + 1)
		if byte < 0x80 or byte >= 0xC0 then
			break
		end
		stop -= 1
	end
	local blocks = ui.markdown(string.sub(source, 1, stop))
	local runs: { Run } = {}
	for _, block in blocks do
		block_runs(block, runs)
	end
	local blank = true
	for _, r in runs do
		if string.find(r.x, "%S") then
			blank = false
			break
		end
	end
	if blank then
		runs = {}
		for _, block in blocks do
			local line = first_code_line(block)
			if line then
				runs = { { x = line, c = true } }
				break
			end
		end
	end
	local out: { Run } = {}
	local space = false
	for _, run in runs do
		local text = {}
		for _, c in utf8.codes(run.x) do
			local ch = utf8.char(c)
			if string.find(ch, "^%s$") then
				space = #out > 0 or #text > 0
			else
				if space then
					if #text == 0 then
						table.insert(out, { x = " " })
					else
						table.insert(text, " ")
					end
					space = false
				end
				table.insert(text, ch)
			end
		end
		if #text > 0 then
			table.insert(out, { x = table.concat(text), b = run.b, i = run.i, c = run.c, s = run.s, a = run.a })
		end
	end
	return out
end

local function preview_text(source: string, size: string, color: string, lines: number): Node?
	local runs = M.preview(source)
	if #runs == 0 then
		return nil
	end
	local props: { [any]: any } = {
		wrap = true,
		lines = lines,
		truncate = true,
		text_size = size,
		color = color,
		w_full = true,
	}
	for _, r in runs do
		local span: { [any]: any } = { r.x, literal = true }
		if r.b then
			span.weight = ui.raw(600, "semibold; a span's weight takes no token")
		end
		if r.i then
			span.italic = true
		end
		if r.s then
			span.strike = true
		end
		if r.c then
			span.font = look.code_family()
			span.bg = "$zeron.prose.code_wash"
			span.padding = { 2, 1 }
			span.radius = ui.raw(3.5, "rail.rs preview's inline code corner")
		end
		if r.a then
			span.underline = true
			span.underline_color = "$text.secondary"
		end
		table.insert(props, ui.span(span))
	end
	return ui.rich(props)
end

local function prompt_source(m: any): string
	local text = m.text or ""
	if string.find(text, "%S") then
		return text
	end
	local shown = m.attachments or {}
	if #shown == 0 then
		return text
	end
	local images = true
	for _, a in shown do
		if a.kind ~= "image" then
			images = false
		end
	end
	if #shown == 1 then
		return locale.s(if images then "ds.message_rail.attached_image" else "ds.message_rail.attached_file")
	end
	return locale.s(if images then "ds.message_rail.attached_images" else "ds.message_rail.attached_files", { n = tostring(#shown) })
end

local function reply_source(t: any, row: number): string?
	local messages = t.messages
	for i = row + 1, messages:len() do
		local m = messages:at(i)
		if m and m.role == "assistant" then
			for _, b in m.blocks or {} do
				if b.kind == "text" and b.text and string.find(b.text, "%S") then
					return string.match(b.text, "^%s*(.-)%s*$")
				end
			end
			local draft = if t.draft then t.draft:peek() else nil
			local text = if draft and draft.id == m.id then draft.text else m.text
			if text and string.find(text, "%S") then
				return string.match(text, "^%s*(.-)%s*$")
			end
		end
	end
	return nil
end

export type Props = {
	transcript: any,
	gate: boolean?,
	name: string?,
}

local function MessageRail(props: Props): Node
	local t = props.transcript
	local messages = t.messages
	local kept = if props.name then `.{props.name}` else ""
	local wide = ui.use_state(false)
	local box_h = ui.use_state(0)
	local hover = ui.use_state(0)
	local scroll = ui.use_state(0)
	local card_h = ui.use_state(0)
	local lit = ui.state(`message_rail.lit{kept}`, { first = 0, last = 0 })
	local prompts = ui.sorted(messages, { filter = { role = "user" } })
	local ticks = ui.computed(function(): { ids: { string }, rows: { number }, at: { [string]: number } }
		local ids, rows, at = {}, {}, {}
		for i = 1, prompts:len() do
			local m: any = prompts:at(i)
			local row = messages:index_of("id", m.id)
			if row then
				table.insert(ids, m.id)
				table.insert(rows, row)
				at[m.id] = #ids
			end
		end
		return { ids = ids, rows = rows, at = at }
	end)
	local ids = ui.computed(function(): { string }
		return ticks:get().ids
	end)
	local n = ui.computed(function(): number
		return #ticks:get().ids
	end)
	local function band(): number
		return box_h:get() - BAND_TOP - t.foot()
	end
	local function height(): number
		return M.height(n:get())
	end
	local function content(): number
		return height() + SLOT
	end
	local function window(): number
		return M.window(content(), band())
	end
	local scrolls = ui.computed(function(): boolean
		return content() > window() + 0.5
	end)
	local shown = ui.computed(function(): boolean
		return (not props.gate or wide:get()) and n:get() >= 2
	end)

	local followed = ""
	local function measure()
		local tk = ticks:peek()
		local count = #tk.ids
		if count == 0 then
			return
		end
		local list_h = t.list.height or 0
		local seen = 0
		local read_top = BAND_TOP + 0.5
		local view_top, view_bottom = BAND_TOP, list_h - t.foot()
		local top_row, first, last = 0, 0, 0
		t.rows(function(id: string, index: number, top: number, bottom: number)
			if top <= read_top and index > top_row then
				top_row = index
			end
			seen += 1
			local p = tk.at[id]
			if p and bottom > view_top and top < view_bottom then
				first = if first == 0 then p else math.min(first, p)
				last = math.max(last, p)
			end
		end)
		if first == 0 then
			local active = M.active(tk.rows, top_row) or 0
			first, last = active, active
		end
		local now = lit:peek()
		if now.first ~= first or now.last ~= last then
			lit:set { first = first, last = last }
		end
		if scrolls:peek() and hover:peek() == 0 and first > 0 then
			local key = `{tk.ids[first]}:{tk.ids[last]}`
			if key ~= followed then
				followed = key
				local h = M.height(count)
				local target = M.follow(scroll:peek(), M.window(h + SLOT, box_h:peek() - BAND_TOP - t.foot()), h + SLOT, M.offset(first, count, h), M.offset(last, count, h) + SLOT, EDGE_FADE)
				if math.abs(target - scroll:peek()) >= 0.5 then
					scroll:set(target)
				end
			end
		end
	end
	ui.effect(function()
		t.offset:get()
		t.sized:get()
		ticks:get()
		t.foot()
		box_h:get()
		if shown:get() then
			ui.after(0, measure)
		end
	end)

	local function at_pointer(y: number): number
		local count = n:peek()
		return M.at(count, M.height(count), y - SLOT / 2) or 0
	end
	local pressed = 0
	local function go(ix: number)
		local id = ticks:peek().ids[ix]
		if id then
			t.go(id)
		end
	end

	local function tick(id: string): Node
		local function distance(): number?
			local h = hover:get()
			if h == 0 then
				return nil
			end
			local p = ticks:get().at[id]
			return if p then math.abs(h - p) else nil
		end
		return ui.row {
			id = `message-rail-{id}`,
			w_full = true,
			h = SPACING,
			items = "center",
			ui.div {
				id = `message-rail-bar-{id}`,
				h = 2,
				rounded = ui.raw(1, "rail.rs tick"),
				w = function(): number
					return M.width(distance())
				end,
				bg = function(): string
					local p = ticks:get().at[id] or 0
					local l = lit:get()
					return if distance() == 0 or (p >= l.first and p <= l.last) then "$zeron.message_rail.lit" else "$zeron.message_rail.tick"
				end,
			},
		}
	end

	local function stack(): Node
		return ui.column {
			id = "message-rail-ticks",
			w_full = true,
			h = content,
			pt = ui.raw((SLOT - SPACING) / 2, "rail.rs: half a slot over the first tick"),
			pb = ui.raw((SLOT - SPACING) / 2, "rail.rs: half a slot under the last tick"),
			cursor = "pointer",
			focusable = false,
			pointer_events = "auto",
			on_pointer_move = function(e: any)
				local ix = at_pointer(e.local_y)
				if hover:peek() ~= ix then
					hover:set(ix)
				end
			end,
			on_hover = function(inside: boolean)
				if not inside and hover:peek() ~= 0 then
					hover:set(0)
				end
			end,
			on_press = function(e: any)
				pressed = at_pointer(e.local_y)
			end,
			on_click = function()
				local ix = if pressed > 0 then pressed else hover:peek()
				pressed = 0
				go(ix)
			end,
			ui.each(ids, tick, {
				key = function(id: string): string
					return id
				end,
			}),
		}
	end

	local function sender_line(m: any): Node?
		local from = m.sender
		if not from then
			return nil
		end
		local c = if t.chat_of then t.chat_of(from.chat) else nil
		local mark = if c then c.mark else nil
		return ui.row {
			gap = ui.raw(5, "rail.rs sender line"),
			items = "center",
			min_w = 0,
			text_size = "$font.size.xs",
			color = "$text.secondary",
			ui.text { locale.t("ds.transcript.message_from"), shrink = 0 },
			ui.row {
				gap = ui.raw(5, "transcript.rs caption_label"),
				items = "center",
				min_w = 0,
				if mark then Mark { name = mark, size = 14, bare = true } else Icon { name = "chat-round-line", size = 14 },
				ui.text { if c then c.title else from.title, literal = true, truncate = true, weight = "$font.weight.medium" },
			},
		}
	end

	local function card(ix: number): Node
		local tk = ticks:peek()
		local id = tk.ids[ix]
		local row = tk.rows[ix]
		local m: any = if row then messages:at(row) else nil
		if not id or not m then
			return ui.div {}
		end
		local reply = reply_source(t, row)
		local shell = glass.style({
			id = "message-rail-card",
			w = CARD_W,
			p = "$space.2",
			gap = ui.raw(6, "rail.rs preview card"),
			rounded = "$radius.xl",
			on_resize = function(size: { w: number, h: number })
				if math.abs(card_h:peek() - size.h) >= 0.5 then
					card_h:set(size.h)
				end
			end,
		}, "menu")
		local sender = sender_line(m)
		local prompt = preview_text(prompt_source(m), "$font.size.sm", "$text.primary", PROMPT_LINES)
		local said = if reply then preview_text(reply, "$font.size.xs", "$text.secondary", REPLY_LINES) else nil
		if sender then
			table.insert(shell, sender)
		end
		if prompt then
			table.insert(shell, prompt)
		end
		if said then
			table.insert(shell, said)
		end
		return ui.column(shell)
	end

	local function card_top(): number
		local ix = hover:get()
		local count = n:get()
		local h = M.height(count)
		local win = window()
		local rail_top = BAND_TOP + (band() - win) / 2
		local within = if scrolls:get() then scroll:get() else 0
		local centre = rail_top + M.offset(ix, count, h) - within + SLOT / 2
		local ch = card_h:get()
		return math.clamp(centre - ch / 2, CARD_MARGIN, math.max(CARD_MARGIN, box_h:get() - CARD_MARGIN - ch))
	end

	return ui.div {
		absolute = true,
		left = 0,
		right = 0,
		top = 0,
		bottom = 0,
		pointer_events = "none",
		on_resize = function(size: { w: number, h: number })
			local on = M.visible(size.w)
			if wide:peek() ~= on then
				wide:set(on)
			end
			if math.abs(box_h:peek() - size.h) >= 0.5 then
				box_h:set(size.h)
			end
		end,
		ui.show(
			shown,
			ui.column {
				id = "message-rail",
				absolute = true,
				left = ui.raw(LEFT, "rail.rs: 16 in from the transcript's edge"),
				top = ui.raw(BAND_TOP, "rail.rs band top"),
				bottom = t.foot,
				w = HIT_WIDTH,
				justify = "center",
				pointer_events = "none",
				ui.presence(function(): string
					return if scrolls:get() then "scrolls" else "fits"
				end, function(kind: string): Node
					if kind == "fits" then
						return stack()
					end
					return ui.scroll {
						id = "message-rail-scroll",
						w_full = true,
						h = window,
						offset = scroll,
						overscroll = "contain",
						pointer_events = "auto",
						fade = function(): { number }
							local at = scroll:get()
							return { if at > 0.5 then EDGE_FADE else 0, if at < content() - window() - 0.5 then EDGE_FADE else 0 }
						end,
						stack(),
					}
				end, { w_full = true }),
			}
		),
		ui.presence(function(): number?
			local ix = hover:get()
			return if shown:get() and ix > 0 then ix else nil
		end, function(ix: number): Node
			return ui.div {
				absolute = true,
				left = ui.raw(LEFT + HIT_WIDTH + 4, "rail.rs: the card 4 past the hit strip"),
				top = card_top,
				pointer_events = "none",
				card(ix),
			}
		end),
	}
end

M.MessageRail = MessageRail

return M
