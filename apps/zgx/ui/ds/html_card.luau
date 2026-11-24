-- Agent pages (`html_render`) as gpui's crates/ui/src/transcript/html_card.rs, and its "Open in" cards.

local Icon = require("@self/icon")
local IconButton = require("@self/icon_button").IconButton
local control = require("@self/control")
local locale = require("@self/locale")
local material = require("@self/material")
local glass = require("@self/glass")
local api = require("@self/html_api")
local picks = require("@self/html_picks")

local M = {}

export type Pages = {
	chosen: (card: string) -> { any },
	sent: (title: string, message: string) -> { list: { any }, locked: { [string]: boolean }, closed: boolean? },
	pick: (card: string, title: string, message: string, p: any) -> (),
	remote: () -> boolean,
	device_name: (() -> string?)?,
	root: () -> string?,
	reveal: (path: string) -> (),
	open_external: (path: string) -> (),
	open_in_zeron: (path: string) -> (),
	resolve: (target: string, root: string?) -> any,
	tray: ((card: string) -> { n: number, total: number, open: boolean }?)?,
	hold_tray: ((card: string) -> ())?,
	toggle_tray: ((card: string) -> ())?,
}

local MIN_H, MAX_H = 80, 2000
-- The shipping app's MAX_LIVE_PAGES.
local MAX_LIVE = 6

local function raw(n: number, why: string): any
	return ui.raw(n, why)
end

local function clamp(h: number): number
	if h ~= h then
		return MIN_H
	end
	return math.clamp(math.floor(h + 0.5), MIN_H, MAX_H)
end

local function hex(token: string): string
	local v = ui.theme.get(token)
	return if type(v) == "string" and string.sub(v, 1, 1) == "#" then v else "#808080"
end

local function alpha(h: string, a: number): string
	return string.sub(h, 1, 7) .. string.format("%02x", math.clamp(math.floor(a * 255 + 0.5), 0, 255))
end

local function number_of(token: string, fallback: number): number
	local v = ui.theme.get(token)
	return if type(v) == "number" then v else fallback
end

local function font_of(token: string): string
	local v = ui.theme.get(token)
	return if type(v) == "string" then v else ""
end

function M.theme(): { [string]: any }
	local kind = ui.theme.kind()
	local dark = kind == "dark" or kind == "high-contrast"
	local text = hex("text.primary")
	local accent = hex("accent")
	return {
		dark = dark,
		glass = material.frosted:peek() == true,
		bg = hex("background"),
		text = text,
		text_faint = hex("text.tertiary"),
		text_muted = hex("text.secondary"),
		element_hover = hex("hover"),
		hover = alpha(text, 0.08),
		selection = hex("selection"),
		-- gpui's dark card is a step over the page (17 on 13); surface.raised, a 6% white, reads 28.
		surface_card = if dark then alpha(text, 0.015) else hex("surface.raised"),
		surface_overlay = hex("surface.overlay"),
		surface_raised = hex("surface.raised"),
		border = hex("border"),
		border_strong = hex("border.strong"),
		input_bg = hex("surface.sunken"),
		accent = accent,
		solid = text,
		on_solid = hex("background"),
		on_accent = hex("accent.text"),
		accent_wash = alpha(accent, 0.16),
		accent_strong = hex("accent.hover"),
		danger = hex("danger"),
		danger_strong = hex("danger"),
		danger_muted = alpha(hex("danger"), 0.12),
		warning = hex("warning"),
		warning_muted = alpha(hex("warning"), 0.12),
		success = hex("success"),
		success_muted = alpha(hex("success"), 0.12),
		code_wash = hex("zeron.prose.code.bg"),
		code_text = text,
		font_sans = font_of("font.sans"),
		font_mono = font_of("font.mono"),
		code_font_size = number_of("zeron.prose.code.size", 12.5),
		text_size = number_of("zeron.prose.size", 14),
		line_height = number_of("zeron.prose.line", 22),
		pick = picks.ink(),
	}
end

