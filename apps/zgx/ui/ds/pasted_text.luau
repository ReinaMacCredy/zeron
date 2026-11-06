-- `PastedText.open { title, path, session }`; `PastedText.Host()` stands once
-- where the window's overlays do.

local Icon = require("@self/icon")
local FadeText = require("@self/fade_text")
local control = require("@self/control")
local glass = require("@self/glass")
local locale = require("@self/locale")

local M = {}

local BLOCK_LINES, BLOCK_BYTES = 50, 8 * 1024
local CARD_MAX_W = 960
local STEPS = 2000

export type Request = { title: string, path: string, session: string? }

local OPEN = ui.state("open", nil :: Request?)
local PHASE = ui.state("phase", "loading")
local LINES = ui.state("lines", 0)
local COUNT = ui.state("count", 0)
-- (Here, not in state: a state would copy megabytes of text to no end.)
local blocks: { string } = {}
local asked = 0

-- (In steps: a script call's budget is 50 ms, and the text can be 32 MiB.)
local function cutter(text: string): any
	local cut = { blocks = {} :: { string }, lines = 0 }
	local parts: { string } = {}
	local in_block, bytes = 0, 0
	local at, size = 1, #text
	local from, last = 0, -1
	local function flush()
		table.insert(cut.blocks, table.concat(parts, "\n"))
		parts, in_block, bytes = {}, 0, 0
	end
	function cut.step(most: number): boolean
		for _ = 1, most do
			if from > last then
				if at > size then
					if in_block > 0 then
						flush()
					end
					return true
				end
				local stop = string.find(text, "\n", at, true) or (size + 1)
				from, last = at, stop - 1
				if last >= at and string.byte(text, last) == 13 then
					last -= 1
				end
				at = stop + 1
				cut.lines += 1
			end
			local to = math.min(last, from + BLOCK_BYTES - 1)
			-- (Never inside a character: back over continuation bytes.)
			while to < last and to > from do
				local b = string.byte(text, to + 1) :: number
				if b < 0x80 or b >= 0xC0 then
					break
				end
				to -= 1
			end
			local length = to - from + 1
			if in_block > 0 and (in_block == BLOCK_LINES or bytes + length > BLOCK_BYTES) then
				flush()
			end
			table.insert(parts, string.sub(text, from, to))
			in_block += 1
			bytes += length + 1
			from = math.max(to, from) + 1
		end
		return false
	end
	return cut
end

function M.decode(text: string): ({ string }, number)
	local cut = cutter(text)
	while not cut.step(STEPS) do
	end
	return cut.blocks, cut.lines
end

function M.read(path: string, session: string?): any
	return ui.call("chat.read_text", { path = path, session = session })
end

function M.close()
	asked += 1
	blocks = {}
	OPEN:set(nil)
	COUNT:set(0)
end

function M.open(request: Request)
	asked += 1
	local mine = asked
	blocks = {}
	ui.batch(function()
		PHASE:set("loading")
		COUNT:set(0)
		OPEN:set(request)
	end)
	local function failed()
		if mine == asked then
			PHASE:set("failed")
		end
	end
	local ok, answer = pcall(M.read, request.path, request.session)
	if not ok or answer == nil then
		failed()
		return
	end
	answer:after(function(read: any)
		if mine ~= asked then
			return
		end
		if type(read) ~= "table" or type(read.text) ~= "string" then
			failed()
			return
		end
		local cut = cutter(read.text)
		local function step()
			if mine ~= asked then
				return
			end
			if not cut.step(STEPS) then
				ui.after(0, step)
				return
			end
			blocks = cut.blocks
			ui.batch(function()
				LINES:set(cut.lines)
				COUNT:set(#cut.blocks)
				PHASE:set("ready")
			end)
		end
		step()
	end, failed)
end

-- (An upload's name: `ab12cd34-Pasted_text_2.txt`.)
function M.number_of(name: string): number?
	local base = string.match(name, "^%x%x%x%x%x%x%x%x%-(.+)$") or name
	return tonumber(string.match(base, "^Pasted[ _]text[ _](%d+)%.txt$"))
end

function M.opener(attachment: { name: string, path: string? }, session: string?): (() -> ())?
	local number = M.number_of(attachment.name)
	local path = attachment.path
	if not number or not path then
		return nil
	end
	return function()
		M.open { title = locale.s("ds.pasted_text.title", { n = number }), path = path, session = session }
	end
end

local function window_size(): (number, number)
	local window = (ui :: any).window
	local state = if type(window) == "table" or type(window) == "userdata" then window.size else nil
	local ok, size = pcall(function(): any
		return if state then state:get() else nil
	end)
	if ok and type(size) == "table" and type(size.width) == "number" and type(size.height) == "number" then
		return math.max(1, size.width), math.max(1, size.height)
	end
	return 640, 480
end

local function subtitle(): string
	local phase = PHASE:get()
	if phase == "failed" then
		return locale.s("ds.pasted_text.failed")
	elseif phase == "loading" then
		return locale.s("ds.pasted_text.loading")
	end
	local count = LINES:get()
	return if count == 1 then locale.s("ds.pasted_text.one_line") else locale.s("ds.pasted_text.lines", { n = count })
end

local function body(): Node
	return ui.list {
		id = "pasted-text-body",
		grow = 1,
		w_full = true,
		min_h = 0,
		py = "$space.2",
		measured = true,
		height = 400,
		count = COUNT,
		overscroll = "contain",
		row_text = function(i: number): string
			return blocks[i] or ""
		end,
		row = function(i: number): Node
			return ui.row {
				w_full = true,
				px = "$space.4",
				ui.text {
					blocks[i] or "",
					literal = true,
					wrap = true,
					w_full = true,
					font = "$font.mono",
					color = "$text.primary",
				},
			}
		end,
	}
end

function M.Host(): Node
	return ui.div {
		theme = { backdrop = "#000000b3" },
		ui.modal(glass.style({
			open = function(): boolean
				return OPEN:get() ~= nil
			end,
			on_close = M.close,
			autofocus = false,
			label = locale.t("ds.pasted_text.viewer"),
			w = function(): number
				local w = window_size()
				return math.min(w * 0.8, CARD_MAX_W)
			end,
			h = function(): number
				local _, h = window_size()
				return h * 0.8
			end,
			max_w = function(): number
				local w = window_size()
				return math.min(w * 0.8, CARD_MAX_W)
			end,
			max_h = function(): number
				local _, h = window_size()
				return h * 0.8
			end,
			p = 0,
			gap = 0,
			rounded = ui.raw(12, "Zeron's popover card"),
			build = function(): Node
				return ui.column {
					id = "pasted-text",
					selectable = true,
					w_full = true,
					h_full = true,
					gap = 0,
					ui.row {
						w_full = true,
						shrink = 0,
						px = "$space.4",
						py = "$zeron.space.snug",
						gap = "$space.2",
						items = "center",
						selectable = false,
						border_sides = { 0, 0, 1, 0 },
						border_color = "$border",
						ui.text {
							function(): string
								local now = OPEN:get()
								return if now then now.title else ""
							end,
							literal = true,
							shrink = 0,
							weight = "$font.weight.medium",
							color = "$text.primary",
						},
						FadeText { subtitle, literal = true, grow = 1, min_w = 0, color = "$text.secondary" },
						ui.button {
							w = 24,
							h = 24,
							p = 0,
							shrink = 0,
							justify = "center",
							items = "center",
							rounded = ui.raw(6, "Zeron's viewer close"),
							bg = "#00000000",
							color = "$text.secondary",
							cursor = "pointer",
							hover = { bg = "$hover", color = "$text.primary" },
							active = { bg = "$pressed", color = "$text.primary" },
							focus = control.ring,
							label = locale.t("ds.pasted_text.close"),
							on_click = M.close,
							Icon { name = "close", size = 14 },
						},
					},
					body(),
				}
			end,
		} :: any, "menu")),
	}
end

return M
