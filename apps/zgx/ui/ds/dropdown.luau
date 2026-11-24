-- A dropdown: a compact trigger (an icon, the chosen item, a chevron) that
-- opens a frosted panel: a search field, the items it filters (each with a
-- letter tile, the chosen one in the selected state), then actions below a
-- rule. Items
-- may carry an icon and a line saying what they do (a mode's), the search
-- may be left out (`search = false`, for a few choices), and `head` and
-- `extra` put something of the caller's at the panel's top and foot, a
-- rule between (a setting beside the choice: where to work, a switch).
--
-- `Dropdown { value = state, items = { { value = "zgx", label = "zgx" }, … },
-- actions = { { label = "New project", icon = "plus", on_click = … } },
-- icon = "folder", placeholder = "No project", search = "Search projects…" }`

local Icon = require("@self/icon")
local motion = require("@self/motion")
local Mark = require("@self/mark")
local AgentMark = require("@self/agent_mark")
local RollText = require("@self/roll_text")
local FadeText = require("@self/fade_text")
local PrivateText = require("@self/private_text")
local Avatar = require("@self/avatar")
local Text = require("@self/text")
local Tile = require("@self/tile")
local ascii = require("@self/ascii")
local Menu = require("@self/menu")
local Skeleton = require("@self/skeleton")
local control = require("@self/control")
local locale = require("@self/locale")
local EditorMenu = require("@self/editor_menu")

-- An item: its name, and the harness mark or icon it shows in place of its
-- letter tile (a model's, a mode's), what it does (under its name), a muted
-- tag at its end ("You"), and a warning that it is out of reach (a device
-- offline).
export type Item = {
	value: string,
	label: string,
	mark: Mark.Name?,
	-- An agent from the ACP registry's SVG, in a mark's place, in the
	-- agent's colours (ds.AgentMark: `svg_seed`, the agent's id, its hue).
	svg: string?,
	svg_seed: string?,
	icon: Icon.Name?,
	-- Its tile on a palette hue (a project's: its initial in the hue's 50 on
	-- its 500), in place of the plain letter tile.
	hue: string?,
	-- A project's id, where the item is one: its tile is then
	-- `.project-icon` (ds/tile).
	project: string?,
	-- Its own artwork (a project's favicon), in place of its hue's tile.
	src: string?,
	-- A picture chosen for it (a project's emoji), on its hue's wash.
	emoji: string?,
	description: string?,
	-- Quiet words beside its name, in the name's size and on its line (a
	-- model's provider, telling it from a namesake), searched as its name
	-- is. The name gives way first: it fades, the hint stays whole. With
	-- `hint_when_found`, they show only while a search found the row by
	-- them (its name alone doesn't hold what was typed). A row whose hint
	-- changes is built again (its key holds them).
	hint: string?,
	hint_when_found: boolean?,
	-- Its label is an email address: drawn as ds.PrivateText draws one, its
	-- tile no letter of it (`seed`'s avatar).
	private: boolean?,
	tag: string?,
	-- A heading over it, where a section of the list starts.
	heading: string?,
	-- Its glyph's and name's ink, when not the menu's (a warning).
	ink: string?,
	warn: boolean?,
	-- Shown but not to be picked (a device out of reach): dimmed, passed
	-- over by the keys, its `description` saying why.
	disabled: boolean?,
	-- Shown and highlighted, but a press does nothing (a model off the
	-- catalog that the chat still runs).
	inert: boolean?,
	-- What it says under the pointer, a moment on.
	tip: string?,
}
-- What a caller's own trigger is given: whether the panel is open, and how
-- to open or close it. A submenu trigger forwards its hover, pointer-move,
-- mount and press events so the hover corridor can keep it open across gaps
-- and sibling rows.
export type Menu = {
	open: State<boolean>,
	toggle: () -> (),
	on_mount: ((el: Element) -> ())?,
	on_hover: ((inside: boolean) -> ())?,
	on_pointer_move: ((ev: PointerEvent) -> ())?,
	on_click: (() -> ())?,
}
-- (The panel's own words, an action's, an empty list's, the placeholder,
-- the search's and the detail, may be messages, `ui.t`. An item's label
-- stays a string: the search reads it, and its tile its first letter.)
-- `shown`: whether it's offered now (followed as it changes); none, always.
export type Action = { label: string | Message | () -> string, icon: Icon.Name, on_click: (() -> ())?, shown: (() -> boolean)? }
-- What an empty list shows: what is missing, and how to fill it (a button doing
-- it, `action`). `skeleton`: its rows are on their way, their shapes held.
export type Empty = {
	title: string | Message,
	hint: (string | Message)?,
	action: { label: string, on_click: () -> () }?,
	skeleton: boolean?,
}
-- The trigger's weight: `filled` (a control on its own, with a chevron),
-- `ghost` (a chip among a bar's controls: no fill until touched), `chip` (a
-- footer's small quiet label). Only `filled` shows a chevron.
export type Tone = "filled" | "ghost" | "chip" | "model_chip" | "model" | "access"
export type Props = {
	value: State<string?>,
	-- The items: a list, or a state or function of one (items that come
	-- and go, followed in place).
	items: { Item } | State<{ Item }> | () -> { Item },
	actions: { Action }?,
	tone: Tone?,
	-- The trigger's leading icon (or the chosen item's mark, with `mark`;
	-- with neither, the chosen item's icon, if items have them), and what it
	-- says with nothing chosen.
	icon: (Icon.Name | () -> Icon.Name)?,
	mark: boolean?,
	placeholder: (string | Message)?,
	-- A muted second half to the trigger's label (a model's effort).
	detail: (string | Message | () -> string)?,
	-- The most the chosen name takes, given: it ends in "…" past it, and the
	-- detail stays whole (the model chip's effort). Not given, the detail
	-- gives way first and a long name fades out.
	name_max: (() -> number)?,
	-- After the detail, at the trigger's end (the model chip's usage ring).
	trail: Node?,
	-- The label and detail roll only for a change made in the open panel; one
	-- from elsewhere (another chat's own choice shown) is there at once.
	roll_in_panel: boolean?,
	-- With `roll_in_panel`, true for a change made with the panel shut that
	-- rolls all the same (the effort stepped with ⌘− ⌘=).
	roll_also: (() -> boolean)?,
	-- The search field's placeholder (a function follows what it names);
	-- `false`, no search.
	search: (string | Message | boolean | () -> string)?,
	-- Which items show at all (a tab's), before the search narrows them.
	filter: ((item: Item) -> boolean)?,
	-- Called as the panel opens.
	on_open: (() -> ())?,
	-- Bumped, the panel opens, as a press on its trigger opens it (a
	-- shortcut that opens this menu).
	open_with: State<number>?,
	-- Bumped, the panel closes (a pick answered in a dialog).
	close_with: State<number>?,
	-- Set to whether the panel is open (the caller holding still what it
	-- hangs from while it is).
	showing: State<boolean>?,
	-- Set as it closes: whether that was from outside it (a press elsewhere,
	-- its trigger gone), not its keys, a pick or its trigger.
	outside: State<boolean>?,
	-- At most this many rows of those that match, and the line saying so.
	cap: number?,
	cap_words: ((shown: number, total: number) -> string)?,
	-- Whether `extra` (and its rule) shows; always, by default.
	extra_when: (() -> boolean)?,
	-- A row's own control at its end (a star to favourite it), given the
	-- item and whether its row is highlighted (under the pointer or the
	-- keys); and what an empty list shows, given what is searched for.
	trailing_item: ((item: Item, lit: () -> boolean) -> Node?)?,
	-- A row wrapped in something of the caller's (a submenu opening beside
	-- it as the pointer rests there), given the item and its row.
	wrap_item: ((item: Item, row: Node) -> Node)?,
	-- While false, no search and no list (and no rules for them): the head
	-- alone, in place (the panel stays open as it turns).
	list_when: (() -> boolean)?,
	-- While true, a ghost bar in its name's place and a spinner in its mark's:
	-- what it runs is still coming.
	ghost: (() -> boolean)?,
	-- With `mark` and no mark chosen, the glyph in its place (no agents).
	no_mark_icon: (() -> Icon.Name?)?,
	-- What the panel's contents fade by, changing page inside it.
	content_opacity: (() -> number)?,
	-- What the trigger says instead of the chosen item, while it says
	-- anything (a chip with nothing to choose: "No project").
	say: (() -> (string | Message)?)?,
	-- With `mark`: the mark the trigger shows while no item is chosen (the
	-- model chip naming a model its catalog doesn't list: its agent's).
	say_mark: (() -> Mark.Name?)?,
	empty: ((query: string) -> Empty)?,
	-- What it lists is still on its way: its chosen item's name and its rows
	-- hold their shape rather than saying there's nothing.
	loading: (() -> boolean)?,
	-- `keep_open`: a pick leaves the panel open (`on_choose` hears it);
	-- `on_escape` takes Escape first (true: handled, the panel stays).
	keep_open: boolean?,
	on_choose: ((value: string) -> boolean?)?,
	on_escape: (() -> boolean)?,
	-- A pick asked of it, before anything happens (true: the caller has taken
	-- it, a dialog to ask first; nothing changes here).
	take: ((value: string) -> boolean)?,
	-- The panel's keys, the caller's first, given the highlighted item and how
	-- to close the panel (true: handled).
	on_key: ((e: any, highlighted: string?, panel: { close: () -> (), choose: (value: string) -> () }) -> boolean)?,
	-- What shows and in what order, given every item and the search, in place
	-- of `filter` and the label's plain match.
	rank: ((items: { Item }, query: string) -> { Item })?,
	-- Its rows' height, and how tall the list gets before it scrolls.
	row_h: number?,
	view: (number | () -> number)?,
	-- While true the list is always as tall as `view`, whatever it holds: its
	-- panel's edge stays put across tabs and searches.
	list_full: (() -> boolean)?,
	-- Its search, scroll and highlight, when the caller moves them too.
	query: State<string>?,
	offset: State<number>?,
	highlight: State<string?>?,
	-- A row's own words after its name (who made it, when two share one).
	row_note: ((item: Item) -> string?)?,
	-- At the panel's top, above a rule, and under a rule at its foot: the
	-- caller's own rows. (`head_when`: the head and its rule only while true.)
	head: Node?,
	head_when: (() -> boolean)?,
	extra: Node?,
	-- While true, Tab moves among the panel's own fields (a form in its head)
	-- rather than leaving it.
	tab_inside: (() -> boolean)?,
	-- With `mark`, the chosen item's mark shows only while true (a chip whose
	-- mark another half carries).
	mark_when: (() -> boolean)?,
	-- The panel's width (232 by default).
	width: number?,
	-- The caller's own trigger in place of the chip (a row of another
	-- panel opening this one beside it, as a submenu).
	trigger: ((menu: Menu) -> Node)?,
	-- What the trigger says on hover, once folded to its glyph.
	tip: (() -> string?)?,
	-- A side-opening menu whose parent stays reachable by the pointer.
	submenu: boolean?,
	-- Shared hover-intent corridor coordinating a parent panel and its submenus.
	corridor: Menu.Corridor?,
	-- How far the trigger folds (a footer short of room): 0, all of it; 1,
	-- without its detail; 2, its leading icon alone. And while `accent` is
	-- true, that icon is in the accent (a mode on).
	fold: (() -> number)?,
	accent: (() -> boolean)?,
	-- Its ink there, when it isn't the accent (a warning).
	accent_ink: string?,
	-- What assistive technology calls it.
	label: Label?,
	-- Where the panel opens (zgx's popover placement); below, from the
	-- trigger's start, by default.
	placement: { side: string?, align: string?, offset: number? }?,
	-- The panel read bottom up (opening above its trigger): the head nearest
	-- the trigger, the search, then the list.
	bottom_up: boolean?,
}

-- A row of the panel: compact, full width. Actions wash under the pointer;
-- items wash where the highlight is (the pointer's or the keys'), so the
-- two never disagree.
-- (ROW is the menus' row, `zeron.menu.row`, here as a number: the list's
-- geometry is worked out from it.)
local ROW = 28
local function row(props: Merge<ContainerProps, { tall: boolean? }>, hover: boolean?): Node
	local p = table.clone(props)
	-- A row with a description grows to hold it, its tile on its title's
	-- line (the tile is a line high) and its check centred.
	local tall = p.tall == true
	p.tall = nil
	if hover == false then
		p.hover = {}
	end
	Menu.look(p, tall)
	return ui.row(p)
end

-- The list's geometry: its height before it scrolls, the inset above its
-- first row, and the room kept between the highlight and either edge as
-- the keys move it (a row: what comes next always shows).
local VIEW, INSET, GUTTER = 220, 4, ROW
-- Between rows: the theme's list gap (`zeron.list.gap`, read as the list
-- lays out: the themes load after this module).
local function gap(): number
	return (ui.theme.get("zeron.list.gap") :: number?) or 2
end
-- An empty list's height: room for its glyph, title and a two-line hint.
local EMPTY_H = 128

local function text(label: string | Message | () -> string, tone: string?): Node
	return FadeText {
		label,
		text_size = "$font.size.md",
		color = tone or "$text.primary",
		grow = 1,
	}
end

-- An item's tile: the first letter of its name, or its mark or icon.
local function tile(item: Item): Node
	if item.private then
		return Avatar { seed = item.label, size = 18, label = locale.t("ds.provider_card.account") }
	end
	if item.hue or item.src or item.emoji then
		return Tile { label = item.label, hue = item.hue, src = item.src, emoji = item.emoji, size = 18, project = item.project }
	end
	if item.icon then
		return Menu.glyph(item.icon)
	end
	if item.svg then
		return ui.div {
			w = 18,
			h = 18,
			shrink = 0,
			items = "center",
			justify = "center",
			AgentMark { label = item.label, seed = item.svg_seed, icon = item.svg, size = 14, bare = true },
		}
	end
	if item.mark then
		return ui.div { w = 18, h = 18, shrink = 0, items = "center", justify = "center", Mark { name = item.mark, size = 14, bare = true } }
	end
	local label = item.label
	return ui.div {
		w = 18,
		h = 18,
		shrink = 0,
		rounded = "$radius.sm",
		bg = "$zeron.secondary.bg",
		items = "center",
		justify = "center",
		ui.text {
			string.upper(string.sub(label, 1, 1)),
			font = "$font.mono",
			text_size = "$font.size.xs",
			weight = "$font.weight.semibold",
			color = "$accent",
		},
	}
end

-- Each tone's trigger: its size, its fill and ink at rest, under the pointer
-- (and open) and pressed, its label's ink (nil: the trigger's own), its
-- detail's (a step quieter than the label) and its leading icon's size.
local Tooltip = require("@self/tooltip")

local TRIGGER = {
	filled = {
		h = 26,
		px = "$space.2",
		gap = "$space.1",
		rounded = "$radius.md",
		text_size = "$font.size.sm",
		bg = "$zeron.secondary.bg",
		color = "$text.secondary",
		hover = { bg = "$zeron.secondary.hover", color = "$text.primary" },
		active = { bg = "$zeron.secondary.active", color = "$text.primary" },
		label = "$text.primary",
		detail = "$text.tertiary",
		glyph = 14,
	},
	ghost = {
		h = 32,
		px = "$zeron.space.tight",
		gap = "$zeron.space.tight",
		rounded = "$radius.md",
		text_size = "$font.size.sm",
		bg = "#00000000",
		color = "$text.primary",
		hover = { bg = "$hover" },
		active = { bg = "$pressed" },
		label = nil,
		detail = "$text.tertiary",
		glyph = 16,
	},
	chip = {
		-- (The composer's footer chips: its 24 and 8, its glyph 6 in all
		-- round.)
		h = 24,
		px = "$zeron.space.tight",
		gap = "$zeron.space.tight",
		rounded = "$radius.lg",
		text_size = "$font.size.sm",
		-- Its words lifted a point: centred by their capitals, their
		-- lowercase (most of what a chip says) reads low beside its glyph.
		-- (A margin under them of twice that.)
		lift = "$zeron.space.hair",
		bg = "#00000000",
		color = "$text.secondary",
		hover = { bg = "$hover", color = "$text.primary" },
		active = { bg = "$pressed", color = "$text.primary" },
		label = nil,
		detail = "$text.tertiary",
		glyph = 12,
	},
	-- The footer's model chip, as gpui's: the model's name bright, its effort a step quieter.
	model_chip = {
		h = 24,
		px = "$zeron.space.tight",
		gap = "$zeron.space.tight",
		rounded = "$radius.lg",
		text_size = "$font.size.sm",
		lift = "$zeron.space.hair",
		bg = "#00000000",
		color = "$text.primary",
		hover = { bg = "$hover" },
		active = { bg = "$pressed" },
		label = nil,
		detail = "$text.tertiary",
		glyph = 12,
	},
	-- In the pill, as gpui's: the model's name and its effort a quieter tone.
	model = {
		h = 32,
		px = "$zeron.space.tight",
		gap = "$zeron.space.tight",
		rounded = "$radius.lg",
		text_size = "$font.size.sm",
		lift = "$zeron.space.hair",
		bg = "#00000000",
		color = "$text.primary",
		hover = { bg = "$hover" },
		active = { bg = "$pressed" },
		label = nil,
		detail = "$text.tertiary",
		glyph = 16,
	},
	access = {
		h = 24,
		px = "$zeron.space.tight",
		gap = ui.raw(5, "gpui's access chip"),
		rounded = "$radius.md",
		text_size = "$font.size.xs",
		lift = "$zeron.space.hair",
		bg = "#00000000",
		color = "$text.secondary",
		hover = { bg = "$hover" },
		active = { bg = "$pressed" },
		label = nil,
		detail = "$text.disabled",
		glyph = 12,
	},
}

-- A chip's label: how wide it may grow, and how long its end fades when
-- it is longer.
local CHIP_LABEL, CHIP_FADE = 160, 24

-- A rule across the whole panel, edge to edge: the menus'.
local function rule(): Node
	return Menu.Rule()
end

return ui.component(function(props: Props): Node
	-- The items now, however they were given (read where they're shown, so
	-- what shows them follows them).
	local function items(): { Item }
		local given: any = props.items
		if type(given) == "function" then
			local call: () -> { Item } = given
			return call()
		elseif type(given) == "userdata" then
			local state: State<{ Item }> = given
			return state:get()
		end
		return given
	end
	local open = ui.use_state(false)
	Menu.hold_while(open)
	-- Closed by its own hand (its keys, a pick, its trigger), not from outside.
	local own_close = false
	local function shut()
		own_close = true
		open:set(false)
	end
	local query = props.query or ui.use_state("")
	local value = props.value
	local row_h = props.row_h or ROW
	local function view(): number
		local v = props.view
		if type(v) == "function" then
			return v()
		end
		return v or VIEW
	end
	-- A row's step down the list, and the list's height for `n` rows: its
	-- insets, the rows, the gaps between.
	local function step(): number
		return row_h + gap()
	end
	local function list_h(n: number): number
		return INSET * 2 + math.max(n * step() - gap(), 0)
	end
	-- Reading bottom up (`bottom_up`): the head and search by the trigger,
	-- the list over them, so a list changing length moves only the far edge.
	local bottom_up = props.bottom_up == true

	local function chosen(): string
		if props.say then
			local said = control.text(props.say())
			if said then
				return said
			end
		end
		local v = value:get()
		for _, item in items() do
			if item.value == v then
				return item.label
			end
		end
		return control.text(props.placeholder) or locale.s("ds.dropdown.placeholder")
	end

	-- Whether a search finds an item: by its name, else by its hint from the
	-- start of one of its words ("code" finds OpenCode's, "ai" not OpenAI's),
	-- or by its name and hint read together ("gpt-5.5 codex").
	local function word_start(text: string, q: string): boolean
		local low = string.lower(text)
		local at = string.find(low, q, 1, true)
		while at do
			local before = if at > 1 then string.sub(text, at - 1, at - 1) else " "
			-- A word starts after a space or a mark, or at a capital inside
			-- one (Open|Code).
			if not string.match(before, "%w") or (string.match(before, "%l") and string.match(string.sub(text, at, at), "%u")) then
				return true
			end
			at = string.find(low, q, at + 1, true)
		end
		return false
	end
	local function by_name(item: Item, q: string): boolean
		return q == "" or string.find(string.lower(item.label), q, 1, true) ~= nil
	end
	local function found(item: Item, q: string): boolean
		if by_name(item, q) then
			return true
		end
		local hint = item.hint
		if not hint then
			return false
		end
		if word_start(hint, q) then
			return true
		end
		-- Read together, the search runs from the name on into the hint,
		-- reaching it at a word's start (its first).
		local label = string.lower(item.label)
		local together = label .. " " .. string.lower(hint)
		local from, to = string.find(together, q, 1, true)
		while from do
			if from <= #label and (to :: number) > #label + 1 then
				return true
			end
			from, to = string.find(together, q, from + 1, true)
		end
		return false
	end
	-- Whether an item's hint shows now (`hint_when_found`: while a search
	-- found it by its hint, not its name).
	local function hint_shown(now: Item): boolean
		return now.hint ~= nil and (not now.hint_when_found or not by_name(now, string.lower(query:get())))
	end
	-- A name with its hint beside it, read as one line: where it runs past
	-- the row, its right edge fades (as FadeText's does), and while the row
	-- is highlighted (`lit`: under the pointer, or the keys') the line glides
	-- left until its end shows, rests, and glides back, over and over. With
	-- motion reduced it stays put, faded.
	local NAMED_FADE = 24
	local function named(item: Item, lit: () -> boolean): Node
		local clip, line = ui.ref(), ui.ref()
		local el: any = nil
		local run: any = nil
		-- How far the line runs past its row.
		local function over(): number
			return math.max(0, (line.width or 0) - (clip.width or 0))
		end
		local function still()
			if run then
				run:cancel()
				run = nil
			end
			if el then
				anime.utils.cleanInlineStyles(el, "x")
			end
		end
		ui.effect(function()
			local on = lit()
			local past = over()
			if not on or past <= 0.5 or motion.reduced() or not el then
				still()
				return
			end
			if run then
				return
			end
			local travel = past + NAMED_FADE
			run = anime.animate(el, {
				x = { 0, -travel },
				-- (About 40 points a second: read as it passes, not a jump.)
				duration = math.max(900, travel * 25),
				delay = 700,
				loopDelay = 1000,
				loop = true,
				alternate = true,
				ease = "inOut(2)",
			})
			ui.on_cleanup(still)
		end)
		return ui.row {
			ref = clip,
			grow = 1,
			min_w = 0,
			overflow = "hidden",
			items = "center",
			fade_x = function()
				local past = (line.width or 0) > (clip.width or 0) + 0.5
				-- (Gliding, its start runs off the left too.)
				local moving = past and lit() and not motion.reduced()
				return { if moving then NAMED_FADE else 0, if past then NAMED_FADE else 0 }
			end,
			ui.row {
				ref = line,
				shrink = 0,
				gap = "$space.2",
				items = "center",
				on_mount = function(node: any)
					el = node
				end,
				ui.text {
					item.label,
					shrink = 0,
					text_size = "$font.size.md",
					color = "$text.primary",
				},
				ui.show(function(): boolean
					return hint_shown(item)
				end, ui.text {
					item.hint :: string,
					shrink = 0,
					text_size = "$font.size.md",
					color = "$text.tertiary",
				}),
			},
		}
	end
	local matched = ui.computed(function(): { Item }
		if props.rank then
			return props.rank(items(), query:get())
		end
		local q = string.lower(query:get())
		local out: { Item } = {}
		local keep = props.filter
		for _, item in items() do
			if (keep == nil or keep(item)) and found(item, q) then
				table.insert(out, item)
			end
		end
		return out
	end)
	local cap = props.cap
	local shown = if cap
		then ui.computed(function(): { Item }
			local all = matched:get()
			return if #all > cap then table.move(all, 1, cap, 1, {}) else all
		end)
		else matched

	-- The highlighted row: an item (by value), or an action ("action:<n>");
	-- the keys run through the items, then on into the actions, and round. And the
	-- list's scroll offset.
	local hi = props.highlight or ui.use_state(nil :: string?)
	local offset = props.offset or ui.use_state(0)
	local function index_of(v: string?): number?
		for i, item in shown:peek() do
			if item.value == v then
				return i
			end
		end
		return nil
	end
	-- The list's height: its rows up to the view (a short list leaves no
	-- space under it), or an empty one's message.
	-- (Its rows as laid out: a row with a description under it is taller
	-- than a row, so the list is as tall as its rows really are.)
	local content_h = ui.use_state(0)
	local function list_height(): number
		if props.list_full and props.list_full() then
			return view()
		end
		local n = #shown:get()
		if n == 0 then
			return EMPTY_H
		end
		local measured = content_h:get()
		return math.min(view(), if measured > 0 then measured else list_h(n))
	end
	-- Scroll so row `i` keeps a gutter from either edge.
	local function reveal(i: number)
		local n = #shown:peek()
		local top = INSET + (i - 1) * step()
		local o = offset:peek()
		local most = math.max(0, list_h(n) - view())
		if top - GUTTER < o then
			offset:set(math.max(0, top - GUTTER))
		elseif top + row_h + GUTTER > o + view() then
			offset:set(math.min(most, top + row_h + GUTTER - view()))
		end
	end
	local actions: { Action } = props.actions or {}
	-- Every row the keys stop at, in order: the items shown, then the actions.
	local function stops(): { string }
		local out: { string } = {}
		for _, item in shown:peek() do
			if not item.disabled then
				table.insert(out, item.value)
			end
		end
		for n, action in actions do
			if not action.shown or action.shown() then
				table.insert(out, `action:{n}`)
			end
		end
		return out
	end
	local function move(by: number)
		local list = stops()
		if #list == 0 then
			return
		end
		local at = 0
		for i, v in list do
			if v == hi:peek() then
				at = i
			end
		end
		if at == 0 then
			at = if by > 0 then 0 else #list + 1
		end
		-- Round the ends: past the last, the first; before the first, the last.
		local i = (at + by - 1) % #list + 1
		hi:set(list[i])
		local n = #shown:peek()
		-- Into the actions: the list at its end, so the order reads on.
		reveal(math.min(i, math.max(n, 1)))
	end
	local function run(n: number)
		shut()
		local action = actions[n]
		if action and action.on_click then
			action.on_click()
		end
	end
	local function choose(v: string)
		local n = string.match(v, "^action:(%d+)$")
		if n then
			run(tonumber(n) :: number)
			return
		end
		-- (One not to be picked stays as it is.)
		for _, item in shown:peek() do
			if item.value == v and (item.disabled or item.inert) then
				return
			end
		end
		if props.take and props.take(v) then
			return
		end
		-- At once: the pick and what the caller does on it.
		ui.batch(function()
			value:set(v)
			local stays = props.on_choose ~= nil and props.on_choose(v) == false
			if not props.keep_open and not stays then
				shut()
			end
		end)
	end
	-- A search: the first match highlighted, the list at its top. No search
	-- (as it opens): the chosen item highlighted, in view.
	local function settle(list: { Item })
		local chosen_at = if query:peek() == "" then index_of(value:peek()) else nil
		offset:set(0)
		if chosen_at then
			hi:set(list[chosen_at].value)
			reveal(chosen_at)
		else
			hi:set(if list[1] then list[1].value else nil)
		end
	end

	-- The list as it changes: a search or another tab goes back to the top, on
	-- the chosen item when it's there; the same rows reordered (a star) stay
	-- where they are, the highlight and the scroll with them.
	local seen_query: string? = nil
	local seen_n = -1
	ui.effect(function()
		local list = shown:get()
		local q = query:peek()
		if q == seen_query and #list == seen_n then
			return
		end
		seen_query, seen_n = q, #list
		settle(list)
	end)
	local tone = props.tone or "filled"
	local function chosen_mark(): Mark.Name?
		local v = value:get()
		for _, item in items() do
			if item.value == v then
				return if item.svg then nil else item.mark
			end
		end
		return if props.say_mark then props.say_mark() else nil
	end
	-- (A registry agent's icon, in the mark's place: its seed, then the SVG.)
	local function chosen_svg(): string
		local v = value:get()
		for _, item in items() do
			if item.value == v then
				return if item.svg then `{item.svg_seed or item.label}\0{item.svg}` else ""
			end
		end
		return ""
	end
	-- Open, the trigger keeps the wash it has under the pointer.
	local trigger_props = table.clone(TRIGGER[tone]) :: any
	trigger_props.bg = function()
		return if open:get() then TRIGGER[tone].hover.bg else TRIGGER[tone].bg
	end
	local leading: Child = nil
	if props.mark then
		local glyph = TRIGGER[tone].glyph
		leading = ui.row {
			shrink = 0,
			if props.ghost
				then ui.show(props.ghost, Skeleton.Bar { w = glyph, h = glyph, rounded = "$radius.full" })
				else nil,
			if props.no_mark_icon
				then ui.show(function(): boolean
					return (props.no_mark_icon :: () -> Icon.Name?)() ~= nil
				end, Icon {
					name = function(): Icon.Name
						return (props.no_mark_icon :: () -> Icon.Name?)() or "terminal"
					end,
					size = glyph,
				})
				else nil,
			ui.show(function()
				return chosen_mark() ~= nil
			end, Mark {
				name = function(): Mark.Name
					return chosen_mark() or "claude"
				end,
				size = glyph,
				bare = true,
			}),
			ui.presence(chosen_svg, function(key: string): Node
				local seed, code = string.match(key, "^(.-)%z(.*)$")
				if not code or code == "" then
					return ui.div {}
				end
				return AgentMark { label = seed :: string, seed = seed, icon = code, size = glyph, bare = true }
			end),
		}
		if props.mark_when then
			leading = ui.show(props.mark_when, leading :: Node)
		end
	elseif props.icon then
		leading = Icon { name = props.icon, size = TRIGGER[tone].glyph }
	else
		local function chosen_icon(): Icon.Name?
			local v = value:get()
			for _, item in items() do
				if item.value == v then
					return item.icon
				end
			end
			return nil
		end
		local any_icon = false
		for _, item in items() do
			any_icon = any_icon or item.icon ~= nil
		end
		if any_icon then
			local function glyph(): Node
				return Icon {
					name = function(): Icon.Name
						return chosen_icon() or "check"
					end,
					size = TRIGGER[tone].glyph,
				}
			end
			local accent = props.accent
			leading = ui.show(function()
				return chosen_icon() ~= nil
			end, if accent
				then ui.div {
					shrink = 0,
					-- Two glyphs, so the plain one keeps following the
					-- trigger's own ink (and its hover).
					ui.show(accent, ui.div { color = props.accent_ink or "$accent", glyph() }),
					ui.show(function()
						return not accent()
					end, glyph()),
				}
				else glyph())
		end
	end
	local roll_also = props.roll_also
	local rolls = if props.roll_in_panel
		then function(): boolean
			return open:peek() or (roll_also ~= nil and roll_also())
		end
		else nil
	-- The detail, shown only while it says something (an empty one would
	-- still take the trigger's gap) and the trigger hasn't folded it away.
	local detail: Child = nil
	if props.detail then
		local d = props.detail
		local function said(): string
			return control.text(d) or ""
		end
		local fold = props.fold
		local whole = props.name_max ~= nil
		detail = ui.show(function()
			return said() ~= "" and (fold == nil or fold() < 1)
		end, FadeText {
			said,
			mb = TRIGGER[tone].lift,
			roll = true,
			rolls = rolls,
			color = TRIGGER[tone].detail,
			-- (Short of room it goes first, the name last; or, kept whole, the name.)
			min_w = if whole then nil else 0,
			shrink = if whole then 0 else 1000,
		})
	end
	-- The chosen item's name. A chip's is held to a width and, past it, fades
	-- out at its end rather than cutting to "…" (only while it overflows).
	local label_node: Node
	if TRIGGER[tone].lift then
		if props.name_max then
			-- (Held to its most, it ends in "…": the detail after it stays.)
			label_node = ui.row {
				mb = TRIGGER[tone].lift,
				min_w = 0,
				items = "center",
				max_w = props.name_max :: any,
				overflow = "hidden",
				RollText { chosen, rolls = rolls, shrink = 1, min_w = 0, truncate = true, color = TRIGGER[tone].label },
			}
		else
			local clip, text = ui.ref(), ui.ref()
			-- A row, so the text keeps its own width inside (a column would
			-- stretch it to the box's, and nothing would overflow).
			label_node = ui.row {
				ref = clip,
				-- (Lifted by a margin under it: the row stays centred, its words
				-- a point up, nothing outside its clip.)
				mb = TRIGGER[tone].lift,
				min_w = 0,
				items = "center",
				max_w = CHIP_LABEL,
				overflow = "hidden",
				fade_x = function()
					local over = (text.width or 0) > (clip.width or 0) + 0.5
					return { 0, if over then CHIP_FADE else 0 }
				end,
				RollText { chosen, rolls = rolls, ref = text, shrink = 0, color = TRIGGER[tone].label },
			}
		end
		if props.ghost then
			label_node = ui.row {
				min_w = 0,
				ui.show(props.ghost, Skeleton.Bar { w = 56, h = 10 }),
				ui.show(function(): boolean
					return not (props.ghost :: () -> boolean)()
				end, label_node),
			}
		end
	else
		label_node = FadeText { chosen, roll = true, rolls = rolls, color = TRIGGER[tone].label }
	end
	if props.loading then
		label_node = ui.show(props.loading, Skeleton.Bar { w = ui.raw(64, "about a model's name"), h = 10 }, label_node)
	end
	local corridor = props.corridor
	local corridor_item: Menu.CorridorItem? = nil

	-- Opening: the search cleared, on the chosen item, in view. (Open
	-- already, it stays as it is: asked again, by its shortcut or its
	-- trigger, it was put back folded, the search and tab lost.)
	local function show_panel()
		if open:peek() then
			return
		end
		if props.on_open then
			props.on_open()
		end
		if corridor and corridor_item then
			corridor:opened(corridor_item)
		end
		query:set("")
		local v = value:peek()
		if v then
			hi:set(v)
		end
		open:set(true)
		local i = index_of(v)
		if i then
			reveal(i)
		end
	end
	if props.open_with then
		local asked = props.open_with
		local seen = asked:peek()
		ui.effect(function()
			local n = asked:get()
			if n ~= seen then
				seen = n
				if not open:peek() then
					show_panel()
				end
			end
		end)
	end
	if props.close_with then
		local asked = props.close_with
		local seen = asked:peek()
		ui.effect(function()
			local n = asked:get()
			if n ~= seen then
				seen = n
				shut()
			end
		end)
	end
	if props.outside then
		local outside = props.outside
		local was = open:peek()
		ui.effect(function()
			local now = open:get()
			if was and not now then
				outside:set(not own_close)
			end
			own_close = false
			was = now
		end)
	end
	if props.showing then
		local showing = props.showing
		ui.effect(function()
			local now = open:get()
			if showing:peek() ~= now then
				showing:set(now)
			end
		end)
		ui.on_cleanup(function()
			if showing:peek() then
				showing:set(false)
			end
		end)
	end
	local function toggle()
		if open:peek() then
			shut()
		else
			show_panel()
		end
	end
	-- A hover-opened submenu stays open across the gap between its row
	-- and panel. When coordinated by a `corridor`, the corridor owns the
	-- hover-intent cone and closing timers; otherwise leaving both starts
	-- a short, cancellable closing grace.
	local over_trigger = false
	local over_panel = false
	local cancel_close: (() -> ())? = nil
	local function stop_close()
		if cancel_close then
			cancel_close()
			cancel_close = nil
		end
	end
	local function hover(inside: boolean, panel: boolean)
		if panel then
			over_panel = inside
		else
			over_trigger = inside
		end
		stop_close()
		if not over_trigger and not over_panel and open:peek() then
			cancel_close = ui.after(150, function()
				cancel_close = nil
				if not over_trigger and not over_panel then
					open:set(false)
				end
			end)
		end
	end
	if props.submenu then
		if corridor then
			local item = corridor:register(open, show_panel, function()
				open:set(false)
			end)
			corridor_item = item
			ui.on_cleanup(function()
				corridor:unregister(item)
			end)
			ui.effect(function()
				if not open:get() then
					corridor:closed(item)
				end
			end)
		else
			ui.on_cleanup(stop_close)
			ui.effect(function()
				if not open:get() then
					stop_close()
					over_panel = false
				end
			end)
		end
	elseif corridor then
		ui.on_cleanup(function()
			corridor:cleanup()
		end)
		ui.effect(function()
			if not open:get() then
				corridor:close_all()
			end
		end)
	end
	local trigger_ref = ui.ref()
	-- (Its trigger gone from view, as Settings stand over the page: closed, not left
	-- with nothing to hang from.)
	ui.effect(function()
		local w = trigger_ref.width
		if w ~= nil and w <= 0 and open:peek() then
			open:set(false)
		end
	end)
	local trigger = if props.trigger then props.trigger({
		open = open,
		toggle = toggle,
		on_mount = if props.submenu and corridor and corridor_item
			then function(el: Element)
				corridor:mount_trigger(corridor_item :: Menu.CorridorItem, el)
			end
			else nil,
		on_hover = if props.submenu
			then if corridor and corridor_item
				then function(inside: boolean)
					local item = corridor_item :: Menu.CorridorItem
					if inside then
						corridor:enter_trigger(item)
					else
						corridor:leave_trigger(item)
					end
				end
				else function(inside: boolean)
					hover(inside, false)
				end
			else nil,
		on_pointer_move = if props.submenu and corridor and corridor_item
			then function(ev: PointerEvent)
				corridor:move_trigger(corridor_item :: Menu.CorridorItem, { x = ev.x, y = ev.y })
			end
			else nil,
		on_click = if props.submenu and corridor and corridor_item
			then function()
				corridor:click_trigger(corridor_item :: Menu.CorridorItem)
			end
			else nil,
	}) else ui.button {
		ref = trigger_ref,
		label = props.label,
		h = trigger_props.h,
		px = trigger_props.px,
		gap = trigger_props.gap,
		items = "center",
		rounded = trigger_props.rounded,
		text_size = trigger_props.text_size,
		weight = "$font.weight.medium",
		min_w = 0,
		bg = trigger_props.bg,
		color = trigger_props.color,
		hover = trigger_props.hover,
		active = trigger_props.active,
		-- (Open, a press on it closes it, as a menu's trigger does.)
		on_click = toggle,
		-- (A trigger's tooltip is fixed once made; this one comes and goes with its fold.)
		if props.tip
			then ui.div {
				relative = true,
				shrink = 0,
				leading,
				ui.show(function(): boolean
					return (props.tip :: () -> string?)() ~= nil
				end, ui.div {
					absolute = true,
					left = -6,
					right = -6,
					top = -6,
					bottom = -6,
					tooltip = Tooltip {
						function(): string
							return (props.tip :: () -> string?)() or ""
						end,
					},
				}),
			}
			else leading,
		if props.fold
			then ui.show(function()
				return (props.fold :: () -> number)() < 2
			end, label_node)
			else label_node,
		detail,
		props.trail,
		if tone == "filled" then Icon { name = "alt-arrow-down", size = 12 } else nil,
	}

	-- The menus' glass (frosted: a light tint over the platform's own
	-- popover material; opaque: a solid panel). No padding of its own: its
	-- rules run edge to edge; each section insets itself.
	local panel = Menu.panel {
		-- (Named for tests.)
		class = "dropdown-panel",
		open = open,
		anchor = trigger,
		placement = props.placement or { side = "bottom", align = "start", offset = 6 },
		w = props.width or "$zeron.menu.width",
		build = function()
			-- The keys: up and down move the highlight, Enter takes it.
			local function keys(e): boolean
				if props.on_key and props.on_key(e, hi:peek(), {
					close = function()
						shut()
					end,
					choose = choose,
				}) then
					return true
				end
				-- (Ctrl+N and Ctrl+P too, as Zeron's menus take them.)
				if e.key == "ArrowDown" or (e.ctrl and e.key == "n") then
					move(1)
					return true
				elseif e.key == "ArrowUp" or (e.ctrl and e.key == "p") then
					move(-1)
					return true
				elseif e.key == "Enter" then
					local v = hi:peek()
					if v then
						choose(v)
					end
					return true
				elseif e.key == "Escape" then
					if props.on_escape and props.on_escape() then
						return true
					end
					-- (The popover's own Escape closes it.)
					own_close = true
					return false
				elseif e.key == "Tab" then
					if props.tab_inside and props.tab_inside() then
						return false
					end
					-- Tab leaves it, as it leaves a menu: closed, the keys
					-- back on its trigger, to go on from there.
					shut()
					local el = trigger_ref:peek()
					if el then
						el:focus()
					end
					return true
				end
				return false
			end
			-- What a search browses: the head, the search, the list, the
			-- actions.
			local function browse(): { Node }
				local rows: { Node } = {}
				-- The head and the search, folding together: above the list,
				-- each over its rule; opening upward, below it (the search,
				-- then the head, by the trigger), each under its rule.
				local top: { Node } = {}
				-- No list at all (no search, nothing to list, nothing to say
				-- for it): the head alone, no rule under it.
				local listless = props.search == false and #items() == 0 and props.empty == nil
				-- Behind `list_when`: all but the head.
				local function gated(node: Node): Node
					return if props.list_when then ui.show(props.list_when, node) else node
				end
				local function section(node: Node, is_head: boolean?)
					local shown = if is_head then node else gated(node)
					local ruled = gated(rule())
					local head_when = props.head_when
					if is_head and head_when then
						shown = ui.show(head_when, node)
						local list_when = props.list_when
						ruled = ui.show(function(): boolean
							return head_when() and (list_when == nil or list_when())
						end, rule())
					end
					if listless then
						table.insert(top, shown)
					elseif bottom_up then
						table.insert(top, ruled)
						table.insert(top, shown)
					else
						table.insert(top, shown)
						table.insert(top, ruled)
					end
				end
				local head = if props.head then ui.column { w_full = true, p = "$space.1", gap = "$zeron.list.gap", props.head } else nil
				if head and not bottom_up then
					section(head, true)
				end
				-- The search, filtering as it is typed.
				if props.search ~= false then
					local given: any = props.search
					local named = type(given) == "string" or control.is_message(given)
					local search = if named or type(given) == "function"
						then given
						else locale.t("ds.dropdown.search_placeholder")
					local field = ui.row {
						h = 36,
						px = "$space.3",
						gap = "$space.2",
						items = "center",
						color = "$text.tertiary",
						Icon { name = "search", size = 14 },
						ui.input(EditorMenu.on({
							value = query,
							-- (A function's search is named "Search": the tests
							-- and assistive technology find it by that name.)
							label = if named or type(given) ~= "function" then search else locale.t("common.search"),
							placeholder = search,
							grow = 1,
							h = 24,
							px = 0,
							border = 0,
							bg = "#00000000",
							-- No ring or border of its own: the panel shows where
							-- focus is.
							focus = { border = 0, outline = 0 },
							text_size = "$font.size.md",
							on_key = keys,
							-- Typing filters as the panel opens, whatever is
							-- above the search (a head's rows): focused after
							-- the popover gives focus to its first row.
							on_mount = function(el)
								ui.after(0, function()
									el:focus()
								end)
							end,
						})),
					}
					section(field)
				end
				if head and bottom_up then
					section(head, true)
				end
				-- Put in place above the list (or, upward, below it).
				local function place_top()
					for _, node in top do
						table.insert(rows, node)
					end
				end
				if not bottom_up then
					place_top()
				end
				-- A row's own control at its end.
				local function trailing(item: Item): Node?
					return if props.trailing_item
						then props.trailing_item(item, function(): boolean
							return hi:get() == item.value
						end)
						else nil
				end
				local items: { Node } = {}
				table.insert(
					items,
					ui.each(shown, function(item: Item)
						local r = row({
							role = "option",
							-- A row with a hint is heard as it reads: its name,
							-- then its hint while that shows.
							label = if item.hint
								then function(): string
									return if hint_shown(item) then `{item.label} {item.hint}` else item.label
								end
								else nil,
							-- What assistive technology hears the selected
							-- state as (the check it no longer shows).
							selected = function(): boolean
								return value:get() == item.value
							end,
							h = if item.description == nil and row_h ~= ROW then row_h else nil,
							tooltip = item.tip,
							tooltip_delay = if item.tip then 500 else nil,
							bg = function()
								-- The chosen item in the selected state (as a
								-- sidebar's row), the highlight (pointer or keys) a
								-- wash on the rest.
								if value:get() == item.value then
									return "$zeron.popup.selected"
								end
								return if hi:get() == item.value then "$hover" else "#00000000"
							end,
							on_hover = function(inside: boolean)
								if inside then
									hi:set(item.value)
								end
							end,
							on_click = function()
								choose(item.value)
							end,
							tall = item.description ~= nil,
							opacity = if item.disabled then ui.raw(0.45, "a choice out of reach, dimmed") else nil,
							cursor = if item.disabled then "default" else nil,
							if item.ink then ui.div { color = item.ink, tile(item) } else tile(item),
							if item.description
								then ui.column {
									grow = 1,
									min_w = 0,
									gap = "$zeron.space.hair",
									if item.private
										then PrivateText {
											item.label,
											reveal = false,
											truncate = true,
											text_size = "$font.size.md",
											line_height = "$font.line.md",
											weight = "$font.weight.medium",
											color = "$text.primary",
										}
										else ui.text {
											item.label,
											text_size = "$font.size.md",
											line_height = "$font.line.md",
											weight = "$font.weight.medium",
											color = item.ink or "$text.primary",
										},
									ui.text {
										item.description,
										text_size = "$font.size.sm",
										line_height = "$font.line.sm",
										color = "$text.tertiary",
										wrap = true,
									},
								}
								elseif item.hint then named(item, function(): boolean
									return hi:get() == item.value
								end)
								else text(item.label, item.ink),
							if props.row_note
								then (function(): Node
									local note = (props.row_note :: (Item) -> string?)(item)
									return if note
										then ui.text { note, shrink = 1, min_w = 0, truncate = true, text_size = "$font.size.sm", color = "$text.tertiary" }
										else nil :: any
								end)()
								else nil,
							if item.tag
								then ui.text {
									item.tag,
									self = "center",
									shrink = 0,
									text_size = "$font.size.xs",
									color = "$text.tertiary",
								}
								else nil,
							-- Out of reach: Wi-Fi off, in the warning's ink (a glyph,
							-- not the word, as Zeron's device menu shows it).
							if item.warn
								then ui.div { self = "center", shrink = 0, color = "$warning", Icon { name = "wifi-off", size = 12 } }
								else nil,
							trailing(item),
						}, false)
						if props.wrap_item then
							r = props.wrap_item(item, r)
						end
						if item.heading then
							return ui.column { w_full = true, Menu.Heading { item.heading }, r }
						end
						return r
					end, {
						-- (A row whose hint changes, or comes or goes, is built
						-- again: its hint is part of what it is.)
						key = function(item: Item)
							return if item.hint
								then `{item.value}\0{item.hint}\0{if item.hint_when_found then "found" else "always"}`
								else item.value
						end,
						w_full = true,
					})
				)
				-- Nothing to list: the list's room as a quiet field of
				-- characters (the slider's, in one faint ink), clearing in the
				-- middle for what is missing (a mono label, as the panel's
				-- section titles) and how to fill it.
				local function said(): Empty
					local q = query:get()
					if props.empty then
						return props.empty(q)
					end
					return {
						title = locale.s("ds.dropdown.no_matches"),
						hint = if q ~= "" then locale.s("ds.dropdown.nothing_matches", { query = q }) else nil,
					}
				end
				table.insert(
					items,
					ui.show(function()
						return #shown:get() == 0 and props.loading ~= nil and props.loading()
					end, ui.column {
						w_full = true,
						gap = "$zeron.list.gap",
						Skeleton.Bar { w_full = true, h = ROW, rounded = Menu.row_radius() },
						Skeleton.Bar { w_full = true, h = ROW, rounded = Menu.row_radius() },
						Skeleton.Bar { w_full = true, h = ROW, rounded = Menu.row_radius() },
					})
				)
				table.insert(
					items,
					ui.show(function()
						return #shown:get() == 0 and not (props.loading ~= nil and props.loading())
					end, ui.column {
						w_full = true,
						h = EMPTY_H - INSET * 2,
						relative = true,
						overflow = "hidden",
						rounded = Menu.row_radius(),
						-- The field thins out toward the list's top and foot.
						fade = { 16, 16 },
						px = "$space.4",
						gap = "$zeron.space.tight",
						items = "center",
						justify = "center",
						-- (A failed state's action and hint stand taller than the art's clearing.)
						ui.show(function(): boolean
							return said().action == nil
						end, ui.div {
							absolute = true,
							left = 0,
							right = 0,
							top = 0,
							bottom = 0,
							pointer_events = "none",
							bg_shader = ascii.QUIET,
							shader_params = ascii.quiet("$text.tertiary", 0.15, { 100, 20 }),
							on_mount = ascii.drift,
						}),
						ui.text {
							function(): string
								return string.upper(control.text(said().title) or "")
							end,
							font = "$font.mono",
							text_size = "$font.size.xs",
							letter_spacing = 1,
							color = "$text.secondary",
						},
						ui.show(function(): boolean
							return said().skeleton == true
						end, ui.column {
							absolute = true,
							left = "$space.1",
							right = "$space.1",
							top = "$space.1",
							gap = "$space.3",
							p = "$space.2",
							Skeleton.Bar { w_full = true, h = 12 },
							Skeleton.Bar { w_full = true, h = 12 },
							Skeleton.Bar { w_full = true, h = 12 },
							Skeleton.Bar { w_full = true, h = 12 },
						}),
						ui.show(function(): boolean
							return said().action ~= nil
						end, ui.button {
							label = function(): string
								local a = said().action
								return if a then a.label else ""
							end,
							px = "$space.2",
							h = 24,
							items = "center",
							rounded = "$radius.md",
							bg = "$zeron.secondary.bg",
							hover = { bg = "$zeron.secondary.hover" },
							text_size = "$font.size.sm",
							color = "$text.primary",
							on_click = function()
								local a = said().action
								if a then
									a.on_click()
								end
							end,
							ui.text {
								function(): string
									local a = said().action
									return if a then a.label else ""
								end,
							},
						}),
						ui.show(function(): boolean
							return said().hint ~= nil
						end, Text {
							function(): string
								return control.text(said().hint) or ""
							end,
							style = "caption",
							tone = "tertiary",
							align = "center",
							wrap = true,
							w_full = true,
						}),
					})
				)
				-- The items scroll past about seven rows; the search and the
				-- actions stay put. Without a search there are only a few: they
				-- sit as they are.
				if listless then
					-- (Nothing to list.)
				elseif props.search == false then
					table.insert(rows, gated(ui.column { w_full = true, p = "$space.1", gap = "$zeron.list.gap", table.unpack(items) }))
				else
					local list = ui.scroll {
						w_full = true,
						-- As tall as its rows, up to the view: a short list leaves
						-- no empty space under it.
						h = list_height,
						offset = offset,
						-- Each edge fades only where there is more to scroll that way
						-- (in the list as it will stand), easing out as the end
						-- comes.
						fade = function()
							local list = shown:get()
							local measured = content_h:get()
							local whole = if measured > 0 then measured else list_h(#list)
							local most = math.max(0, whole - list_height())
							local o = offset:get()
							return { math.min(o, 12), math.min(math.max(0, most - o), 12) }
						end,
						ui.column {
							w_full = true,
							p = "$space.1",
							gap = "$zeron.list.gap",
							on_resize = function(size: { w: number, h: number })
								if math.abs(size.h - content_h:peek()) >= 0.5 then
									content_h:set(size.h)
								end
							end,
							table.unpack(items),
						},
					}
					table.insert(rows, gated(list))
				end
				if props.actions and #props.actions > 0 then
					local all = props.actions
					-- (The rule and the actions' inset only while an action
					-- shows: a menu whose every action is hidden ends at its
					-- last item, with no line or gap under it.)
					local function any_shown(): boolean
						for _, action in all do
							if not action.shown or action.shown() then
								return true
							end
						end
						return false
					end
					local acts: { Node } = {}
					for n, action in props.actions do
						local key = `action:{n}`
						local made = row({
							color = "$text.secondary",
							bg = function()
								return if hi:get() == key then "$hover" else "#00000000"
							end,
							on_hover = function(inside: boolean)
								if inside then
									hi:set(key)
								end
							end,
							on_click = function()
								run(n)
							end,
							Menu.glyph(action.icon),
							text(action.label),
						}, false)
						table.insert(acts, if action.shown then ui.show(action.shown, made) else made)
					end
					table.insert(
						rows,
						ui.show(
							any_shown,
							ui.column {
								w_full = true,
								gap = 0,
								rule(),
								ui.column { w_full = true, p = "$space.1", gap = "$zeron.list.gap", table.unpack(acts) },
							}
						)
					)
				end
				if bottom_up then
					place_top()
				end
				return rows
			end
			local rows: { Node } = {}
			for _, row in browse() do
				table.insert(rows, row)
			end
			if cap and props.cap_words then
				local words = props.cap_words
				table.insert(rows, ui.show(function(): boolean
					return #matched:get() > cap
				end, ui.text {
					function(): string
						return words(cap, #matched:get())
					end,
					p = "$space.1",
					text_size = "$font.size.xs",
					color = "$text.disabled",
				}))
			end
			if props.extra then
				local foot = ui.column { w_full = true, rule(), ui.column { w_full = true, p = "$space.1", gap = "$zeron.list.gap", props.extra } }
				table.insert(rows, if props.extra_when then ui.show(props.extra_when, foot) else foot)
			end
			local function finish(col: { [any]: any }): Node
				if corridor and not props.submenu then
					col.on_pointer_move = function(ev: PointerEvent)
						corridor:move_parent({ x = ev.x, y = ev.y })
					end
					col.on_hover = function(inside: boolean)
						if not inside then
							corridor:leave_parent()
						end
					end
				end
				return ui.column(col)
			end
			if props.search == false then
				-- No field to type in: the panel itself takes the keys, focused
				-- as it opens.
				return finish {
					-- The panel's full width, so its rules reach both edges
					-- (not the width of its widest row).
					w_full = true,
					gap = 0,
					-- (It has the keys: what it is, for assistive technology.)
					role = "listbox",
					label = props.label,
					focusable = true,
					focus = { outline = 0 },
					on_mount = function(el)
						el:focus()
					end,
					on_key = keys,
					table.unpack(rows),
				}
			end
			-- A list that can go (`list_when`): the panel itself holds the focus
			-- when it does, as the search it was in goes with it (a popover
			-- loses its focus, and closes, otherwise), and takes the keys.
			if props.list_when then
				local list_when = props.list_when
				local root: any = nil
				ui.effect(function()
					if not list_when() and root then
						-- At once (before the search can take the focus with it),
						-- and again once the list has gone.
						local el = root
						el:focus()
						ui.after(0, function()
							el:focus()
						end)
					end
				end)
				return finish {
					w_full = true,
					gap = 0,
					-- (A listbox is one Tab stop: a form in its head, Tab walking
					-- it, needs a plain group.)
					role = if props.tab_inside then "group" else "listbox",
					label = props.label,
					focusable = true,
					focus = { outline = 0 },
					opacity = props.content_opacity,
					on_key = keys,
					on_mount = function(el)
						root = el
						if not list_when() then
							ui.after(0, function()
								el:focus()
							end)
						end
					end,
					table.unpack(rows),
				}
			end
			-- Escape from whatever in it has the keys (the slider, a tab, a
			-- star, not only the search) is the panel's first: opened out,
			-- it folds it back, rather than reaching the popover, which
			-- closes.
			local on_escape = props.on_escape
			return finish {
				w_full = true,
				gap = 0,
				on_key = if on_escape
					then function(e): boolean
						return e.key == "Escape" and on_escape()
					end
					else nil,
				table.unpack(rows),
			}
		end,
	}
	if props.submenu then
		local build = panel.build
		panel.build = nil
		panel.anchor = nil
		panel.trigger = trigger
		panel.label = props.label
		panel[1] = ui.presence(function(): boolean
			return open:get()
		end, function(visible: boolean): Node
			if not visible then
				return ui.div {}
			end
			return ui.column {
				w_full = true,
				on_mount = if corridor and corridor_item
					then function(el: Element)
						corridor:mount_panel(corridor_item :: Menu.CorridorItem, el)
					end
					else nil,
				on_hover = function(inside: boolean)
					if corridor and corridor_item then
						local item = corridor_item :: Menu.CorridorItem
						if inside then
							corridor:enter_panel(item)
						else
							corridor:leave_panel(item)
						end
					else
						hover(inside, true)
					end
				end,
				build(),
			}
		end)
		return ui.submenu(panel)
	end
	return ui.popover(panel)
end)
