-- crates/ui shell/switcher.rs.

local ds = require("@ds")
local SessionRow = require("@ds/session_row")
local motion = require("@ds/motion")
local sessions = require("@account/sessions")
local route = require("@app/screens/shell/route")
local view = require("@app/screens/shell/view")
local tabs = require("@app/screens/shell/session_tabs")
local chat_shared = require("@app/screens/chat/shared")
local refocus = require("@app/screens/shell/refocus")
local swipe = require("@app/screens/shell/swipe")
local Pictures = require("@app/screens/shell/switcher_pictures")
local boundary = require("@app/screens/shell/boundary")

local M = {}

local TILE_W, TILE_H, THUMB_PX = 166, 108, 332
local TILE_GAP, RIM, TITLE_GAP, TITLE_H = 24, 8, 10, 16
local ROW_H = TILE_H + TITLE_GAP + TITLE_H + 2 * RIM
local PAGE, BAND_H = 5, 430
local ARRIVE = { duration = 180, ease = "cubicBezier(.5,1,.89,1)" }
local SLIDE = { duration = 60, ease = "cubicBezier(.33,1,.68,1)" }
local PICTURES = 40
-- (A chat left sooner than this has hardly drawn: no picture, and none of its cost when
-- going through them fast.)
local SETTLED_MS = 300
-- (A session's card shuts on the press that goes to another chat; the frame showing still
-- has it.)
local CARD_LINGER_MS = 100
-- (A quick press goes to the chat before with no cards: they come up once Control has been held this long, or at a second press.)
local HOLD_MS = 120

ui.theme.tokens {
	zeron = {
		switcher = {
			plate = "alpha({text.primary}, 0.16)",
			plate_edge = "alpha({text.primary}, 0.2)",
			edge = "alpha({text.primary}, 0.1)",
			chip = "{zeron.menu.solid}",
			veil = "alpha({zeron.window}, 0.3)",
			shadow = "0 4 14 0 #00000038",
		},
	},
}

M.window = ui.persist("shell_switcher_window", "all")
M.limit = ui.persist("shell_switcher_limit", 0)
M.cycles_tabs = ui.persist("shell_switcher_cycles_tabs", false)
M.WINDOWS = {
	{ id = "all", hours = nil },
	{ id = "1h", hours = 1 },
	{ id = "2h", hours = 2 },
	{ id = "4h", hours = 4 },
	{ id = "8h", hours = 8 },
	{ id = "12h", hours = 12 },
	{ id = "24h", hours = 24 },
	{ id = "3d", hours = 72 },
	{ id = "7d", hours = 168 },
}
local function cutoff(now: number): number?
	for _, w in M.WINDOWS do
		if w.id == M.window:peek() and w.hours then
			return now - w.hours * 3600
		end
	end
	return nil
end

local cards = ui.state("shell_switcher_cards", nil :: { string }?)
local card_list = ui.computed(function(): { string }
	return cards:get() or {}
end)
local hi = ui.state("shell_switcher_hi", 1)
local up = ui.state("shell_switcher_up", false)
local raised = 0
local held = ui.modifiers

local pictures = Pictures.Book(PICTURES)
local column: any = nil
function M.set_column(ref: any)
	column = ref
end
local card_was_up = false
local card_gone_at = -math.huge
ui.effect(function()
	local sidebar: any = boundary.load("@app/screens/shell/sidebar")
	local up = sidebar ~= nil and sidebar.card:get() ~= nil
	if card_was_up and not up then
		card_gone_at = ui.now()
	end
	card_was_up = up
end)
local shown_since = ui.now()
-- (A wrong picture is worse than an older one, or none.)
local function picture_of(id: string): boolean
	local el = column and column.element
	if not el or el:covered() or swipe.showing() or card_was_up or ui.now() - card_gone_at < CARD_LINGER_MS or ui.now() - shown_since < SETTLED_MS then
		return false
	end
	pictures.take(id, function(slot: string): string?
		local ok, src = pcall(el.snapshot, el, THUMB_PX, slot)
		return if ok then src else nil
	end)
	return true
