-- Mentions in the draft: `@` a file or folder, `/` a command, `$` a skill
-- (Zeron's triggers, crates/ui/src/composer.rs: `@` and `/` after a space
-- or at a start, `$` not before a digit or a capital, none inside code). As
-- one is typed, a card opens over the pill, as wide as the pill and growing
-- upward only, so it never covers what is typed (crates/ui/src/composer/
-- slash.rs, mentions.rs, popover.rs). Its first row is lit; ↑ and ↓ move,
-- Enter or Tab take the row. Escape or a press outside the card puts it away
-- until the token is edited. A row chosen lands in the draft as its token and
-- a space: a file by its name alone (as Zeron's chips name it), the shortest
-- end of its path that tells it from another of that name; the path it stands
-- for is kept beside the draft (`refs`). A staged attachment lands as its name.
--
-- A token in the draft is styled in type alone (zgx's text has no inline
-- chips, and a wash made a selection of every mention): its trigger greyed
-- as a markdown marker is, a file's name in the text's ink a weight up, a
-- command's in the accent, a skill's in the sidecar branch's violet. Quiet
-- at rest, plainly a reference, calm in a long sentence.
--
--   local m = mentions.use { field = field, value = draft, sources = { ["@"] = files, ["/"] = commands, ["$"] = skills } }
--   m.menu(pill)                     -- the pill, with the card over it
--   on_key = function(e) if m.keys(e) then return true end … end
--   on_caret = m.on_caret, highlight = mentions.highlight

local Icon = require("@self/icon")
local FileIcon = require("@self/file_icons")
local FadeText = require("@self/fade_text")
local Menu = require("@self/menu")
local Rail = require("@self/rail")
local motion = require("@self/motion")
local AgentMark = require("@self/agent_mark")
local status = require("@self/status")

-- A thread's state in the `@` list, in words (Zeron's `thread_status_label`).
local STATUS_WORDS: { [string]: string } = { working = "ds.status.working", input = "ds.status.input", failed = "ds.status.failed", done = "ds.status.done" }

local M = {}

local tokens = require("@self/mention_tokens")
local locale = require("@self/locale")

export type Trigger = tokens.Trigger
export type Item = tokens.Item
export type Sources = tokens.Sources
export type Token = tokens.Token

-- The words' side, for callers that have the one module: the token at a
-- caret, a pick, the list's filter, a file's own name, the highlight.
M.token = tokens.token
M.pick = tokens.pick
M.filter = tokens.filter
M.short = tokens.short
M.highlight = tokens.highlight

-- gpui's popover.rs, in points. A row is its 6 of padding each way round a
-- 13-point line at gpui's 1.618 line height, with the list's gap after it; a
-- heading its 6 and 4 round a 10-point line.
local ROW, ROW_GAP, HEADING = 33, 2, 26
local LIST_MAX, CARD_MAX, ABOVE, WINDOW_MARGIN = 310, 320, 6, 8
local EDGE_FADE = 12
local OPEN_MS = 140
local SKELETON_ROW = 28

