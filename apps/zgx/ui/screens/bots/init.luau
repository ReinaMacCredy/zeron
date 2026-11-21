local ds = require("@ds")
local locale = require("@ds/locale")
local motion = require("@ds/motion")
local avatar = require("@app/screens/bots/avatar")
local create = require("@app/screens/bots/create")
local store = require("@app/screens/bots/store")

local M = {}

local PAGE_W = 760
local CARD_H, CARD_AVATAR, COLUMNS = 140, 58, 3
local GAP = ui.raw(14, "the mockup's gallery grid")
local HOVER_MS = 110


local function Tab(props: { id: string, word: string, count: () -> number, on: () -> boolean, pick: () -> () }): Node
	return ui.button {
		id = props.id,
		role = "tab",
		selected = props.on,
		h = ui.raw(28, "the mockup's gallery tab"),
		px = ui.raw(10, "the mockup's gallery tab"),
		gap = ui.raw(5, "the mockup's gallery tab"),
		items = "center",
		rounded = ui.raw(9, "the mockup's gallery tab"),
		cursor = "pointer",
		text_size = ui.raw(13.5, "the mockup's gallery tab"),
		bg = function(): string
			return if props.on() then "$zeron.secondary.bg" else "#00000000"
		end,
		color = function(): string
			return if props.on() then "$text.primary" else "$text.secondary"
		end,
		hover = { color = "$text.primary" },
		on_click = props.pick,
		ui.text { ui.t(props.word) },
		ui.text {
			function(): string
				return tostring(props.count())
			end,
			color = "$text.tertiary",
		},
	}
end

-- A card of the grid: its box, and the fill that eases in under the pointer.
local Card = ui.component(function(props: { id: string, label: any, on_click: () -> (), build: () -> { any } }): Node
	local over = ui.use_state(false)
	local out: { [any]: any } = {
		id = props.id,
		class = "bots-card",
		label = props.label,
		relative = true,
		h = CARD_H,
		basis = 0,
		grow = 1,
		min_w = 0,
		direction = "column",
		items = "center",
		justify = "center",
		gap = ui.raw(14, "the mockup's gallery card"),
		px = "$space.3",
		rounded = ui.raw(12, "the mockup's gallery card"),
		border = 1,
		border_color = "$border.strong",
		bg = "#00000000",
		hover = { bg = "#00000000" },
		active = { bg = "$pressed" },
		cursor = "pointer",
		on_hover = function(inside: boolean)
			over:set(inside)
		end,
		on_click = props.on_click,
		ui.div {
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			bottom = 0,
			rounded = ui.raw(11, "inside the card's border"),
			bg = "$hover",
			opacity = ui.tween(
				ui.computed(function(): number
					return if over:get() then 1 else 0
				end),
				{ duration = math.max(motion.ms(HOVER_MS), 1), ease = "outQuad" }
			),
			pointer_events = "none",
		},
	}
	for _, child in props.build() do
		table.insert(out, child)
	end
	return ui.button(out)
end)

local function BotCard(bot: store.Bot, archived: boolean): Node
	return Card {
		id = `bots-card-{bot.id}`,
		label = bot.name,
		on_click = function()
			store.go(bot)
		end,
		build = function(): { any }
			return {
				avatar.Avatar { look = bot.avatar, size = CARD_AVATAR },
				ui.text { bot.name, literal = true, truncate = true, max_w = "100%", text_size = "$font.size.md", weight = "$font.weight.medium", color = "$text.primary" },
				if archived then ui.text { ui.t("bots.gallery.archived_tag"), mt = ui.raw(-10, "the mockup's gallery card"), text_size = "$font.size.xs", color = "$text.tertiary" } else nil,
			}
		end,
	}
end

local function CreateCard(): Node
	return Card {
		id = "bots-card-create",
		label = ui.t("bots.gallery.create"),
		on_click = function()
			create.open("page")
		end,
		build = function(): { any }
			return {
				ui.div {
					w = ui.raw(56, "the mockup's Create Bot card"),
					h = ui.raw(56, "the mockup's Create Bot card"),
					rounded = ui.raw(12, "the mockup's Create Bot card"),
					bg = "$zeron.secondary.bg",
					items = "center",
					justify = "center",
					color = "$text.secondary",
					ds.Icon { name = "plus", size = 20 },
				},
				ui.text { ui.t("bots.gallery.create"), text_size = "$font.size.md", color = "$text.secondary" },
			}
		end,
	}
end