local PAGES: { [string]: any } = {}
local ORDER: { string } = {}
-- (zgx places every page under all overlay paint: a card's page left under the open sheet puts the sheet's glass over the sheet's own page.)
local SHEET = ui.state("html_sheet_open", nil :: string?)
local SHEETS_MADE = 0

local function touch(key: string)
	local i = table.find(ORDER, key)
	if i then
		table.remove(ORDER, i)
	end
	table.insert(ORDER, key)
	local over = #ORDER - MAX_LIVE
	local at = 1
	while over > 0 and at <= #ORDER do
		local old = ORDER[at]
		local p = PAGES[old]
		if p and (p.mounted or 0) <= 0 then
			api.close(p.web)
			if p.sheet then
				api.close(p.sheet)
			end
			PAGES[old] = nil
			table.remove(ORDER, at)
			local shared = false
			for _, other in PAGES do
				shared = shared or (other.path == p.path and other.device == p.device)
			end
			if not shared and p.path then
				api.forget({ path = p.path, device = p.device })
			end
			over -= 1
		else
			at += 1
		end
	end
end

local leave = api.leave

-- (Made once the page is read: its bootstrap goes in with it.)
local function web_of(e: any, id: string): any
	local web = api.web(id, e.boot)
	api.keep_page(web, function(url: string)
		leave(e, url)
	end)
	return web
end

local function entry_of(key: string): any
	local e = PAGES[key]
	if not e then
		e = { web = nil, nonce = nil, loaded = false, html = nil, height = nil, capped = false, groups = {} }
		PAGES[key] = e
	end
	return e
end

-- One line each, so the agent's block reads back (zeron_proto `extract_picks`).
local function word(v: any, most: number): string?
	if type(v) ~= "string" then
		return nil
	end
	-- ASCII only: %c and %s follow the C locale, and under a UTF-8 one they match bytes inside letters.
	local s = string.gsub((string.gsub(v, "[%z\1-\31\127]", " ")), " +", " ")
	s = string.match(s, "^ ?(.-) ?$") or ""
	if #s > most then
		local cut = most
		while cut > 0 and (string.byte(s, cut + 1) or 0) >= 0x80 and (string.byte(s, cut + 1) or 0) < 0xC0 do
			cut -= 1
		end
		s = string.sub(s, 1, cut)
	end
	return s
end

local function code_of(v: any): string?
	local c = word(v, 32)
	return if c and c ~= "" and not string.find(c, " ", 1, true) then c else nil
end

local function groups_of(list: any, options: boolean): { any }
	local groups = {}
	for i, g in list do
		if type(i) ~= "number" or i > 12 or type(g) ~= "table" then
			break
		end
		local c = code_of(g.code)
		if c then
			local n = word(g.name, 60)
			local group: any = { code = c, name = if n and n ~= "" then n else c }
			if options then
				group.options = {}
				for j, o in if type(g.options) == "table" then g.options else {} do
					if type(j) ~= "number" or j > 24 or type(o) ~= "table" then
						break
					end
					local oc, label = code_of(o.code), word(o.label, 120)
					if oc then
						table.insert(group.options, { code = oc, label = if label and label ~= "" then label else oc, desc = word(o.desc, 160) or "" })
					end
				end
			end
			table.insert(groups, group)
		end
	end
	return groups
end

local function clean_pick(p: any): any?
	if type(p) ~= "table" then
		return nil
	end
	local code, group, label = word(p.code, 32), word(p.group, 32) or "", word(p.label, 120)
	if not code or code == "" or not label or string.find(code, "%s") or string.find(group, "%s") then
		return nil
	end
	if label == "" then
		label = code
	end
	return { code = code, group = group, label = label, groups = if type(p.groups) == "table" then groups_of(p.groups, false) else {} }
end

function M.count(groups: { any }, chosen: { any }, sent: { list: { any }, locked: { [string]: boolean } }): string
	local total = #groups
	if total == 0 then
		if #sent.list > 0 and #chosen == 0 then
			return locale.s("ds.html_card.count_sent")
		end
		return if #chosen > 0 then locale.s("ds.html_card.count_free", { n = #chosen }) else ""
	end
	local locked = 0
	for _, g in groups do
		if sent.locked[g.code] then
			locked += 1
		end
	end
	if locked >= total then
		return locale.s("ds.html_card.count_sent")
	end
	if locked > 0 then
		if #chosen > 0 then
			return locale.s("ds.html_card.count_partly_sent_chosen", { n = locked, total = total, chosen = #chosen })
		end
		return locale.s("ds.html_card.count_partly_sent", { n = locked, total = total })
	end
	if #chosen >= total then
		return locale.s("ds.html_card.count_all", { total = total })
	end
	return locale.s("ds.html_card.count_chosen", { n = #chosen, total = total })
end

local function picks_state(e: any, chosen: { any }, sent: any): any
	local list, done = {}, {}
	for _, it in chosen do
		table.insert(list, { group = it.group or "", code = it.code })
	end
	for _, s in sent.list do
		table.insert(done, { group = s.group or "", code = if (s.group or "") ~= "" then s.code else s.label })
	end
	local locked = {}
	for g in sent.locked do
		table.insert(locked, g)
	end
	table.sort(locked)
	local groups = e.groups
	if #groups == 0 and chosen[1] then
		groups = chosen[1].groups or {}
	end
	return { chosen = list, sent = done, locked = locked, count = M.count(groups, chosen, sent), closed = sent.closed == true }
end

local function header_button(id: string, icon: string, key: string, on_click: () -> ()): Node
	return IconButton {
		id = id,
		icon = icon :: any,
		label = locale.t(key),
		size = "sm",
		on_click = on_click,
	}
end

local function placeholder(words: any, h: any): Node
	return ui.column {
		w_full = true,
		h = h,
		items = "center",
		justify = "center",
		rounded = raw(10, "Zeron's page placeholder"),
		bg = ui.raw(alpha(hex("text.primary"), 0.035), "Zeron's ink(0.035)"),
		ui.text { words, text_size = "$font.size.sm", color = "$text.secondary", align = "center" },
	}
end

local function header(children: { Node }): Node
	return ui.row {
		w_full = true,
		h = raw(22, "Zeron's card header"),
		items = "center",
		gap = raw(6, "Zeron's card header"),
		children,
	}
end

local function closed_tag(key: string): Node
	return ui.row {
		id = `html-card-closed-{key}`,
		h = raw(18, "the picks' code chip"),
		px = raw(6, "the picks' code chip"),
		items = "center",
		shrink = 0,
		rounded = raw(5, "the picks' code chip"),
		bg = "$surface.raised",
		border = 0.5,
		border_color = "$border",
		ui.text { locale.t("ds.html_card.closed"), wrap = false, text_size = "$font.size.xs", color = "$text.tertiary" },
	}
end

local function tray_chip(key: string, pages: Pages?): Node
	local now = ui.computed(function(): any
		return if pages and pages.tray then pages.tray(key) else nil
	end)
	local function tray(): { n: number, total: number, open: boolean }?
		return now:get()
	end
	local function words(): string
		local t = tray()
		return if t then locale.s("ds.html_card.picked", { n = t.n, total = t.total }) else ""
	end
	local function open(): boolean
		local t = tray()
		return t ~= nil and t.open
	end
	return ui.show(function(): boolean
		return tray() ~= nil
	end, ui.button {
		id = `html-card-picks-{key}`,
		label = words,
		expanded = open,
		h = raw(22, "the mockup's picked chip"),
		px = raw(8, "the mockup's picked chip"),
		gap = raw(6, "the mockup's picked chip"),
		shrink = 0,
		items = "center",
		rounded = raw(11, "the mockup's picked chip"),
		border = 1,
		border_color = function(): string
			return if open() then "$border.strong" else "$border"
		end,
		bg = function(): string
			return if open() then "$hover" else "#00000000"
		end,
		hover = { bg = "$hover", border_color = "$border.strong" },
		focus_on_press = false,
		on_press = function()
			if pages and pages.hold_tray then
				pages.hold_tray(key)
			end
		end,
		on_click = function()
			if pages and pages.toggle_tray then
				pages.toggle_tray(key)
			end
		end,
		ui.div { w = 6, h = 6, shrink = 0, rounded = "$radius.full", bg = function(): any
			return raw(picks.ink() :: any, "the pick colour")
		end },
		ui.text { words, wrap = false, text_size = "$font.size.xs", color = "$text.primary" },
		ui.div {
			rotate = function(): number
				return if open() then 180 else 0
			end,
			Icon { name = "alt-arrow-down", size = 11, color = "$text.tertiary" },
		},
	})
end

local function title_text(words: any): Node
	return ui.text { words, literal = true, grow = 1, min_w = 0, truncate = true, text_size = "$font.size.sm", color = "$text.secondary" }
end

local function signed(value: any, main_frame: any, nonce: string?): boolean
	return main_frame == true and type(value) == "table" and value.kind == "zeron.page" and type(nonce) == "string" and nonce ~= "" and value.nonce == nonce
end

local function selected_text(v: any): string
	local t = if type(v) == "table" then v.text else nil
	return if type(t) == "string" and #t <= 16000 and string.find(t, "[^ \t\r\n]") then t else ""
end

local function line_in(el: any, l: any): any
	if type(l) ~= "table" then
		return nil
	end
	for i = 1, 4 do
		local v = l[i]
		if type(v) ~= "number" or v ~= v or math.abs(v) > 1e6 then
			return nil
		end
	end
	return { x = el.x + l[1], y = el.y + l[2], w = l[3], h = l[4] }
end

-- What a page says is selected in it, its lines in the window's own coordinates; nil for none.
local function selection(page_ref: any, v: any): any
	local el = page_ref:peek()
	local text = selected_text(v)
	if text == "" or not el or el.x == nil or el.y == nil then
		return nil
	end
	local top, bottom = line_in(el, v.top), line_in(el, v.bottom)
	return if top and bottom then { text = text, top = top, bottom = bottom } else nil
end

local function Sheet(props: any, open: State<boolean>, e: any, live: any): Node
	local view = ui.state(`html_sheet_view:{props.key}`, "live")
	local picked = ui.use_state("")
	local function heard(value: any, origin: any, main_frame: any)
		if signed(value, main_frame, e.picks) and value.select ~= nil then
			picked:set(selected_text(value.select))
		end
		if props.sheet_heard then
			props.sheet_heard(value, origin, main_frame)
		end
	end
	local function size(): any
		local s = (ui :: any).window.size:get()
		return s
	end
	-- (Its own token, not the key: one chat can show the same card twice.)
	SHEETS_MADE += 1
	local mine = `{props.key}#{SHEETS_MADE}`
	local function let_go()
		if SHEET:peek() == mine then
			SHEET:set(nil)
		end
	end
	ui.effect(function()
		if open:get() then
			SHEET:set(mine)
		else
			let_go()
		end
	end)
	ui.on_cleanup(let_go)
	return ui.modal(glass.style({
		id = "html-sheet",
		label = props.title,
		open = open,
		placement = { x = "center", y = "start", inset = { 46, 24, 96, 24 } },
		p = 0,
		gap = 0,
		w = function(): number
			local s = size()
			return math.max(320, (if s then s.width else 960) - 48)
		end,
		h = function(): number
			local s = size()
			return math.max(240, (if s then s.height else 700) - 46 - 96)
		end,
		rounded = raw(14, "Zeron's sheet"),
		build = function(): Node
			view:set("live")
			if not live and not e.sheet_loaded and e.html then
				e.sheet = e.sheet or web_of(e, `html:{props.key}#sheet`)
				api.load(e.sheet, e.html)
				e.sheet_loaded, e.sheet_theme, e.sheet_shown = true, e.baked, nil
			end
			return ui.column {
				w_full = true,
				h_full = true,
				ui.row {
					w_full = true,
					h = raw(42, "Zeron's sheet header"),
					px = raw(14, "Zeron's sheet header"),
					gap = "$space.2",
					items = "center",
					border_sides = { 0, 0, 1, 0 },
					border_color = "$border",
					ui.text { props.title, literal = true, grow = 1, min_w = 0, truncate = true, text_size = "$font.size.md", color = "$text.primary" },
					ui.show(function(): boolean
						return props.on_mention ~= nil and picked:get() ~= ""
					end, ui.button {
						id = "html-sheet-mention",
						label = locale.t("ds.annotation.mention"),
						h = 26,
						px = "$space.2",
						rounded = "$radius.md",
						bg = "$hover",
						color = "$text.primary",
						focus = control.ring,
						on_click = function()
							props.on_mention(picked:peek(), props.title)
							picked:set("")
							open:set(false)
						end,
						ui.text { locale.t("ds.annotation.mention"), text_size = "$font.size.sm" },
					}),
					if live
						then ui.button {
							id = "html-sheet-open",
							label = locale.t("ds.html_card.open_in_browser"),
							h = 26,
							px = "$space.2",
							rounded = "$radius.md",
							bg = "#00000000",
							color = "$text.secondary",
							hover = { bg = "$hover", color = "$text.primary" },
							focus = control.ring,
							on_click = function()
								api.open_url(props.url)
							end,
							ui.text { locale.t("ds.html_card.open_in_browser"), text_size = "$font.size.sm" },
						}
						else ui.row {
							gap = raw(2, "Zeron's segments"),
							ui.button {
								id = "html-sheet-live",
								label = locale.t("ds.html_card.live"),
								selected = function(): boolean
									return view:get() == "live"
								end,
								h = 24,
								px = raw(10, "Zeron's segment"),
								rounded = raw(7, "Zeron's segment"),
								bg = function(): any
									return if view:get() == "live" then "$hover" else "#00000000"
								end,
								color = function(): any
									return if view:get() == "live" then "$text.primary" else "$text.secondary"
								end,
								focus = control.ring,
								on_click = function()
									view:set("live")
								end,
								ui.text { locale.t("ds.html_card.live"), text_size = "$font.size.sm" },
							},
							ui.button {
								id = "html-sheet-source",
								label = locale.t("ds.html_card.source"),
								selected = function(): boolean
									return view:get() == "source"
								end,
								h = 24,
								px = raw(10, "Zeron's segment"),
								rounded = raw(7, "Zeron's segment"),
								bg = function(): any
									return if view:get() == "source" then "$hover" else "#00000000"
								end,
								color = function(): any
									return if view:get() == "source" then "$text.primary" else "$text.secondary"
								end,
								focus = control.ring,
								on_click = function()
									view:set("source")
								end,
								ui.text { locale.t("ds.html_card.source"), text_size = "$font.size.sm" },
							},
						},
					if live
						then nil
						else ui.button {
							id = "html-sheet-save",
							label = locale.t("ds.html_card.save"),
							h = 26,
							px = "$space.2",
							rounded = "$radius.md",
							bg = "#00000000",
							color = "$text.secondary",
							hover = { bg = "$hover", color = "$text.primary" },
							focus = control.ring,
							on_click = function()
								ui.task(function()
									local file = ui.save_file({ title = locale.s("ds.html_card.save"), name = api.file_name(props.title) }):await()
									if file then
										api.save({ path = props.path, device = props.device, to = file.path }, function() end)
									end
								end)
							end,
							ui.text { locale.t("ds.html_card.save"), text_size = "$font.size.sm" },
						},
					IconButton {
						id = "html-sheet-close",
						icon = "close",
						label = locale.t("ds.html_card.close"),
						size = "sm",
						tone = "quiet",
						on_click = function()
							open:set(false)
						end,
					},
				},
				ui.presence(function(): string
					return if live then "live" else view:get()
				end, function(which: string): Node
					if which == "source" then
						return ui.scroll {
							id = "html-sheet-source-text",
							w_full = true,
							grow = 1,
							min_h = 0,
							p = "$space.4",
							ui.text {
								api.source({ path = props.path, device = props.device }) or "",
								literal = true,
								font = "$font.mono",
								text_size = "$font.size.sm",
								color = "$text.primary",
							},
						}
					end
					return ui.column {
						w_full = true,
						grow = 1,
						min_h = 0,
						pt = "$space.3",
						px = "$space.4",
						api.view {
							id = "html-sheet-page",
							label = props.title,
							web = if live then nil else e.sheet,
							src = if live then props.url else nil,
							transparent = not live,
							isolated = if live and e.boot then { e.boot } else nil,
							w_full = true,
							grow = 1,
							min_h = 0,
							on_message = heard,
							on_navigate = props.sheet_navigated,
							on_new_window = props.open_out or api.open_url,
						},
					}
				end, { w_full = true, grow = 1, min_h = 0 }),
			}
		end,
	}, "menu"))
end

local function theme_now(): string
	return `{tostring(ui.theme.current:peek())}|{tostring(material.frosted:peek())}`
end

-- The page's theme swapped in place when it was shown under another; returns the one it has now.
local function retheme(web: any, has: string?): string
	local now = theme_now()
	if has ~= now then
		local js = api.retheme({ theme = M.theme(), font_size = 16 })
		if js then
			api.run(web, js)
		end
	end
	return now
end

local function Page(props: any): Node
	local key = props.key
	local e = entry_of(key)
	e.path, e.device = props.path, props.device
	e.mounted = (e.mounted or 0) + 1
	touch(key)
	ui.on_cleanup(function()
		e.mounted = (e.mounted or 1) - 1
	end)
	local status = ui.use_state(if e.loaded then "ready" elseif e.gone then "failed" else "loading")
	local content = ui.use_state(e.height or -1)
	local sheet = ui.use_state(false)
	if not e.loaded and not e.reading and not e.gone then
		e.reading = true
		local baked = theme_now()
		api.page({ path = props.path, device = props.device, theme = M.theme(), font_size = 16, source = props.source }, function(got: any, _why: string?)
			e.reading = false
			if type(got) == "table" and type(got.html) == "string" then
				e.nonce, e.picks, e.html, e.loaded, e.baked, e.theme = got.nonce, got.picks, got.html, true, baked, baked
				e.boot = got.boot
				e.web = web_of(e, `html:{key}`)
				api.load(e.web, got.html)
				status:set("ready")
			else
				status:set("failed")
			end
		end)
	end
	local frame = ui.computed(function(): number
		local c = content:get()
		local f = api.frame(props.height, if c >= 0 then c else nil)
		if f.overflows then
			e.capped = true
		end
		return if e.capped then f.agent else f.height
	end)
	local pages: Pages? = props.pages
	local function chosen(): { any }
		return if pages then pages.chosen(key) else {}
	end
	local function sent(): any
		return if pages then pages.sent(props.title, props.message) else { list = {}, locked = {} }
	end
	-- (The host call only when the state changed: this reruns with every streamed token.)
	local function sync_picks()
		local state = picks_state(e, chosen(), sent())
		local sig = ui.json.encode(state)
		if sig ~= e.sig then
			e.sig, e.js = sig, api.picks(state)
		end
		local js = e.js
		if not js then
			return
		end
		if e.loaded and js ~= e.shown then
			e.shown = js
			api.run(e.web, js)
		end
		if e.sheet_loaded and js ~= e.sheet_shown then
			e.sheet_shown = js
			api.run(e.sheet, js)
		end
	end
	ui.effect(function()
		local _ = status:get()
		sync_picks()
	end)
	ui.effect(function()
		ui.theme.current:get()
		material.frosted:get()
		if e.loaded then
			e.theme = retheme(e.web, e.theme)
		end
		if e.sheet_loaded then
			e.sheet_theme = retheme(e.sheet, e.sheet_theme)
		end
	end)
	local function take_pick(value: any)
		if value.pick ~= nil and pages then
			local p = clean_pick(value.pick)
			if p then
				if #p.groups > 0 then
					e.groups = p.groups
				end
				pages.pick(key, props.title, props.message, p)
			end
		end
	end
	local page_ref = ui.ref()
	local function take_select(value: any)
		if props.on_select and type(value.select) == "table" then
			props.on_select(selection(page_ref, value.select))
		end
	end
	local function heard(value: any, _origin: any, main_frame: any)
		if signed(value, main_frame, e.picks) then
			take_pick(value)
			take_select(value)
			return
		end
		if not signed(value, main_frame, e.nonce) then
			return
		end
		if type(value.edges) == "number" then
			api.chain(e.web, value.edges)
		end
		if type(value.height) == "number" and value.height == value.height and value.height >= 0 then
			e.height = value.height
			if math.abs(content:peek() - value.height) >= 1 then
				content:set(value.height)
			end
		end
		if type(value.ready) == "table" then
			e.groups = groups_of(value.ready, true)
			if pages and pages.ready then
				pages.ready(key, props.title, props.message, e.groups)
			end
			e.shown = nil
			e.theme = retheme(e.web, e.theme)
			sync_picks()
		end
	end
	-- (The sheet's copy is wider: its height is not the card's.)
	local function sheet_heard(value: any, _origin: any, main_frame: any)
		if signed(value, main_frame, e.picks) then
			take_pick(value)
		elseif signed(value, main_frame, e.nonce) and type(value.ready) == "table" then
			e.sheet_shown = nil
			e.sheet_theme = retheme(e.sheet, e.sheet_theme)
			sync_picks()
		end
	end
	local function open_out(url: string)
		leave(e, url)
	end
	-- (A backstop where `api.keep_page` couldn't refuse it first: a page leaving its document is put back.)
	local function navigated(on_sheet: boolean): (any) -> ()
		return function(nav: any)
			if type(nav) ~= "table" or (nav.kind ~= "started" and nav.kind ~= "committed") or not e.html then
				return
			end
			local url = if type(nav.url) == "string" then nav.url else ""
			if url == "" or url == "about:blank" or string.sub(url, 1, 12) == "about:blank#" then
				return
			end
			local web = if on_sheet then e.sheet else e.web
			local t = ui.now()
			-- (A link's start and commit come as two events: one leaving.)
			if url == e.left and t - (e.left_at or -1e9) < 2000 then
				return
			end
			e.left, e.left_at = url, t
			local recent = {}
			for _, at in e.bounces or {} do
				if t - at < 10000 then
					table.insert(recent, at)
				end
			end
			table.insert(recent, t)
			e.bounces = recent
			if #recent > 6 then
				api.load(web, "")
				if not on_sheet then
					e.loaded, e.gone = false, true
					status:set("failed")
				end
				return
			end
			api.load(web, e.html)
			if on_sheet then
				e.sheet_theme, e.sheet_shown = e.baked, nil
			else
				e.theme, e.shown = e.baked, nil
			end
			open_out(url)
		end
	end
	local sheet_props = table.clone(props)
	sheet_props.sheet_heard = sheet_heard
	sheet_props.sheet_navigated = navigated(true)
	sheet_props.open_out = open_out
	return ui.column {
		id = `html-card-{key}`,
		class = "html-card",
		w_full = true,
		gap = raw(6, "Zeron's card"),
		header {
			Icon { name = "global", size = 12, color = "$text.tertiary" },
			title_text(props.title),
			ui.show(function(): boolean
				return sent().closed == true
			end, closed_tag(key)),
			tray_chip(key, pages),
			header_button(`html-card-expand-{key}`, "expand-arrows", "ds.html_card.expand", function()
				if e.loaded then
					sheet:set(true)
				end
			end),
		},
		ui.presence(function(): string
			local now = status:get()
			return if now == "ready" and SHEET:get() ~= nil then "under_sheet" else now
		end, function(now: string): Node
			if now == "failed" then
				return placeholder(locale.t("ds.html_card.unavailable"), frame)
			end
			if now == "loading" or now == "under_sheet" then
				return ui.column { w_full = true, h = frame }
			end
			return ui.column {
				ref = page_ref,
				w_full = true,
				api.view {
					id = `html-page-{key}`,
					label = props.title,
					web = e.web,
					w_full = true,
					h = frame,
					on_message = heard,
					on_navigate = navigated(false),
					on_new_window = open_out,
				},
			}
		end, { w_full = true }),
		Sheet(sheet_props, sheet, e, false),
	}
end

local function Live(props: any): Node
	local live = api.live(props.url)
	if not live then
		return ui.column {}
	end
	local height = clamp(props.height)
	local pages: Pages? = props.pages
	if pages and pages.remote() then
		local where = if pages.device_name then pages.device_name() else nil
		return ui.column {
			id = `html-card-{props.key}`,
			class = "html-card",
			w_full = true,
			gap = raw(6, "Zeron's card"),
			header { title_text(live.label) },
			placeholder(if where then locale.t("ds.html_card.running_on", { device = where }) else locale.t("ds.html_card.running_elsewhere"), math.min(height, 120)),
		}
	end
	local reach = ui.use_state("checking")
	local nonce = ui.use_state(0)
	local sheet = ui.use_state(false)
	local page_ref = ui.ref()
	local function probe()
		api.reach(props.url, function(up: boolean)
			reach:set(if up then "up" else "down")
		end)
	end
	probe()
	local function reload()
		reach:set("checking")
		nonce:set(nonce:peek() + 1)
		probe()
	end
	return ui.column {
		id = `html-card-{props.key}`,
		class = "html-card",
		w_full = true,
		gap = raw(6, "Zeron's card"),
		header {
			Icon { name = "global", size = 12, color = "$text.tertiary" },
			title_text(live.label),
			header_button(`live-page-reload-{props.key}`, "refresh", "ds.html_card.reload", reload),
			header_button(`live-page-open-{props.key}`, "arrow-up-right", "ds.html_card.open_in_browser", function()
				api.open_url(props.url)
			end),
			header_button(`html-card-expand-{props.key}`, "expand-arrows", "ds.html_card.expand", function()
				if reach:peek() == "up" then
					sheet:set(true)
				end
			end),
		},
		-- (Keyed on the sheet too, so the sheet's live view is the only one loading the server.)
		ui.presence(function(): string
			return `{reach:get()}:{nonce:get()}:{if SHEET:get() ~= nil then "sheet" else ""}`
		end, function(now: string): Node
			if string.sub(now, 1, 5) == "down:" then
				return ui.column {
					w_full = true,
					h = height,
					gap = "$space.2",
					items = "center",
					justify = "center",
					rounded = raw(10, "Zeron's page placeholder"),
					bg = ui.raw(alpha(hex("text.primary"), 0.035), "Zeron's ink(0.035)"),
					ui.text { locale.t("ds.html_card.not_responding", { server = live.server }), text_size = "$font.size.sm", color = "$text.secondary" },
					ui.button {
						id = `live-page-retry-{props.key}`,
						label = locale.t("ds.html_card.retry"),
						h = 26,
						px = "$space.3",
						rounded = "$radius.md",
						bg = "#00000000",
						color = "$text.secondary",
						hover = { bg = "$hover", color = "$text.primary" },
						focus = control.ring,
						on_click = reload,
						ui.text { locale.t("ds.html_card.retry"), text_size = "$font.size.sm" },
					},
				}
			end
			if string.sub(now, 1, 3) ~= "up:" or SHEET:peek() ~= nil then
				return ui.column { w_full = true, h = height }
			end
			return ui.column {
				ref = page_ref,
				w_full = true,
				api.view {
					id = `html-page-{props.key}`,
					label = live.label,
					src = props.url,
					isolated = if live.boot then { live.boot } else nil,
					w_full = true,
					h = height,
					on_message = function(value: any, _origin: any, main_frame: any)
						if props.on_select and signed(value, main_frame, live.picks) and value.select ~= nil then
							props.on_select(selection(page_ref, value.select))
						end
					end,
					on_error = function()
						probe()
					end,
					on_new_window = function(url: string)
						api.open_url(url)
					end,
				},
			}
		end, { w_full = true }),
		Sheet(props, sheet, { picks = live.picks, boot = live.boot }, true),
	}
end

M.Card = ui.component(function(props: any): Node
	if props.url and props.url ~= "" then
		return Live(props)
	end
	return Page(props)
end)

M.open_app = ui.persist("html_open_app", "default")

local function open_card(props: any, ix: number, path: string, absolute: string): Node
	local pages: Pages = props.pages
	local remote = pages.remote()
	local state = ui.use_state(if remote then { missing = false, remote = true } else nil :: any)
	local menu = ui.use_state(false)
	if not remote then
		api.title(absolute, function(got: any)
			state:set(got)
		end)
	end
	local function disabled(): boolean
		local s = state:get()
		return remote or (s ~= nil and s.missing == true)
	end
	local function title(): string
		local s = state:get()
		if s and type(s.title) == "string" and s.title ~= "" then
			return s.title
		end
		return string.match(absolute, "([^/]+)$") or path
	end
	local function detail(): string
		if remote then
			return locale.s("ds.html_card.website_elsewhere")
		end
		local s = state:get()
		if s and s.missing then
			return locale.s("ds.html_card.website_gone")
		end
		return locale.s("ds.html_card.website", { path = path })
	end
	local function open(app: string)
		menu:set(false)
		if disabled() then
			return
		end
		if app == "zeron" then
			pages.open_in_zeron(path)
		else
			pages.open_external(path)
		end
	end
	local function choose(app: string)
		M.open_app:set(app)
		open(app)
	end
	local function app_icon(): Node
		return ui.presence(function(): string
			return M.open_app:get()
		end, function(app: string): Node
			return Icon { name = (if app == "zeron" then "zeron-logo" else "global") :: any, size = 16 }
		end)
	end
	local function row(id: string, label: any, icon: string, checked: () -> boolean, on_click: () -> ()): Node
		return ui.menu_item {
			id = id,
			label = label,
			on_click = on_click,
			ui.row {
				w_full = true,
				gap = "$space.2",
				items = "center",
				ui.row { w = 18, h = 18, items = "center", justify = "center", Icon { name = icon :: any, size = 16 } },
				ui.text { label, grow = 1, min_w = 0, truncate = true },
				ui.show(checked, Icon { name = "check", size = 14, color = "$text.secondary" }),
			},
		}
	end
	local chevron = ui.button {
		id = `html-card-chevron-{props.key}-{ix}`,
		label = locale.t("ds.html_card.choose_app"),
		disabled = disabled,
		h = 30,
		px = raw(8, "Zeron's chevron"),
		items = "center",
		bg = function(): any
			return if menu:get() then "$hover" else "#00000000"
		end,
		color = "$text.secondary",
		border_sides = { 0, 0, 0, 1 },
		border_color = "$border",
		hover = { bg = "$hover" },
		focus = control.ring,
		on_click = function()
			menu:set(not menu:peek())
		end,
		Icon { name = "alt-arrow-down", size = 14 },
	}
	return ui.row {
		id = `html-card-{props.key}-{ix}`,
		class = "html-open-card",
		w_full = true,
		items = "center",
		gap = "$space.3",
		p = "$space.3",
		rounded = raw(14, "Zeron's open card"),
		bg = ui.raw(alpha(hex("text.primary"), 0.035), "Zeron's ink(0.035)"),
		border = 1,
		border_color = "$border",
		ui.row {
			w = 40,
			h = 40,
			shrink = 0,
			items = "center",
			justify = "center",
			rounded = raw(10, "Zeron's open card"),
			bg = ui.raw(alpha(hex("text.primary"), 0.06), "Zeron's ink(0.06)"),
			opacity = function(): number
				return if disabled() then 0.5 else 1
			end,
			Icon { name = "global", size = 22, color = "$accent" },
		},
		ui.column {
			grow = 1,
			min_w = 0,
			gap = raw(2, "Zeron's open card"),
			ui.text { title, literal = true, truncate = true, text_size = "$font.size.md", color = "$text.primary" },
			ui.text { detail, literal = true, truncate = true, text_size = "$font.size.sm", color = "$text.secondary" },
		},
		ui.row {
			shrink = 0,
			rounded = raw(9, "Zeron's split button"),
			border = 1,
			border_color = "$border",
			overflow = "hidden",
			opacity = function(): number
				return if disabled() then 0.45 else 1
			end,
			ui.button {
				id = `html-card-open-{props.key}-{ix}`,
				label = locale.t("ds.html_card.open_in"),
				disabled = disabled,
				h = 30,
				pl = raw(9, "Zeron's open button"),
				pr = raw(10, "Zeron's open button"),
				gap = raw(6, "Zeron's open button"),
				items = "center",
				bg = "#00000000",
				color = "$text.primary",
				hover = { bg = "$hover" },
				focus = control.ring,
				on_click = function()
					open(M.open_app:peek())
				end,
				ui.show(function(): boolean
					return not disabled()
				end, app_icon()),
				ui.text { locale.t("ds.html_card.open_in"), text_size = "$font.size.md" },
			},
			ui.menu {
				id = `html-card-apps-{props.key}-{ix}`,
				open = menu,
				anchor = chevron,
				placement = { side = "bottom", align = "end", offset = 4 },
				w = 240,
				row(`html-card-open-zeron-{ix}`, locale.t("ds.html_card.zeron"), "zeron-logo", function(): boolean
					return M.open_app:get() == "zeron"
				end, function()
					choose("zeron")
				end),
				row(`html-card-open-default-{ix}`, locale.t("ds.html_card.default_browser"), "global", function(): boolean
					return M.open_app:get() ~= "zeron"
				end, function()
					choose("default")
				end),
				ui.separator {},
				row(`html-card-reveal-{ix}`, locale.t("ds.html_card.reveal"), "folder", function(): boolean
					return false
				end, function()
					pages.reveal(path)
				end),
				row(`html-card-copy-{ix}`, locale.t("ds.html_card.copy_path"), "copy", function(): boolean
					return false
				end, function()
					ui.clipboard.write(absolute)
				end),
			},
		},
	}
end

local OpenCard = ui.component(function(props: any): Node
	return open_card(props.card, props.ix, props.path, props.absolute)
end)

-- (Code spans count toward the three only once found on disk, as gpui filters before it truncates.)
M.OpenCards = ui.component(function(props: any): Node
	local pages: Pages? = props.pages
	if not pages then
		return ui.column {}
	end
	local root = pages.root()
	if not root or root == "" then
		return ui.column {}
	end
	local remote = pages.remote()
	local cards = {}
	local seen: { [string]: boolean } = {}
	for _, t in props.targets or {} do
		if #cards >= 12 then
			break
		end
		local r = pages.resolve(t.dest, root)
		if r and not r.outside and not seen[r.path] and (string.match(string.lower(r.path), "%.html?$") ~= nil) and (t.linked or not remote) then
			seen[r.path] = true
			table.insert(cards, { path = r.path, absolute = `{string.gsub(root, "/+$", "")}/{r.path}`, linked = t.linked })
		end
	end
	if #cards == 0 then
		return ui.column {}
	end
	for _, c in cards do
		if not c.linked then
			local found = ui.state(`html_found:{c.absolute}`, nil :: boolean?)
			if found:peek() == nil then
				api.title(c.absolute, function(got: any)
					found:set(not (got and got.missing))
				end)
			end
			c.found = found
		end
	end
	local function shown(ix: number): boolean
		local n = 0
		for i, c in cards do
			local here = c.linked or c.found:get() == true
			if here then
				n += 1
			end
			if i == ix then
				return here and n <= 3
			end
		end
		return false
	end
	local nodes = {}
	for ix, c in cards do
		table.insert(nodes, ui.show(function(): boolean
			return shown(ix)
		end, OpenCard { card = props, ix = ix, path = c.path, absolute = c.absolute }))
	end
	return ui.column { id = `html-open-{props.key}`, w_full = true, gap = "$space.2", nodes }
end)

return M