end
local leaving: string? = route.session:peek() and (route.session:peek() :: any).id
local pressed_on: string? = nil
local pressed_at = -math.huge
local on_chat = route.current:peek().name == "session"
ui.effect(function()
	local s = route.session:get()
	local id = if s then s.id else nil
	-- (A quick press took its picture as it was pressed: not again as it lands.)
	if leaving and id ~= leaving and on_chat and not (leaving == pressed_on and ui.now() - pressed_at < HOLD_MS) then
		picture_of(leaving)
	end
	if id ~= leaving then
		shown_since = ui.now()
	end
	leaving = id
	on_chat = route.current:get().name == "session"
end)

local function chat_row(id: string): any
	for _, c in sessions.now:peek().chats do
		if c.id == id then
			return c
		end
	end
	return nil
end

local function here_now(): string?
	local s = route.session:peek()
	return if s then s.id else nil
end

local function strip(live: boolean): { string }
	local out: { string } = {}
	local here = here_now()
	for _, key in tabs.shown:peek() do
		local id = tabs.face(key)
		local c = chat_row(id)
		if c and (not live or c.status or id == here) then
			table.insert(out, id)
		end
	end
	return out
end

function M.order(recent: { string }, sidebar: { string }, live_tabs: { string }, updated: (id: string) -> number?, since: number?, limit: number): { string }
	local candidates: { string } = {}
	for _, id in sidebar do
		local at = updated(id)
		if at and (not since or at >= since) then
			table.insert(candidates, id)
		end
	end
	for _, id in live_tabs do
		if not table.find(candidates, id) then
			table.insert(candidates, id)
		end
	end
	local out: { string } = {}
	for _, id in recent do
		if table.find(candidates, id) then
			table.insert(out, id)
		end
	end
	for _, id in candidates do
		if not table.find(out, id) then
			table.insert(out, id)
		end
	end
	if limit > 0 and #out > limit then
		local capped = table.move(out, 1, limit, 1, {})
		for _, id in live_tabs do
			if not table.find(capped, id) then
				table.insert(capped, id)
			end
		end
		return capped
	end
	return out
end

local function order_now(sidebar: { string }, going_home: boolean): { string }
	local list = M.order(tabs.recent:peek(), sidebar, strip(true), function(id: string): number?
		local c = chat_row(id)
		return if c then c.updated else nil
	end, cutoff(os.time()), M.limit:peek())
	local home = going_home or route.chat:peek().name == "home"
	local first = if home then tabs.tab_of(route.draft:peek()) else nil
	if first then
		table.insert(list, 1, first)
	else
		-- (The first press goes one on from where it is: where it is comes first.)
		local at = table.find(list, here_now() or "")
		if at and at > 1 then
			table.insert(list, 1, table.remove(list, at) :: string)
		end
	end
	for _, key in tabs.drafts:peek() do
		local tab = tabs.tab_of(key)
		if tab ~= first then
			table.insert(list, tab)
		end
	end
	return list
end

local function here_tab(): string?
	return if route.chat:peek().name == "home" then tabs.tab_of(route.draft:peek()) else here_now()
end

local function go(id: string?)
	local draft = tabs.draft_in(id)
	if draft then
		route.open_draft(draft)
	elseif id then
		route.open_session(id)
	end
	refocus()
end

function M.commit()
	local list = cards:peek()
	if not list then
		return
	end
	local id = list[hi:peek()]
	cards:set(nil)
	local here = here_tab()
	-- (A chat deleted since the cards came up is no place to land.)
	if id ~= here and (tabs.draft_in(id) or chat_row(id)) then
		go(id)
	end
end

function M.close()
	cards:set(nil)
end