M.Page = ui.component(function(): Node
	local tab = ui.use_state("you")
	local query = ui.use_state("")
	local function matches(list: { store.Bot }): { store.Bot }
		local q = string.lower(query:get())
		local out = {}
		for _, b in list do
			if q == "" or string.find(string.lower(b.name), q, 1, true) then
				table.insert(out, b)
			end
		end
		return out
	end
	-- Rows of three; the last From you row ends in the Create Bot card.
	local rows = ui.computed(function(): { { any } }
		local archived = tab:get() == "archived"
		store.list:get()
		local cells: { any } = {}
		for _, b in matches(if archived then store.archived() else store.active()) do
			table.insert(cells, b)
		end
		if not archived then
			table.insert(cells, "create")
		end
		local out: { { any } } = {}
		for i = 1, #cells, COLUMNS do
			local row = { key = "", cells = {} :: { any } }
			for j = i, math.min(i + COLUMNS - 1, #cells) do
				local c = cells[j]
				table.insert(row.cells, c)
				row.key ..= (if type(c) == "string" then c else `{c.id}:{c.name}:{c.avatar.glyph}:{c.avatar.frame}:{c.avatar.color}:{c.avatar.image ~= nil}`) .. "|"
			end
			table.insert(out, row :: any)
		end
		return out
	end)
	local function none(): boolean
		local archived = tab:get() == "archived"
		store.list:get()
		return #matches(if archived then store.archived() else store.active()) == 0 and (archived or query:get() ~= "")
	end
	return ui.scroll {
		id = "bots-page",
		w_full = true,
		h_full = true,
		items = "center",
		create.Dialog { from = "page" },
		ui.column {
			w_full = true,
			max_w = PAGE_W,
			px = "$space.5",
			pt = ui.raw(70, "the mockup's gallery inset"),
			pb = ui.raw(40, "the mockup's gallery inset"),
			ui.row {
				w_full = true,
				items = "center",
				gap = "$space.2",
				ui.column {
					grow = 1,
					min_w = 0,
					gap = ui.raw(6, "the mockup's gallery head"),
					ui.text { locale.s("bots.title"), role = "heading", text_size = ui.raw(18, "the mockup's gallery head"), weight = "$font.weight.medium", color = "$text.primary" },
					ui.text { id = "bots-blurb", locale.s("bots.gallery.blurb"), text_size = ui.raw(13.5, "the mockup's gallery head"), color = "$text.secondary", wrap = true },
				},
				ds.Button {
					ds.Icon { name = "plus", size = 15 },
					ui.text { ui.t("bots.gallery.create") },
					id = "bots-create",
					tone = "primary",
					size = "sm",
					on_click = function()
						create.open("page")
					end,
				},
			},
			ui.row {
				role = "tablist",
				label = ui.t("bots.title"),
				gap = "$space.1",
				mt = ui.raw(20, "the mockup's gallery tabs"),
				mb = ui.raw(14, "the mockup's gallery tabs"),
				Tab {
					id = "bots-tab-you",
					word = "bots.gallery.from_you",
					count = function(): number
						store.list:get()
						return #store.active()
					end,
					on = function(): boolean
						return tab:get() == "you"
					end,
					pick = function()
						tab:set("you")
					end,
				},
				Tab {
					id = "bots-tab-archived",
					word = "bots.gallery.archived",
					count = function(): number
						store.list:get()
						return #store.archived()
					end,
					on = function(): boolean
						return tab:get() == "archived"
					end,
					pick = function()
						tab:set("archived")
					end,
				},
			},
			ui.row {
				w_full = true,
				h = ui.raw(34, "the mockup's gallery search"),
				px = "$space.3",
				gap = "$space.2",
				items = "center",
				rounded = ui.raw(10, "the mockup's gallery search"),
				border = 1,
				border_color = "$border.strong",
				color = "$text.secondary",
				ds.Icon { name = "search", size = 15 },
				ui.input {
					id = "bots-search",
					grow = 1,
					min_w = 0,
					h = 20,
					px = 0,
					border = 0,
					bg = "#00000000",
					focus = { border = 0, outline = 0 },
					label = ui.t("bots.gallery.search"),
					placeholder = ui.t("bots.gallery.search"),
					placeholder_color = "$text.tertiary",
					text_size = ui.raw(13.5, "the mockup's gallery search"),
					color = "$text.primary",
					value = function(): string
						return query:get()
					end,
					on_change = function(v: string)
						query:set(v)
					end,
				},
			},
			ui.column {
				id = "bots-grid",
				w_full = true,
				gap = GAP,
				mt = ui.raw(28, "the mockup's gallery grid"),
				ui.each(rows, function(row: any): Node
					local out: { [any]: any } = { w_full = true, gap = GAP }
					for _, c in row.cells do
						table.insert(out, if type(c) == "string" then CreateCard() else BotCard(c, c.archived == true))
					end
					for _ = #row.cells + 1, COLUMNS do
						table.insert(out, ui.div { basis = 0, grow = 1, min_w = 0 })
					end
					return ui.row(out)
				end, {
					key = function(row: any): string
						return row.key
					end,
				}),
				ui.show(none, ui.text {
					id = "bots-none",
					function(): string
						return locale.s(if tab:get() == "archived" and query:get() == "" then "bots.gallery.none_archived" else "bots.gallery.no_match")
					end,
					w_full = true,
					align = "center",
					py = ui.raw(60, "the mockup's gallery empty line"),
					text_size = "$font.size.md",
					color = "$text.secondary",
				}),
			},
		},
	}
end)

return M