export type Use = {
	field: any,
	value: State<string>,
	-- What each trigger lists: a table, or a state of one (a search's
	-- results, arriving as the query is typed). `attachments`: the staged
	-- ones `@` pins first (`{ value, label, detail?, src?, file? }`).
	-- (`any`: a union with the state type is past luau-lsp's type functions.)
	sources: any,
	-- What an empty list calls its contents, by trigger ("commands or skills").
	empty: { [string]: string }?,
	failed: ((trigger: Trigger) -> string?)?,
	-- Called with the item taken (after it lands in the draft).
	-- (And the token it is in the draft: a file's is its shortest name.)
	on_choose: ((item: Item, trigger: Trigger, token: string) -> ())?,
	-- Whether a trigger opens anything (an agent's "Use $ for skills"
	-- off: `$` is ordinary text); by default, every one.
	enabled: ((trigger: Trigger) -> boolean)?,
	-- Whether its composer is in view (by default, it is).
	in_view: (() -> boolean)?,
	on_token: ((token: Token?) -> ())?,
}

local function tracked_upper(label: string): string
	local out: { string } = {}
	for _, code in utf8.codes(string.upper(label)) do
		table.insert(out, utf8.char(code))
	end
	return table.concat(out, "\u{200A}")
end

-- Wires the mentions to a draft: returns what the composer needs.
-- (Called as a component is built: its states are the component's own.)
function M.use(opts: Use)
	local function sources(): any
		local given: any = opts.sources
		if type(given) == "userdata" then
			return given:get()
		end
		return given
	end
	local caret = ui.use_state(0)
	local selecting = ui.use_state(false)
	local dismissed = ui.use_state(nil :: { from: number, to: number, text: string }?)
	local active = ui.use_state(1)
	local offset = ui.use_state(0)
	local seen = ui.use_state(0)
	local list_el = ui.ref()
	local top_edge = ui.ref()
	local card = ui.ref()
	-- Tokens a pick wrote, how many of each: words stay in the draft where gpui's
	-- become chips, so a caret passing back through one opens nothing. One more
	-- of the same words is typed, and opens as any.
	local taken: { [string]: number } = {}
	-- How many times `tok` stands in `text` up to byte `upto` (all, if nil).
	local function uses_of(text: string, tok: string, upto: number?): number
		local n, at = 0, 1
		while true do
			local from = string.find(text, tok, at, true)
			if not from or (upto and from > upto) then
				return n
			end
			n += 1
			at = from + #tok
		end
	end

	-- (Set here, not in ds/tokens: the sidecar's own token exists only while
	-- its column is mounted, and these are its two values.)
	ui.effect(function()
		local kind = ui.theme.kind()
		local ink = if kind == "light" or kind == "high-contrast-light" then "#7f22fe" else "#c4b4ff"
		if ui.theme.get("zeron.composer.token.skill") ~= ink then
			ui.theme.tokens { zeron = { composer = { token = { skill = ink } } } }
		end
	end)

	local token = ui.computed(function(): Token?
		if selecting:get() then
			return nil
		end
		local t = M.token(opts.value:get(), caret:get())
		if t and opts.enabled and not opts.enabled(t.trigger) then
			return nil
		end
		return t
	end)
	local function text_of(t: Token): string
		return string.sub(opts.value:get(), t.from + 1, t.to)
	end
	local shown = ui.computed(function(): { Item }
		local t = token:get()
		if not t then
			return {}
		end
		local all = sources()
		-- (`@task …`: tasks alone, composer.rs `task_scope_query`.)
		local tasks = if t.trigger == "@" then all.tasks or {} else {}
		local function with_heading(list: { Item }): { Item }
			local out: { Item } = {}
			for i, item in list do
				local row = table.clone(item)
				row.heading = if i == 1 then locale.s("ds.composer_mentions.tasks") else nil
				table.insert(out, row)
			end
			return out
		end
		if t.trigger == "@" and string.lower(string.sub(t.query, 1, 5)) == "task " then
			return with_heading(tasks)
		end
		local items = M.filter(all[t.trigger] or {}, t.query)
		if t.trigger ~= "@" or (not all.attachments and not all.threads and #tasks == 0) then
			return items
		end
		local rows = if all.attachments then tokens.attachments(all.attachments, t.query) else {}
		for _, row in rows do
			if not row.detail then
				row.detail = locale.s(if row.src then "ds.composer_mentions.attached_image" else "ds.composer_mentions.attached_file")
			end
		end
		for _, item in items do
			table.insert(rows, item)
		end
		-- (The shipping app's order: Threads, Tasks, then Files.)
		local threads = all.threads or {}
		if #threads == 0 and #tasks == 0 then
			return rows
		end
		local out: { Item } = {}
		for i, th in threads do
			local row = table.clone(th)
			row.heading = if i == 1 then locale.s("ds.composer_mentions.threads") else nil
			table.insert(out, row)
		end
		for _, row in with_heading(tasks) do
			table.insert(out, row)
		end
		for i, row in rows do
			if i == 1 then
				row = table.clone(row)
				row.heading = locale.s("ds.composer_mentions.files")
			end
			table.insert(out, row)
		end
		return out
	end)
	local layout = ui.computed(function(): { heights: { number }, total: number }
		local heights: { number } = {}
		local total = 0
		for i, item in shown:get() do
			local h = ROW + ROW_GAP + (if item.heading then HEADING else 0)
			heights[i] = h
			total += h
		end
		return { heights = heights, total = total }
	end)
	local function loading(): boolean
		local t = token:get()
		return t ~= nil and sources()[t.trigger] == nil and #shown:get() == 0
	end
	local function is_open(t: Token?): boolean
		if not t then
			return false
		end
		-- (Its composer in view: Settings standing over the page, it has nothing to
		-- hang from.)
		if opts.in_view and not opts.in_view() then
			return false
		end
		local text = text_of(t)
		if taken[text] and uses_of(opts.value:peek(), text, t.from + 1) <= taken[text] then
			return false
		end
		local d = dismissed:get()
		return not (d and d.from == t.from and d.to == t.to and d.text == text)
	end
	local function open(): boolean
		return is_open(token:get())
	end
	local function remember(t: Token)
		dismissed:set({ from = t.from, to = t.to, text = text_of(t) })
	end
	-- The window going to the back closes every popover, as AppKit's do: not a
	-- dismissal. Back at the front, the list that was open opens again. (Which comes
	-- first, the window's state or the popover's closing, isn't for this to rely on.)
	local closed_from, closed_at = -1, -math.huge
	local away_from = -1
	local function closed(t: Token)
		remember(t)
		closed_from, closed_at = t.from, os.clock()
		if ui.window.active:peek() == false then
			away_from = t.from
		end
	end
	local was_active = ui.window.active:peek()
	ui.effect(function()
		local now = ui.window.active:get()
		if now == was_active then
			return
		end
		was_active = now
		if not now then
			if os.clock() - closed_at < 0.5 then
				away_from = closed_from
			end
			return
		end
		local t = token:peek()
		local d = dismissed:peek()
		if t and d and away_from == t.from and d.from == t.from then
			dismissed:set(nil)
		end
		away_from = -1
	end)
	ui.effect(function()
		local t = token:get()
		local d = dismissed:peek()
		if d and (not t or t.from ~= d.from or t.to ~= d.to or text_of(t) ~= d.text) then
			dismissed:set(nil)
		end
	end)
	-- A new token (or a new query) lights the first row again, at the top.
	local last_from: number?, last_query: string? = nil, nil
	ui.effect(function()
		local t = token:get()
		local from, query = if t then t.from else nil, if t then t.query else nil
		if from == last_from and query == last_query then
			return
		end
		last_from, last_query = from, query
		active:set(1)
		offset:set(0)
	end)
	ui.effect(function()
		if active:peek() > math.max(#shown:get(), 1) then
			active:set(1)
		end
	end)
	if opts.on_token then
		local tell = opts.on_token
		ui.effect(function()
			local t = token:get()
			tell(if is_open(t) then t else nil)
		end)
	end
	-- What each token in the draft stands for (a file's name, its path).
	-- A token that leaves the draft (replaced, deleted, edited) takes its
	-- path with it.
	local refs = ui.use_state({} :: { [string]: string })
	ui.effect(function()
		local now = opts.value:get()
		for tok, n in table.clone(taken) do
			local left = uses_of(now, tok)
			if left == 0 then
				taken[tok] = nil
			elseif left < n then
				taken[tok] = left
			end
		end
		local kept = refs:peek()
		local stale: { string } = {}
		for tok in kept do
			if not string.find(now, tok, 1, true) then
				table.insert(stale, tok)
			end
		end
		if #stale > 0 then
			local next = table.clone(kept)
			for _, tok in stale do
				next[tok] = nil
			end
			refs:set(next)
		end
	end)
	local function choose(item: Item)
		local t = token:peek()
		local el = opts.field:peek()
		if not t or not el or item.blocked then
			return
		end
		local now = opts.value:peek()
		local from, to, text, tok = M.pick(now, t.to, item, sources())
		if not from or not to or not text or not tok then
			return
		end
		-- The path the new token stands for, in place of the old one's.
		local old = string.sub(now, from + 1, to)
		local next = table.clone(refs:peek())
		next[old] = nil
		if item.file and not item.attached and item.value ~= string.sub(tok, 2) then
			next[tok] = item.value
		end
		refs:set(next)
		if tok ~= "" and not item.attached then
			taken[tok] = (taken[tok] or 0) + 1
		end
		el:set_selection_bytes(from, to)
		el:replace_selection(text)
		if opts.on_choose then
			opts.on_choose(item, t.trigger, tok)
		end
	end
	local function keys(e: any): boolean
		if not open() then
			return false
		end
		local list = shown:peek()
		if e.key == "ArrowDown" or e.key == "ArrowUp" then
			if #list == 0 then
				return false
			end
			local step = if e.key == "ArrowDown" then 1 else -1
			local i = (active:peek() - 1 + step) % #list + 1
			active:set(i)
			local el = list_el:peek()
			if el then
				el:scroll_to_row(i)
			end
			return true
		elseif e.key == "Tab" and e.shift then
			return true
		elseif e.key == "Enter" or e.key == "Tab" then
			local item = list[active:peek()]
			if not item then
				return false
			end
			-- (Rows of the search before: the query has moved on, so the key takes nothing yet.)
			local t = token:peek()
			if item.searched and item.query ~= nil and t and item.query ~= t.query then
				return true
			end
			choose(item)
			return true
		elseif e.key == "Escape" then
			local t = token:peek()
			if t then
				remember(t)
			end
			return true
		end
		return false
	end
	local function on_caret(_line: number, _column: number, anchor: number, focus: number)
		ui.batch(function()
			caret:set(focus)
			selecting:set(anchor ~= focus)
		end)
	end
	local function key_of(item: Item): string
		return item.key or item.value
	end
	local function descendant(): string?
		if not open() then
			return nil
		end
		local item = shown:get()[active:get()]
		return if item then `mention-{key_of(item)}` else nil
	end
	-- A press in the card lets focus go, the toolkit's default where nothing
	-- under it takes focus: not a dismissal, and the keys come back.
	local in_card = false
	local function pressed_in_card()
		in_card = true
		ui.after(0, function()
			in_card = false
			local el = opts.field:peek()
			if el then
				el:focus({ scroll = false })
			end
		end)
	end
	local function thread_row(item: Item, index: State<number>, lit: () -> boolean, over: State<boolean>): Node
		local th = (item :: any).thread
		local st = th.status
		local badge: Node? = nil
		if STATUS_WORDS[st or ""] then
			badge = ui.row {
				shrink = 0,
				gap = ui.raw(4, "mentions.rs status badge"),
				items = "center",
				color = `$zeron.status.{st}`,
				if st == "working" then status.Spinner { size = 11, ink = "$zeron.status.working" }
					elseif st == "done" then Icon { name = "check", size = 11 }
					else ui.div { w = 6, h = 6, rounded = "$radius.full", bg = `$zeron.status.{st}` },
				ui.text { locale.s(STATUS_WORDS[st]), wrap = false, text_size = ui.raw(10, "mentions.rs status badge"), weight = "$font.weight.medium" },
			}
		end
		local body: { [any]: any } = {
			id = `mention-{key_of(item)}`,
			role = "option",
			selected = lit,
			label = if badge then `{item.label} {locale.s(STATUS_WORDS[st])}` else item.label,
			relative = true,
			h = ROW,
			shrink = 0,
			w_full = true,
			px = "$space.2",
			gap = ui.raw(8, "mentions.rs thread row"),
			items = "center",
			rounded = ui.raw(7, "the card's 12 less its edge and inset"),
			bg = function(): string
				return if lit() then "$zeron.popup.selected" else "#00000000"
			end,
			cursor = "pointer",
			on_hover = function(inside: boolean)
				over:set(inside)
			end,
			on_click = function()
				choose(item)
			end,
		}
		local parts: { Node } = {
			ui.div {
				absolute = true,
				left = 0,
				right = 0,
				top = 0,
				bottom = 0,
				rounded = ui.raw(7, "the row's corner"),
				bg = "$zeron.popup.selected",
				pointer_events = "none",
				aria_hidden = true,
				opacity = motion.tween(
					ui.computed(function(): number
						return if over:get() and not lit() then 1 else 0
					end),
					motion.hover()
				),
			},
			ui.div {
				w = 16,
				h = 16,
				shrink = 0,
				items = "center",
				justify = "center",
				color = "$text.secondary",
				if th.mark or th.icon then AgentMark.Mark { label = th.mark or item.label, mark = th.mark, icon = th.icon, seed = th.mark, size = 14, bare = true } else Icon { name = "chat-round-line", size = 14 },
			},
			FadeText { item.label, literal = true, grow = 1, shrink = 1, basis = 0, min_w = 0, text_size = ui.raw(13, "mentions.rs thread title"), color = "$text.primary" },
		}
		if badge then
			table.insert(parts, badge)
		end
		if item.detail and item.detail ~= "" then
			table.insert(parts, FadeText { item.detail, literal = true, shrink = 0, max_w = 160, text_size = ui.raw(12.5, "mentions.rs thread project"), color = "$text.secondary" })
		end
		for i, n in parts do
			body[i] = n
		end
		return ui.column {
			w_full = true,
			px = "$space.1",
			if item.heading
				then ui.text {
					tracked_upper(item.heading),
					literal = true,
					h = HEADING,
					shrink = 0,
					px = "$space.2",
					pt = "$zeron.space.tight",
					pb = "$space.1",
					text_size = ui.raw(10, "gpui's menu heading"),
					weight = "$font.weight.medium",
					color = "$text.secondary",
				}
				else nil,
			ui.row(body),
		}
	end
	local function row(item: Item, index: State<number>): Node
		local over = ui.use_state(false)
		local function lit(): boolean
			return active:get() == index:get()
		end
		local thread = (item :: any).thread
		if thread then
			return thread_row(item, index, lit, over)
		end
		local lead: Node
		if item.src then
			lead = ui.image { src = item.src, w = 16, h = 16, rounded = "$radius.sm", fit = "cover" }
		elseif item.file or item.dir then
			lead = FileIcon { name = item.label, dir = item.dir, size = 16 }
		else
			lead = Icon { name = item.icon or "command", size = 16 }
		end
		local has_detail = item.detail ~= nil and item.detail ~= ""
		return ui.column {
			w_full = true,
			px = "$space.1",
			if item.heading
				then ui.text {
					tracked_upper(item.heading),
					literal = true,
					h = HEADING,
					shrink = 0,
					px = "$space.2",
					pt = "$zeron.space.tight",
					pb = "$space.1",
					text_size = ui.raw(10, "gpui's menu heading"),
					weight = "$font.weight.medium",
					color = "$text.secondary",
				}
				else nil,
			ui.row {
				id = `mention-{key_of(item)}`,
				role = "option",
				selected = lit,
				label = item.label,
				relative = true,
				h = ROW,
				shrink = 0,
				w_full = true,
				px = "$space.2",
				gap = "$space.2",
				items = "center",
				rounded = ui.raw(7, "the card's 12 less its edge and inset"),
				opacity = if item.blocked then 0.45 elseif item.terminal_only then 0.55 else 1,
				bg = function(): string
					return if lit() then "$zeron.popup.selected" else "#00000000"
				end,
				cursor = if item.blocked then "default" else "pointer",
				on_hover = function(inside: boolean)
					over:set(inside)
				end,
				on_click = function()
					choose(item)
				end,
				ui.div {
					absolute = true,
					left = 0,
					right = 0,
					top = 0,
					bottom = 0,
					rounded = ui.raw(7, "the row's corner"),
					bg = "$zeron.popup.selected",
					pointer_events = "none",
					aria_hidden = true,
					opacity = motion.tween(
						ui.computed(function(): number
							return if over:get() and not lit() then 1 else 0
						end),
						motion.hover()
					),
				},
				ui.div { w = 16, h = 16, shrink = 0, items = "center", justify = "center", color = "$text.secondary", lead },
				FadeText {
					item.label,
					literal = true,
					shrink = if has_detail then 0 else 1,
					grow = if has_detail then 0 else 1,
					min_w = 0,
					max_w = if has_detail then "55%" else nil,
					text_size = ui.raw(13, "gpui's completion row"),
					weight = "$font.weight.medium",
					color = "$text.primary",
				},
				if has_detail
					then FadeText {
						item.detail,
						literal = true,
						grow = 1,
						shrink = 1,
						min_w = 0,
						text_size = ui.raw(12.5, "gpui's completion detail"),
						color = "$text.secondary",
					}
					else nil,
				if item.terminal_only
					then ui.div {
						shrink = 0,
						px = "$zeron.space.tight",
						rounded = "$radius.sm",
						bg = "$hover",
						ui.text {
							locale.s("ds.composer_mentions.terminal"),
							text_size = ui.raw(10.5, "gpui's terminal tag"),
							color = "$text.secondary",
						},
					}
					else nil,
			},
		}
	end
	local function empty_words(): string
		local t = token:get()
		if not t then
			return ""
		end
		local says = opts.empty or {}
		local what = says[t.trigger]
		if not what and t.trigger == "@" then
			return locale.s(if t.query == "" then "ds.composer_mentions.empty.files" else "ds.composer_mentions.empty.files_matching")
		end
		local all = sources()[t.trigger]
		local none = all == nil or #all == 0
		if not what then
			if none then
				return locale.s(if t.trigger == "/" then "ds.composer_mentions.empty.commands" else "ds.composer_mentions.empty.skills")
			end
			what = locale.s(if t.trigger == "/" then "ds.composer_mentions.what.commands" else "ds.composer_mentions.what.skills")
		end
		return locale.s(if none then "ds.composer_mentions.none_available" else "ds.composer_mentions.none_matching", { what = what })
	end
	local function failed_words(): string?
		local t = token:get()
		if not t or not opts.failed then
			return nil
		end
		-- (A staged attachment still answers: gpui says nothing of the search.)
		local first = shown:get()[1]
		if t.trigger == "@" and first and first.attached then
			return nil
		end
		return opts.failed(t.trigger)
	end
	local function title(): string
		local t = token:get()
		return locale.s(
			if t and t.trigger == "@"
				then "ds.composer_mentions.files"
				elseif t and t.trigger == "$" then "ds.composer_mentions.skills"
				else "ds.composer_mentions.commands"
		)
	end
	local function card_body(): Node
		local skeleton: { Node } = {}
		for _ = 1, 3 do
			table.insert(
				skeleton,
				ui.div {
					w_full = true,
					h = SKELETON_ROW,
					shrink = 0,
					rounded = "$radius.md",
					bg = "$hover",
					aria_hidden = true,
					opacity = ui.loop { from = 0.35, to = 0.75, duration = 1200, yoyo = true, ease = "inOut(2)", fps = 30 },
				}
			)
		end
		return ui.column {
			id = "mention-panel",
			w_full = true,
			min_h = 0,
			gap = 0,
			pt = "$space.1",
			-- (The list's last row brings its own gap.)
			pb = "$zeron.space.hair",
			on_press = pressed_in_card,
			on_mount = function(el: Element)
				anime.animate(card:peek() or el, motion.params({ opacity = { 0.3, 1 }, translateY = { -2, 0 }, duration = OPEN_MS, ease = motion.EASE }))
			end,
			ui.show(function(): boolean
				return failed_words() ~= nil
			end, ui.row {
				id = "mention-failed",
				w_full = true,
				shrink = 0,
				px = "$space.4",
				py = "$zeron.space.snug",
				ui.text {
					function(): string
						return failed_words() or ""
					end,
					wrap = true,
					text_size = "$font.size.sm",
					color = "$danger",
				},
			}),
			ui.show(loading, ui.column { id = "mention-skeleton", w_full = true, shrink = 0, px = "$space.1", py = "$space.1", gap = "$zeron.space.tight", table.unpack(skeleton) }),
			ui.show(function(): boolean
				return not loading() and #shown:get() == 0 and failed_words() == nil
			end, ui.row {
				id = "mention-empty",
				w_full = true,
				shrink = 0,
				px = "$space.4",
				py = "$zeron.space.snug",
				ui.text { empty_words, text_size = "$font.size.sm", color = "$text.secondary" },
			}),
			ui.div {
				relative = true,
				w_full = true,
				min_h = 0,
				shrink = 1,
				h = function(): number
					return math.min(layout:get().total, LIST_MAX)
				end,
				on_resize = function(size: { w: number, h: number })
					seen:set(size.h)
				end,
				ui.list {
					id = "mention-list",
					ref = list_el,
					w_full = true,
					h_full = true,
					data = shown,
					key = key_of,
					heights = function(): { number }
						return layout:get().heights
					end,
					row = row,
					offset = offset,
					overscroll = "contain",
					role = "listbox",
					label = title,
					fade = function(): { number }
						local most = math.max(0, layout:get().total - seen:get())
						local at = offset:get()
						return { math.min(at, EDGE_FADE), math.min(math.max(0, most - at), EDGE_FADE) }
					end,
				},
				Rail {
					offset = offset,
					extent = function(): number
						return layout:get().total
					end,
					view = seen,
				},
			},
		}
	end
	local function close_on(t: Token?)
		if t and is_open(t) then
			closed(t)
		end
	end
	-- The most the card can be and stay above the pill: the toolkit flips or
	-- clamps a card with no room, and either would cover the draft.
	local function room(): number
		token:get()
		local edge = top_edge:peek()
		local y = if edge then edge.y else nil
		return if y then math.max(0, math.min(CARD_MAX, y - ABOVE - WINDOW_MARGIN)) else CARD_MAX
	end
	-- Built again each opening, so the open motion plays and nothing stale
	-- shows. (`build` on a popover wants `open` as a state: this one's a function.)
	local openings, was_open = 0, false
	local function opening(): number?
		local now = open()
		if now and not was_open then
			openings += 1
		end
		was_open = now
		return if now then openings else nil
	end
	local panel = Menu.panel {
		id = "mention-popover",
		ref = card,
		open = open,
		on_close = function()
			close_on(token:peek())
		end,
		-- (Its anchor holds no field, so this places it at the anchor's box. It
		-- is what keeps the keys in the draft: a plain popover takes them.)
		at_caret = true,
		autofocus = false,
		match_anchor_width = true,
		w = 0,
		max_h = room,
		rounded = "$radius.xl",
		placement = { side = "top", align = "start", offset = ABOVE },
		anchor = ui.div { ref = top_edge, absolute = true, left = 0, right = 0, top = 0, h = 0, aria_hidden = true },
		ui.presence(opening, card_body, { w_full = true, min_h = 0, shrink = 1 }),
	}
	local function menu(pill: Node): Node
		return ui.div {
			relative = true,
			w_full = true,
			-- A press on the pill is outside the card; a press anywhere else,
			-- with no hook of its own, shows as the draft losing focus.
			on_press = function()
				local t = token:peek()
				if t and is_open(t) then
					remember(t)
				end
			end,
			on_blur = function()
				if not in_card then
					close_on(token:peek())
				end
			end,
			pill,
			ui.popover(panel),
		}
	end
	return {
		token = token,
		shown = shown,
		active = active,
		open = open,
		keys = keys,
		on_caret = on_caret,
		descendant = descendant,
		choose = choose,
		menu = menu,
		refs = refs,
	}
end

return M