local active: any = (ui :: any).window and (ui :: any).window.active
ui.effect(function()
	local down = held.control:get() or held.meta:get() or held.alt:get()
	-- (The window losing focus lets go of what's held here; nobody let go on a card.)
	local away = active ~= nil and active:get() == false
	-- (Up with nothing held, a tap or the Go menu: it lands at once, as Zeron's.)
	if not cards:get() then
		return
	end
	if away then
		M.close()
	elseif not down then
		M.commit()
	end
end)

local function raise(list: { string }, start: number, where: string)
	local s = route.session:peek()
	if s and where == "session" and picture_of(s.id) then
		pressed_on, pressed_at = s.id, ui.now()
	end
	raised += 1
	local mine = raised
	ui.batch(function()
		hi:set(start)
		up:set(false)
		cards:set(list)
	end)
	ui.after(HOLD_MS, function()
		if raised == mine and cards:peek() then
			up:set(true)
		end
	end)
end

function M.step(by: number): boolean
	local list = cards:peek()
	if list then
		ui.batch(function()
			hi:set((hi:peek() - 1 + by) % #list + 1)
			up:set(true)
		end)
		return true
	end
	local where = route.current:peek().name
	-- (Settings is a modal itself, left below: only what's over it keeps the press.)
	local settings: any = if where == "settings" then boundary.load("@app/screens/shell/settings") else nil
	local el = if settings then settings.inside.element else column and column.element
	if view.palette:peek() or (el and el:overlaid()) then
		return true
	end
	if where == "settings" then
		route.leave_settings()
	elseif where == "plugin" then
		route.new_chat()
	end
	if M.cycles_tabs:peek() then
		local strip_now = strip(false)
		for _, key in tabs.drafts:peek() do
			table.insert(strip_now, tabs.tab_of(key))
		end
		if #strip_now == 0 then
			return true
		end
		local here = here_tab()
		local at = if here then table.find(strip_now, here) else nil
		if at and #strip_now == 1 then
			return true
		end
		local i = if at then (at - 1 + by) % #strip_now + 1 elseif by > 0 then 1 else #strip_now
		raise(strip_now, i, where)
		return true
	end
	local sidebar: any = boundary.load("@app/screens/shell/sidebar")
	local order = order_now(if sidebar then sidebar.order:peek() else {}, where == "plugin")
	if #order == 0 then
		return true
	elseif #order == 1 then
		if route.chat:peek().name == "home" and not tabs.draft_in(order[1]) then
			go(order[1])
		end
		return true
	end
	raise(order, (1 - 1 + by) % #order + 1, where)
	return true
end

local page = ui.computed(function(): number
	return (hi:get() - 1) // PAGE
end)
local page_list = ui.computed(function(): { string }
	local list = card_list:get()
	local first = page:get() * PAGE
	return table.move(list, first + 1, math.min(first + PAGE, #list), 1, {})
end)

local function window_size(): (number, number)
	local size = (ui.window.size :: any):get()
	return math.max(1, size.width or 0), math.max(1, size.height or 0)
end

local function row_w(): number
	local n = #page_list:get()
	return n * TILE_W + math.max(n - 1, 0) * TILE_GAP + 2 * RIM
end

local function Tile(id: string): Node
	local draft = tabs.draft_in(id)
	local c = if draft then nil else chat_row(id)
	local function lit(): boolean
		return card_list:get()[hi:get()] == id
	end
	local function pick()
		local at = table.find(card_list:peek(), id)
		if at and hi:peek() ~= at then
			hi:set(at)
		end
	end
	local picture = pictures.src(id)
	local text = if draft then chat_shared.draft_of(draft).text elseif c then c.preview else nil
	local title: any = if c then c.title else ui.t("shell.tabs.new_session")
	return ui.column {
		id = `switcher-card-{id}`,
		role = "button",
		label = title,
		selected = lit,
		shrink = 0,
		w = TILE_W,
		gap = ui.raw(TITLE_GAP, "Yab's gap under the picture"),
		cursor = "pointer",
		-- (Moved over it, not its coming up under a pointer at rest, which would be where a quick press lands.)
		on_pointer_move = pick,
		on_click = function()
			pick()
			M.commit()
		end,
		ui.div {
			relative = true,
			w = TILE_W,
			h = TILE_H,
			overflow = "hidden",
			rounded = ui.raw(13, "Yab's tile corner"),
			bg = "$zeron.window",
			border = 1,
			border_over = true,
			border_color = "$zeron.switcher.edge",
			shadow = "$zeron.switcher.shadow",
			if picture
				then ui.image { src = picture, w = TILE_W, h = TILE_H, fit = "cover", object_position = { 0.5, 0 } }
				else ui.div {
					w_full = true,
					h_full = true,
					p = ui.raw(10, "Zeron's text's inset"),
					pb = ui.raw(46, "clear of the mark plate"),
					overflow = "hidden",
					color = "$text.secondary",
					ui.text { if text and text ~= "" then text else ui.t("shell.switcher.no_messages"), wrap = true, text_size = "$font.size.xs" },
				},
			ui.div {
				absolute = true,
				left = ui.raw(8, "Yab's mark's inset"),
				bottom = ui.raw(8, "Yab's mark's inset"),
				w = ui.raw(30, "Yab's mark plate"),
				h = ui.raw(30, "Yab's mark plate"),
				rounded = ui.raw(8, "Yab's mark plate's corner"),
				bg = "$zeron.switcher.chip",
				border = 1,
				border_color = "$zeron.switcher.edge",
				items = "center",
				justify = "center",
				if c
					then SessionRow.mark(function(): any
						return c
					end, function(): boolean
						return true
					end)
					else ui.div { color = "$warning", ds.Icon { name = "pen", size = 13 } },
			},
		},
		ui.row {
			w = TILE_W,
			h = ui.raw(TITLE_H, "Yab's title line"),
			px = ui.raw(8, "Yab's title inset, level with the mark"),
			items = "center",
			color = function(): string
				return if lit() then "$text.primary" else "$text.tertiary"
			end,
			ds.FadeText { title, grow = 1, min_w = 0, text_size = ui.raw(13, "Yab's tile title") },
		},
	}
end

local function Row(): Node
	local function fit(): number
		local w = window_size()
		return math.min(1, (w - 48) / row_w())
	end
	return ui.div {
		absolute = true,
		left = function(): number
			local w = window_size()
			return (w - row_w()) / 2
		end,
		top = function(): number
			local _, h = window_size()
			return (h - ROW_H) / 2
		end,
		w = row_w,
		h = ui.raw(ROW_H, "Yab's row: a tile, its title and the plate's rim"),
		scale = fit,
		relative = true,
		-- (Built again on a page flip, so the plate lands there at once rather than sliding.)
		ui.presence(function(): number
			return page:get()
		end, function(): Node
			return ui.div {
				absolute = true,
				left = 0,
				top = 0,
				w = ui.raw(TILE_W + 2 * RIM, "Yab's plate: the tile and its rim"),
				h = ui.raw(TILE_H + 2 * RIM, "Yab's plate: the tile and its rim"),
				rounded = ui.raw(21, "Yab's plate's corner, concentric with the tile's"),
				bg = "$zeron.switcher.plate",
				border = 1,
				border_color = "$zeron.switcher.plate_edge",
				x = motion.tween(
					ui.computed(function(): number
						return (hi:get() - 1) % PAGE * (TILE_W + TILE_GAP)
					end),
					SLIDE
				),
			}
		end),
		ui.row {
			p = ui.raw(RIM, "Yab's plate's rim"),
			gap = ui.raw(TILE_GAP, "Yab's gap between tiles"),
			items = "start",
			ui.each(page_list, function(id: string): Node
				return Tile(id)
			end, { key = function(id: string): string
				return id
			end, direction = "row", gap = ui.raw(TILE_GAP, "Yab's gap between tiles") }),
		},
	}
end

local function showing(): boolean
	return cards:get() ~= nil and up:get()
end
local arrive = ui.state("shell_switcher_arrive", 0)
local arriving: any = nil
local was_showing = false
ui.effect(function()
	local on = showing()
	if on == was_showing then
		return
	end
	was_showing = on
	if arriving then
		arriving:cancel()
		arriving = nil
	end
	arrive:set(0)
	if on then
		arriving = anime.animate(arrive, {
			value = 1,
			duration = if motion.reduced_now() then motion.FADE else ARRIVE.duration,
			ease = ARRIVE.ease,
		})
	end
end)
local function shown(): number
	return math.clamp(arrive:get(), 0, 1)
end

-- (zgx masks a fill but not a backdrop blur: the band's blur softens in steps, as Yab's stacked blurs do.)
local function Band(h: number, blur: any): Node
	return ui.div {
		absolute = true,
		left = 0,
		right = 0,
		top = function(): number
			local _, wh = window_size()
			return (wh - h) / 2
		end,
		h = ui.raw(h, "a step of Yab's blurred band about the row"),
		bg = ui.raw("#00000001", "a backdrop blur paints only over a fill"),
		backdrop_blur = blur,
	}
end

function M.Overlay(): Node
	return ui.div {
		theme = { backdrop = "#00000000" },
		ui.modal {
			open = showing,
			on_close = function()
				M.close()
			end,
			label = ui.t("shell.switcher.label"),
			placement = { x = "start", y = "start", inset = 0 },
			w = function(): number
				return (window_size())
			end,
			h = function(): number
				local _, h = window_size()
				return h
			end,
			max_w = function(): number
				return (window_size())
			end,
			max_h = function(): number
				local _, h = window_size()
				return h
			end,
			p = 0,
			gap = 0,
			rounded = 0,
			bg = "#00000000",
			border = 0,
			shadow = "$zeron.dialog.stage",
			build = function()
				return ui.div {
					id = "session-switcher",
					w_full = true,
					h_full = true,
					relative = true,
					autofocus = true,
					focusable = true,
					focus = { outline = 0, border = 0 },
					-- (Tab moves on, not to the next control, and no command runs under it.)
					capture_keys = true,
					on_key = function(e: any): boolean?
						if e.key == "Tab" then
							M.step(if e.shift then -1 else 1)
						elseif e.key == "ArrowLeft" or e.key == "ArrowUp" then
							M.step(-1)
						elseif e.key == "ArrowRight" or e.key == "ArrowDown" then
							M.step(1)
						elseif e.key == "Escape" then
							M.close()
						elseif e.key == "Enter" then
							M.commit()
						end
						return true
					end,
					ui.div {
						absolute = true,
						left = 0,
						top = 0,
						right = 0,
						bottom = 0,
						opacity = shown,
						aria_hidden = true,
						on_click = function()
							M.close()
						end,
						ui.div {
							absolute = true,
							left = 0,
							top = 0,
							right = 0,
							bottom = 0,
							bg = ui.raw("#00000001", "a backdrop blur paints only over a fill"),
							backdrop_blur = ui.raw(1.3, "Yab's blur over the whole window"),
						},
						Band(BAND_H, ui.raw(3.3, "Yab's blur at the band's edge")),
						Band(340, ui.raw(4.9, "Yab's blur nearer the row")),
						Band(240, ui.raw(6.7, "Yab's blur about the row")),
						ui.div {
							absolute = true,
							left = 0,
							right = 0,
							top = function(): number
								local _, h = window_size()
								return (h - BAND_H) / 2
							end,
							h = ui.raw(BAND_H, "Yab's veil about the row"),
							bg = "$zeron.switcher.veil",
							mask_gradient = {
								angle = 180,
								stops = {
									{ 0, "#00000000" },
									{ 0.32, "#000000ff" },
									{ 0.68, "#000000ff" },
									{ 1, "#00000000" },
								},
							},
						},
					},
					ui.div {
						absolute = true,
						left = 0,
						top = 0,
						right = 0,
						bottom = 0,
						opacity = shown,
						scale = function(): number
							return if motion.reduced() then 1 else 0.95 + 0.05 * arrive:get()
						end,
						pointer_events = "none",
						Row(),
					},
				}
			end,
		},
	}
end

return M
