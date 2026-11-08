-- Zeron's composer, from the design system's parts: a pill holding, on one
-- line with the draft, the paperclip and plan's switch on its left, and the
-- access mode, the model and Send on its right; under it a footer of where
-- it works (a chat's checkout and branch; a new chat's device, branch and
-- checkout, its project in the pill) and the ring for how much of the
-- model's context is used. Its geometry is Zeron's
-- (crates/ui/src/composer.rs): 26-point corners, 28-point round controls,
-- one 49-point line while the draft fits beside the controls.
--
-- Past that (or always, with `expanded`, as a new chat's is) it opens out:
-- the text on top, growing with it from 76 to 260 points and scrolling past
-- that, the controls in a row beneath. The pill grows from its bottom edge
-- (it sits at the foot of a column), the controls stay put, and only the
-- text moves.
--
-- Everything else is in the pill or behind it. In it, over the draft: the
-- cargo (ds/composer_cargo: what the message carries, as chips), or what
-- the agent asks (ds/composer_ask: a question's options, an approval), the
-- draft then being the answer. Behind it: the trays (ds/composer_dock: the
-- queue, what is running, the plan), tucked under the pill, one open at a
-- time. Above all of it, a line of notice when there is one (offline, a
-- failed send, a draft restored): small text with a dot, never a box.
-- Mentions (ds/composer_mentions) open at the caret. Dictating, a track
-- unrolls from the microphone over the controls: a dot, "Listening", the
-- voice, a clock.
--
-- `Composer { value = draft, model = model, models = { … }, on_send = … }`.

local composer_surface = require("@self/composer_surface")
local Button = require("@self/button")
local Icon = require("@self/icon")
local Kbd = require("@self/kbd")
local Menu = require("@self/menu")
local control = require("@self/control")
local FadeText = require("@self/fade_text")
local Dropdown = require("@self/dropdown")
local ModelPicker = require("@self/model_picker")
local ProfilePicker = require("@self/profile_picker")
local AgentMark = require("@self/agent_mark")
local profile_look = require("@self/profile_look")
local IconButton = require("@self/icon_button").IconButton
local status = require("@self/status")
local markdown = require("@self/markdown_edit")
local material = require("@self/material")
local Tooltip = require("@self/tooltip")
local DropOverlay = require("@self/drop_overlay")
local cargo = require("@self/composer_cargo")
local Lightbox = require("@self/lightbox")
local dock = require("@self/composer_dock")
local ask = require("@self/composer_ask")
local mentions = require("@self/composer_mentions")
local locale = require("@self/locale")
local context_usage = require("@self/context_usage")
local Compaction = require("@self/compaction")
local SessionStats = require("@self/session_stats")
local motion = require("@self/motion")
local EditorMenu = require("@self/editor_menu")
local limit_notice = require("@self/composer_limit")

-- What the model's context holds, as its agent reports it: the tokens in
-- it and the window's size (either may be missing).
export type Usage = context_usage.Usage

-- A device it can run on: its name; Cloud (a machine of its own), this
-- device ("You"), or one out of reach.
-- `disabled`: shown but not to be picked, `description` saying why.
export type Device = {
	value: string,
	label: string,
	cloud: boolean?,
	you: boolean?,
	offline: boolean?,
	description: string?,
	disabled: boolean?,
}
-- A line above the composer: its tone (a dot in its ink; danger, an
-- "Error" badge), what it says, a spinner in place of the dot while
-- something is on its way, an action at its end (its name; the composer's
-- `on_notice_action` does it), and whether it can be put away
-- (`on_dismiss_notice`). Data alone: a state holds no functions.
export type Notice = {
	tone: ("neutral" | "info" | "success" | "warning" | "danger")?,
	text: string,
	spinner: boolean?,
	action: string?,
	dismissable: boolean?,
}
export type Answer = { values: { string }, text: string }
-- Where an existing chat works (`branch_fixed`).
export type Fixed = { name: string?, worktree: boolean, project: any, hue: string?, src: string?, emoji: string?, icon: Icon.Name?, label: string?, removed: boolean?, badge: any? }
export type Props = {
	value: State<string>,
	placeholder: (string | Message)?,
	-- What sends (the shipping app's setting): "enter", ↩ sends and ⇧↩
	-- breaks the line; "mod_enter", ⌘↩ sends and ↩ breaks the line. ⌘↩
	-- sends either way. Enter by default.
	send_with: (() -> string)?,
	-- A name to keep its measures and footer fold under, across the
	-- composer being made again (a reload), so it is laid out as it was
	-- from its first frame rather than growing into place. Without one,
	-- each composer measures afresh.
	keep: string?,
	-- Always opened out (a new chat's), rather than one line while it fits.
	expanded: (boolean | () -> boolean)?,
	-- Moved as the composer is shown for something else (a new chat's
	-- page or a session's, as the route switches): its form and size are
	-- there at once for a moment after, not eased. Growing in place (more
	-- lines, a tray, chips) still eases.
	snap: State<number>?,
	-- A turn is running: Send becomes Stop while the draft is empty, and
	-- queues what is typed.
	running: State<boolean>?,
	-- What the agent may do unasked: "supervised", "edits" or "full"; and
	-- plan mode (explore and propose before changing anything).
	access: State<string?>?,
	plan: State<boolean>?,
	-- Whether the agent has a plan mode (by default, yes), and whether it
	-- asks before going on from a plan (Start plan, and the words for it).
	plan_available: (() -> boolean)?,
	plan_asks: (() -> boolean)?,
	-- The access modes the agent offers (nil: all, as while its catalog
	-- comes), and its name for the note when only one is.
	access_offered: (() -> { string }?)?,
	access_note: (() -> string?)?,
	-- What the agent would have you say next (nil: nothing): over an empty
	-- draft, or after one it starts with; Tab takes it.
	prediction: (() -> string?)?,
	-- A stopped turn can go on, with nothing written.
	resume: (() -> boolean)?,
	on_resume: (() -> ())?,
	field: any?,
	-- A usage limit reached (ds/composer_limit), over the draft.
	limit: (() -> any)?,
	-- The tab tucked behind the pill's top right corner.
	side_tab: any?,
	pills: Node?,
	-- gpui's review chip beside the paperclip, and its findings over the pill (screens/chat/review).
	review: Node?,
	review_findings: Node?,
	-- A line under the trays, over the pill (an agent that can't be steered).
	caption: (() -> string?)?,
	-- Why sending wouldn't do now: the button dimmed and saying so, a press
	-- still going to `on_send`, which explains.
	send_block: (() -> string?)?,
	-- The model (a ModelPicker: its agents' tabs, its models, its reasoning,
	-- Fast mode and its other options' picks).
	model: State<string?>,
	effort: State<string?>,
	fast: State<boolean>?,
	options: State<{ [string]: string }>?,
	-- The agents and their models (ds.ModelPicker's: lists, or states or
	-- functions of them, followed in place as a catalog comes and changes).
	harnesses: { ModelPicker.Harness } | State<{ ModelPicker.Harness }> | () -> { ModelPicker.Harness },
	models: { ModelPicker.Model } | State<{ ModelPicker.Model }> | () -> { ModelPicker.Model },
	-- A model's agent's accounts, putting one in use, and asking after them
	-- again (ds.ModelPicker's).
	accounts: ((harness: string, model: string) -> { ModelPicker.Account }?)?,
	on_account: ((harness: string, id: string) -> ())?,
	on_accounts_open: (() -> ())?,
	-- How much of the model's context is used (0 to 1): a ring beside its
	-- chip.
	context: (number | () -> number)?,
	-- Or what it holds (tokens and window, as the agent reports them): the
	-- ring shows only once a window is reported (an agent that never says
	-- one has none), and hovering or focusing it opens a card saying the
	-- numbers, as Zeron's does (crates/ui/src/context_usage.rs).
	usage: (() -> Usage?)?,
	-- Where it works: the device, the checkout ("local" or "worktree") and
	-- the branch, one chip; its panel lists the branches under rows
	-- choosing the device and the checkout.
	device: State<string?>?,
	devices: ({ Device } | State<{ Device }> | () -> { Device })?,
	checkout: State<string?>?,
	stats: (() -> SessionStats.Stats?)?,
	working: (() -> boolean)?,
	on_terminal: (() -> ())?,
	terminal_open: (() -> boolean)?,
	terminal_keys: (() -> string)?,
	footer_git: (() -> boolean)?,
	footer_status: boolean?,
	branch: State<string?>?,
	branches: ({ Dropdown.Item } | State<{ Dropdown.Item }> | () -> { Dropdown.Item })?,
	-- What the branches' list says with none to show (still loading, none
	-- found, none matching what is searched for).
	branches_empty: ((query: string) -> Dropdown.Empty)?,
	-- Where an existing chat works, fixed (its branch, live, whether its
	-- folder is a worktree, and its project's name, hue and artwork): the
	-- chip says so and opens nothing, as a chat can't move. Nil (a new
	-- chat): the picker.
	branch_fixed: (() -> Fixed?)?,
	-- Its branch chip's menu: the project's refs (screens/chat/session_place).
	branch_menu: any?,
	-- `false`: the draft stays as it stands (nothing went, or it was taken already).
	on_send: ((text: string, opts: { inverted: boolean? }?) -> boolean?)?,
	on_stop: (() -> ())?,
	-- A bare Escape no one else took (no mention list, question or
	-- dictation): true if it did something with it.
	on_escape: (() -> boolean)?,
	on_attach: (() -> ())?,
	-- Its menus open below it (a composer with room under it: a new
	-- session's, floating mid-page); by default, above (one at the window's
	-- foot). A function follows where the composer is (one travelling
	-- between the two).
	menus_below: (boolean | () -> boolean)?,
	-- The project it works in ("none": no project), the projects to pick
	-- from, and making a new one.
	project: State<string?>?,
	projects: ({ Project } | State<{ Project }> | () -> { Project })?,
	on_new_project: (() -> ())?,

	-- What the message carries (chips over the draft), and what to do about
	-- one: take it off, try its upload again, open it.
	cargo: State<{ cargo.Item }>?,
	on_remove_cargo: ((id: string) -> ())?,
	on_retry_cargo: ((id: string) -> ())?,
	on_open_cargo: ((id: string) -> ())?,
	-- Text pasted in: true takes it (the caller made a chip of it).
	on_paste_text: ((text: string) -> boolean)?,
	-- A pasted chip pressed: its words read, put back through `insert`.
	on_open_pasted: ((id: string, insert: (text: string) -> boolean) -> ())?,
	-- A paste with no text (a picture on the clipboard): the caller's to
	-- read, true taking it. Files copied in a file manager go to `on_files`.
	on_paste_image: (() -> boolean)?,
	-- Files dropped on the pill.
	on_files: ((files: { any }) -> ())?,
	-- Files are being dragged over it (the caller's, for a state of its
	-- own; otherwise the pill's own sense of it).
	dropping: State<boolean>?,

	-- The trays behind it (ds/composer_dock's).
	queue: State<{ dock.Queued }>?,
	can_steer: (boolean | () -> boolean?)?,
	on_queue_send: ((id: string) -> ())?,
	on_queue_steer_all: (() -> ())?,
	queue_pictures: any?,
	on_queue_edit: ((id: string) -> ())?,
	on_queue_remove: ((id: string) -> ())?,
	on_queue_reorder: ((from: number, to: number) -> ())?,
	on_queue_save: ((id: string) -> ())?,
	on_queue_cancel: ((id: string) -> ())?,
	todos: State<{ dock.Todo }>?,
	on_dismiss_todos: (() -> ())?,
	tasks: State<{ dock.Task }>?,
	on_stop_task: ((id: string) -> ())?,
	on_stop_all: (() -> ())?,
	tray: State<dock.Tray?>?,

	-- What the agent asks: a question (its chosen options kept by the
	-- caller), or an approval.
	question: State<ask.Question?>?,
	chosen: State<{ [string]: boolean }>?,
	on_pick: ((value: string) -> ())?,
	on_answer: ((answer: Answer) -> ())?,
	on_back: ((set_text: (string) -> ()) -> ())?,
	on_close_question: (() -> ())?,
	approval: State<ask.Approval?>?,
	on_approve: ((scope: "once" | "always") -> ())?,
	on_deny: (() -> ())?,
	-- Whether an approval can be given for good ("Always allow"); by
	-- default, yes. An agent that only asks each time hides it.
	can_always: (boolean | () -> boolean)?,

	-- The line above it, what its action does, and putting it away.
	notice: State<Notice?>?,
	on_notice_action: (() -> ())?,
	on_dismiss_notice: (() -> ())?,
	-- Dictating: the track over the controls; the mic starts and stops it,
	-- Escape or the × cancels.
	dictating: State<boolean>?,
	on_dictate: (() -> ())?,
	on_cancel_dictation: (() -> ())?,
	-- Nothing can be written (a chat archived, an agent gone): dimmed, the
	-- notice saying why.
	disabled: (boolean | () -> boolean)?,
	-- What `@`, `/` and `$` offer.
	mentions: any?,
	-- Short of room (the caller knows; else the pill's own width says).
	narrow: (() -> boolean)?,
	-- Bumped, it takes the keys, the caret at the draft's end (a shortcut
	-- that goes to the composer).
	focus: State<number>?,
	-- Its controls' row's id (the window's one composer's, for selectors).
	toolbar_id: string?,
	-- False: files over it show no overlay on the pill (the caller draws
	-- one over a wider area).
	drop_overlay: boolean?,
	-- Bumped, the model picker opens (a shortcut to it).
	open_model: State<number>?,
	-- Set to whether the model picker is open.
	model_open: State<boolean>?,
	-- ds.ModelPicker's: whether the chat has its agent, a pick to ask about
	-- first, whether the catalog is still coming; what the chat holds of its
	-- context.
	model_committed: (() -> boolean)?,
	model_take: ((value: string) -> boolean)?,
	model_looking: (() -> boolean)?,
	model_retry: (() -> ())?,
	model_close: State<number>?,
	context_tokens: (() -> number?)?,
	-- Compaction (ds/compaction): `label` the chip's words while the chat
	-- checks or compacts; `resume` the offer to resume with less context
	-- (`on_resume_compact`, `on_resume_keep`); `plan` a planned or running
	-- compaction (the ring's dot and its card's section), `blocked` why
	-- Compact context can't now, `on_compact` it pressed.
	compaction: {
		label: () -> string?,
		resume: () -> any,
		on_resume_compact: () -> (),
		on_resume_keep: () -> (),
		plan: () -> any,
		blocked: () -> string?,
		on_compact: () -> (),
	}?,
	-- Agent profiles (ds.ProfilePicker's props, but its trigger and
	-- placement): with one applied, the model's chip is two, the profile's
	-- half opening the profiles card. `open_profiles` bumped opens it.
	profiles: { [string]: any }?,
	open_profiles: State<number>?,
	-- Which of `@`, `/`, `$` open their list (an agent's completion
	-- settings), and what an empty list says, by trigger.
	mention_enabled: ((trigger: string) -> boolean)?,
	mention_failed: ((string) -> string?)?,
	mention_token: ((any) -> ())?,
	-- A mention's row taken (after its token lands in the draft): its item
	-- and trigger (a command's or skill's reference, remembered for the
	-- send).
	on_mention: ((item: any, trigger: string, token: string) -> ())?,
	mention_empty: { [string]: string }?,
}

-- Zeron's geometry. The one line: 47 points inside the hairline, the text
-- centred in it. Opened out: the text box (16 above the text, 4 below)
-- between its floor and cap, then the controls' row (2, a 32-point chip
-- zone, 8). Round controls are 28 across, concentric with the pill's
-- corner: on one line (a pill, its corner half its height) centred in it,
-- as far from its sides as from its foot; opened out, as far in from the
-- side and the foot as the pill's corner (inside its hairline) is round
-- less theirs.
local LINE = 22
local ONE_LINE = 47
local TEXT_TOP, TEXT_BOTTOM, TEXT_MIN, TEXT_MAX = 16, 4, 76, 260
local ACTIONS = 42
local HAIRLINES = 2
local CONTROL = 28
local INSET_LINE = (ONE_LINE - CONTROL) / 2
local function open_inset(): number
	return control.nest("$zeron.composer.radius", 1) - CONTROL / 2
end
-- The controls' row opened out: its 42 points, or more where a rounder
-- pill (a template's) sets its controls further in from the foot, so they
-- keep the room over them they have at Zeron's own corner (its controls'
-- top 3 under the row's) rather than rising into the text.
local ACTIONS_OVER = 3
local function actions(): number
	return math.max(ACTIONS, open_inset() + CONTROL + ACTIONS_OVER)
end
-- The draft's own inset either side on one line, and opened out.
local TEXT_SIDE = 8
local TEXT_OPEN_SIDE = 16
-- Opened out, the pill folds back to one line only once the text is this
-- much narrower than the line holds, so a width at the boundary cannot
-- flicker between the two.
local HYSTERESIS = 32
-- gpui's fold (composer.rs `CHIP_LABEL_MIN_WIDTH`, `MIN_COMPACT_INPUT_WIDTH`):
-- on one line, plan's and the access mode's words go below LABELS_LINE (its 720
-- at gpui's 737 pill: this pill carries 660), and the pill opens out rather
-- than leave the draft under LINE_ROOM. Opened out they come back, but for
-- under LABELS_OPEN, where gpui's run under one another. The model takes at
-- most MODEL_SHARE of the pill, its effort giving way first.
local LABELS_LINE, LABELS_OPEN = 645, 375
local LINE_ROOM, LINE_MIN = 200, 490
local MODEL_SHARE = 0.45
-- gpui's `chip_max_width`: 248, and 320 with a profile's half.
local PROFILE_SHARE = MODEL_SHARE * 320 / 248
-- What a chip takes of the name it says.
local PROJECT_MAX, LABEL_MAX, CHIP_MAX = 180, 160, 280
local FOOTER_CHIP = 20
-- The ring's box.
local CHIP_H, CHIP_RADIUS = 24, "$radius.lg"
-- Its words lifted a point, as the footer's other chips' are (ds/dropdown's
-- chip): centred by their capitals, lowercase reads low beside a glyph. (A
-- margin under them of twice that: their box stays centred, a point up.)
local LIFT = "$zeron.space.hair"
-- With cargo or a question over the draft, the draft keeps the strip's
-- `below` over it (the theme's; read as the composer is built) rather than
-- the open text box's own top.
local function head_gap(): number
	local v = ui.theme.get("zeron.composer.chip.below")
	return if type(v) == "number" then v else 12
end
-- A pill narrower than this is short of room: the trays' rows fold their
-- thumbnails and actions.
local NARROW = 480
-- Dictation's limit, and when its clock warns.
local VOICE_LIMIT, VOICE_WARN = 60, 10

local function read<T>(v: T | () -> T): T
	return if type(v) == "function" then (v :: () -> T)() else v :: T
end

-- A table some of whose fields are words: a copy of `t` with each of
-- `words` (field → key) its message's text now. Made where it's read (a
-- list given as a function, read as it's shown), it follows the locale.
local function worded(t: { [string]: any }, words: { [string]: string }): any
	local out = table.clone(t)
	for field, key in words do
		out[field] = locale.s(key)
	end
	return out
end

-- The access modes, as Zeron offers them, with its icons (pickers.rs
-- `permission_icon`). (Made as they're read, in the locale's words.)
local function ACCESS(): { Dropdown.Item }
	return {
	worded({ value = "supervised", icon = "checklist" }, {
		label = "ds.composer.access.supervised",
		description = "ds.composer.access.supervised_hint",
	}),
	worded({ value = "edits", icon = "bot" }, {
		label = "ds.composer.access.edits",
		description = "ds.composer.access.edits_hint",
	}),
	worded({ value = "full", icon = "danger-triangle", ink = "$warning" }, {
		label = "ds.composer.access.full",
		description = "ds.composer.access.full_hint",
	}),
	}
end

-- A choice in a nested menu: an item of a dropdown (its name and icon, a
-- line under its name saying what it is, a muted tag, a warning that it is
-- out of reach).
type Choice = Dropdown.Item

-- The checkouts, as Zeron offers them (made as they're read).
local function CHECKOUTS(): { Choice }
	return {
	worded({ value = "local", icon = "folder" }, {
		label = "ds.composer.checkout.local",
		description = "ds.composer.checkout.local_hint",
	}),
	worded({ value = "worktree", icon = "worktree" }, {
		label = "ds.composer.checkout.worktree",
		description = "ds.composer.checkout.worktree_hint",
	}),
	}
end

-- A list, however it was given (a list, or a state or function of one).
local function list_of(given: any): any
	if type(given) == "function" then
		local call: () -> any = given
		return call()
	elseif type(given) == "userdata" then
		local state: State<any> = given
		return state:get()
	end
	return given or {}
end

-- A chip of where it works, its panel over or under it: a glyph, what it
-- says, a chevron, washed under the pointer and while open.
type Chip = { id: string?, label: any, icon: () -> string, says: () -> string, says_id: string?, warn: (() -> boolean)? }
local function chip_ink(warn: (() -> boolean)?): any
	if not warn then
		return "$text.tertiary"
	end
	return function(): string
		return if warn() then "$warning" else "$text.tertiary"
	end
end
local function lead_glyph(icon: () -> string, size: number): Node
	return ui.presence(icon, function(kind: string): Node
		-- (Its kind named, for tests: "place-lead-worktree".)
		return ui.div { class = `place-lead-{kind}`, Icon { name = kind :: Icon.Name, size = size } }
	end, { shrink = 0, w = size, h = size, items = "center", justify = "center" })
end
local function footer_chip(chip: Chip, m: Dropdown.Menu): Node
	return ui.button {
		id = chip.id,
		label = chip.label,
		h = FOOTER_CHIP,
		max_w = CHIP_MAX,
		px = "$space.2",
		gap = "$zeron.space.tight",
		items = "center",
		min_w = 0,
		rounded = "$radius.md",
		text_size = "$font.size.sm",
		weight = "$font.weight.medium",
		bg = function(): string
			return if m.open:get() then "$hover" else "#00000000"
		end,
		color = chip_ink(chip.warn),
		hover = { bg = "$hover", color = "$text.secondary" },
		active = { bg = "$pressed", color = "$text.secondary" },
		on_click = m.toggle,
		lead_glyph(chip.icon, 12),
		FadeText { chip.says, roll = true, id = chip.says_id, mb = LIFT, min_w = 0, shrink = 1, fade = 16 },
		ui.div { shrink = 0, color = "$text.disabled", aria_hidden = true, Icon { name = "alt-arrow-down", size = 12 } },
	}
end

-- The project a chat works in (or none), and the projects offered.
-- `warn` and `disabled` (with `description` saying why): one out of reach
-- (a project no longer where it was).
export type Project = {
	value: string,
	label: string,
	hue: string?,
	src: string?,
	emoji: string?,
	warn: boolean?,
	disabled: boolean?,
	description: string?,
}
-- What working in no project is, among the projects.
local NO_PROJECT = "none"

-- Whether the device chosen is Cloud (a device row marked `cloud`): a
-- machine of its own, so no checkout to choose.
local function on_cloud(device: State<string?>?, devices: any): boolean
	if not device or not devices then
		return false
	end
	local chosen = device:get()
	for _, d in list_of(devices) :: { Device } do
		if d.value == chosen then
			return d.cloud == true
		end
	end
	return false
end

-- The project, in a new chat's pill, as gpui's: the folder, its name, its
-- list with working in none and making one under it.
local function project_chip(
	project: State<string?>,
	projects: ({ Project } | State<{ Project }> | () -> { Project }),
	on_new_project: (() -> ())?,
	room: () -> number
): Node
	local function choices(): { Choice }
		local out: { Choice } = {}
		for _, p in list_of(projects) :: { Project } do
			local icon: Icon.Name? = if p.hue or p.src or p.emoji then nil else "folder"
			table.insert(out, {
				value = p.value,
				label = p.label,
				hue = p.hue,
				src = p.src,
				emoji = p.emoji,
				icon = icon,
				warn = p.warn,
				disabled = p.disabled,
				description = p.description,
			})
		end
		return out
	end
	local function says(): string
		local v = project:get()
		for _, c in choices() do
			if c.value == v then
				return c.label
			end
		end
		return locale.s("ds.composer.project.none")
	end
	local function warned(): boolean
		local v = project:get()
		for _, c in choices() do
			if c.value == v then
				return c.warn == true
			end
		end
		return false
	end
	return Dropdown {
		value = project,
		items = choices,
		actions = {
			worded({
				icon = "close",
				on_click = function()
					project:set(NO_PROJECT)
				end,
			}, { label = "ds.composer.project.none_action" }),
			worded({ icon = "plus", on_click = on_new_project }, { label = "ds.composer.project.new" }),
		},
		label = locale.t("ds.composer.project.label") :: any,
		search = locale.s("ds.composer.project.search"),
		width = 280,
		trigger = function(m: Dropdown.Menu): Node
			return ui.button {
				label = locale.t("ds.composer.project.label"),
				h = CONTROL,
				px = ui.raw(9, "gpui's project chip"),
				gap = "$zeron.space.tight",
				items = "center",
				min_w = 0,
				rounded = "$radius.lg",
				text_size = "$font.size.sm",
				bg = function(): string
					return if m.open:get() then "$hover" else "#00000000"
				end,
				color = chip_ink(warned),
				hover = { bg = "$hover" },
				active = { bg = "$pressed" },
				focus = control.ring,
				on_click = m.toggle,
				tooltip = locale.s("ds.composer.project.tip"),
				ui.div { shrink = 0, class = "place-lead-folder", Icon { name = "folder", size = 14 } },
				ui.show(function(): boolean
					return room() > 0
				end, FadeText {
					says,
					roll = true,
					id = "composer-place-project",
					mb = LIFT,
					min_w = 0,
					max_w = room :: any,
					shrink = 1,
					fade = 16,
				}),
				ui.div { shrink = 0, color = "$text.tertiary", aria_hidden = true, Icon { name = "alt-arrow-down", size = 12 } },
			}
		end,
	}
end

local function ring(used: number | () -> number, usage: (() -> Usage?)?, side: () -> string, compaction: any?, stats: (() -> any)?): Node
	local function known(): number?
		if usage then
			return context_usage.fraction(usage())
		end
		return math.clamp(if type(used) == "function" then used() else used, 0, 1)
	end
	local function fraction(): number
		return known() or 0
	end
	local open = ui.use_state(false)
	local function plan(): any
		return if compaction then compaction.plan() or nil else nil
	end
	-- A compaction planned: the accent, its dot; running, the busy ink.
	local function ink(): string
		local p = plan()
		if p then
			return if p.busy then "$zeron.status.working" else "$accent"
		end
		local f = known()
		return if f then context_usage.ink(f) else "$text.tertiary"
	end
	local chip = ui.row {
		id = "composer-context",
		h = CHIP_H,
		px = "$zeron.space.tight",
		gap = ui.raw(5, "gpui's ring to its percent"),
		items = "center",
		shrink = 0,
		rounded = CHIP_RADIUS,
		cursor = "pointer",
		selectable = false,
		role = "button",
		expanded = function(): boolean
			return open:get()
		end,
		focus = control.ring,
		text_size = "$font.size.xs",
		color = ink,
		bg = function(): string
			return if open:get() then "$hover" else "#00000000"
		end,
		hover = { bg = "$hover" },
		focusable = usage ~= nil,
		label = locale.t("ds.composer.context_used"),
		on_click = if usage
			then function()
				open:set(not open:peek())
			end
			else nil,
		relative = true,
		context_usage.Ring {
			fraction = fraction,
			footer = true,
			ink = function(f: number): string
				local p = plan()
				return if p then ink() else context_usage.ink(f)
			end,
		},
		ui.show(function(): boolean
			return plan() and true or false
		end, ui.div {
			id = "composer-context-dot",
			absolute = true,
			left = ui.raw(19, "gpui's plan dot"),
			top = 1,
			w = 5,
			h = 5,
			rounded = "$radius.full",
			bg = ink :: any,
			aria_hidden = true,
		}),
		ui.text {
			function(): string
				local f = known()
				return if f
					then locale.s("ds.composer.context.percent", { n = string.format("%.0f", f * 100) })
					else locale.s("ds.session_stats.missing")
			end,
			id = "composer-context-percent",
			tabular = true,
		},
	}
	if not usage then
		return chip
	end
	-- (A placement is read as it's built: one following the composer is
	-- built again as it changes sides.)
	return ui.presence(side, function(at: string): Node
		return ui.popover(Menu.panel {
			class = "composer-context-panel",
			open = open,
			anchor = chip,
			autofocus = false,
			w = 248,
			placement = { side = at, align = "end", offset = 6 },
			build = function(): Node
				return context_usage.Body {
					usage = usage,
					more = if compaction
						then Compaction.CardPart {
							plan = compaction.plan,
							blocked = compaction.blocked,
							resume = compaction.resume,
							stats = stats,
							on_compact = function()
								open:set(false)
								compaction.on_compact()
							end,
						}
						else nil,
				}
			end,
		})
	end)
end

local function terminal_button(run: () -> (), open: (() -> boolean)?, keys: (() -> string)?): Node
	local function is_open(): boolean
		return open ~= nil and open()
	end
	return ui.button {
		id = "composer-terminal",
		label = locale.t("ds.composer.terminal"),
		checked = is_open,
		tooltip = Tooltip { locale.t("ds.composer.terminal") },
		tooltip_placement = { side = "top", align = "end", offset = 6 },
		h = 20,
		px = "$space.1",
		gap = "$zeron.space.tight",
		shrink = 0,
		items = "center",
		rounded = "$radius.md",
		text_size = "$font.size.sm",
		weight = "$font.weight.medium",
		bg = function(): string
			return if is_open() then "$zeron.composer.accent_wash" else "#00000000"
		end,
		color = function(): string
			return if is_open() then "$accent" else "$text.tertiary"
		end,
		hover = { bg = "$hover", color = "$text.primary" },
		active = { bg = "$pressed", color = "$text.primary" },
		on_click = run,
		ui.div { shrink = 0, Icon { name = "terminal", size = 12 } },
		ui.show(is_open, ui.presence(keys or function(): string
			return "cmd+j"
		end, function(combo: string): Node
			return Kbd { keys = combo, tone = "quiet" }
		end)),
	}
end

-- An existing chat's footer, as Zeron's: where it works (a word, opening
-- nothing), the live branch (a chip opening the project's refs), and how a
-- worktree stands against its base.
local REFS_SHOWN = 300
local function refs_cap_words(shown: number, total: number): string
	return locale.s("ds.composer.branch.showing", { shown = shown, total = total })
end

local function session_footer(fixed: () -> Fixed?, menu: any): Node
	local function now(): any
		local f: any = fixed() or {}
		if f.label then
			return f
		end
		-- (A caller that names only its project: that, in its folder's place.)
		local out = table.clone(f)
		out.label = if type(f.project) == "string" then f.project else nil
		out.icon = if f.worktree then "worktree" else "folder"
		return out
	end
	local function branch(): string
		return now().name or locale.s("ds.composer.branch.no_ref")
	end
	local function branch_label(): string
		return locale.s("ds.composer.branch.fixed", { name = branch() })
	end
	local function branch_glyph(): string
		return "git-branch"
	end
	local chip: Node
	if menu then
		-- (What's lit as it opens, this composer's and no chat's: only a pick moves one.)
		local lit = ui.use_state(nil :: string?)
		chip = Dropdown {
			value = lit,
			on_choose = function(name: string): boolean?
				local went = menu.on_pick(name)
				if went == false then
					lit:set(branch())
				end
				return went
			end,
			extra = if menu.note
				then ui.text {
					function(): string
						return menu.note:get() or ""
					end,
					wrap = true,
					px = "$space.2",
					py = "$space.1",
					text_size = "$font.size.xs",
					color = "$text.secondary",
				}
				else nil,
			extra_when = if menu.note
				then function(): boolean
					return menu.note:get() ~= nil
				end
				else nil,
			label = locale.t("ds.composer.branch.label") :: any,
			search = menu.search,
			items = menu.items,
			empty = menu.empty,
			on_open = function()
				lit:set(branch())
				if menu.on_open then
					menu.on_open()
				end
			end,
			width = 320,
			cap = REFS_SHOWN,
			cap_words = refs_cap_words,
			trigger = function(m: Dropdown.Menu): Node
				return footer_chip({ id = "composer-session-branch", label = branch_label, icon = branch_glyph, says = branch, says_id = "composer-place-branch" }, m)
			end,
		}
	else
		chip = ui.row {
			label = branch_label,
			h = FOOTER_CHIP,
			px = "$space.2",
			gap = "$zeron.space.tight",
			items = "center",
			min_w = 0,
			text_size = "$font.size.sm",
			weight = "$font.weight.medium",
			color = "$text.tertiary",
			ui.div { shrink = 0, Icon { name = "git-branch", size = 12 } },
			FadeText { branch, roll = true, id = "composer-place-branch", mb = LIFT, min_w = 0, shrink = 1, fade = 16 },
		}
	end
	return ui.row {
		id = "composer-session-footer",
		h = FOOTER_CHIP,
		gap = "$space.1",
		items = "center",
		min_w = 0,
		text_size = "$font.size.sm",
		weight = "$font.weight.medium",
		ui.row {
			h = FOOTER_CHIP,
			px = "$space.2",
			gap = "$zeron.space.tight",
			items = "center",
			min_w = 0,
			color = "$text.tertiary",
			opacity = 0.75,
			ui.presence(function(): string
				return now().icon or "folder"
			end, function(kind: string): Node
				return ui.div { class = `place-lead-{kind}`, Icon { name = kind :: Icon.Name, size = 12 } }
			end, { shrink = 0, w = 12, h = 12, items = "center", justify = "center", id = "composer-place-lead" }),
			FadeText { function(): string
				return now().label or ""
			end, id = "composer-place-label", mb = LIFT, min_w = 0, max_w = LABEL_MAX, shrink = 1, fade = 16 },
			ui.show(function(): boolean
				return now().removed == true
			end, ui.text { locale.t("ds.composer.footer_removed"), shrink = 0, color = "$text.disabled" }),
		},
		ui.show(function(): boolean
			return now().project ~= false
		end, chip),
		ui.presence(function(): string?
			local b = now().badge
			return if b then b.kind else nil
		end, function(kind: string): Node
			local b = now().badge or {}
			return ui.row {
				id = "composer-work-tree",
				h = 20,
				px = 6,
				gap = 3,
				shrink = 0,
				items = "center",
				rounded = 6,
				bg = "$hover",
				color = "$text.tertiary",
				text_size = "$font.size.xs",
				weight = "$font.weight.medium",
				tooltip = b.tip,
				label = b.tip,
				Icon { name = if kind == "ahead" then "arrow-up" else "check", size = 11 },
				if kind == "ahead" then ui.text { tostring(b.count or 0), font = "$font.mono" } else nil,
				if kind == "merged" and (b.dirty or 0) > 0 then ui.div { w = 5, h = 5, rounded = "$radius.full", bg = "$warning" } else nil,
			}
		end),
	}
end

-- The voice, while dictating: bars across the track, each breathing on
-- its own, from the time alone (no script per frame). params[0]: the ink.
local VOICE = ui.shader([[
fn fill(input: FillInput) -> vec4<f32> {
 let ink = input.params[0];
 let n = floor(input.size.x / 4.0);
 let x = input.p.x / input.size.x * n;
 let i = floor(x);
 let fx = fract(x);
 if (fx < 0.2 || fx > 0.8) { return vec4(0.0); }
 let t = input.time;
 let a1 = 0.5 + 0.5 * sin(i * 1.71 + t * 7.3);
 let a2 = 0.5 + 0.5 * sin(i * 0.37 - t * 2.9 + 1.3);
 let env = 0.55 + 0.45 * sin(t * 1.9 + i * 0.11);
 let h = 0.12 + 0.88 * a1 * a2 * env;
 let y = abs(input.p.y / input.size.y - 0.5) * 2.0;
 let a = clamp((h - y) * input.size.y * 0.5 * input.scale, 0.0, 1.0);
 return vec4(ink.rgb, ink.a * a);
}
]], { fps = 30 })

-- The notice line: its words on the draft's own left edge, in the tone's
-- ink (a warning amber, a failure red, the rest the secondary ink), a
-- spinner before them while something is on its way, an action, and the ×
-- that puts it away. No dot, no badge: the words say it.
-- One line while there is a notice, following it in place as it changes
-- (its words, its tone, a spinner, an action): a notice that moves on
-- ("Uploading 2 of 5", "3 of 5") never fades out and in again. Going, it
-- keeps the last it said as it fades.
local function notice_line(notice: State<Notice?>, on_action: (() -> ())?, on_dismiss: (() -> ())?): Node
	return ui.presence(function(): string?
		return if notice:get() then "notice" else nil
	end, function(): Node
		local last = notice:peek() :: Notice
		local function now(): Notice
			local n = notice:get()
			if n then
				last = n
			end
			return last
		end
		-- A failure put away by a click: a card, Zeron's notice (notice.rs), its
		-- kind, its words and a way to copy them.
		local function failure(): boolean
			local n = now()
			return n.dismissable == true and n.action == nil and (n.tone == "danger" or n.tone == "warning")
		end
		local function ink(): string
			return `$zeron.tone.{if now().tone == "warning" then "warning" else "danger"}.ink`
		end
		local card = ui.column {
			id = "composer-failure",
			role = "alert",
			mx = ui.raw(4, "gpui's failure chip"),
			mt = ui.raw(6, "gpui's failure chip"),
			mb = ui.raw(6, "gpui's failure chip"),
			px = ui.raw(12, "gpui's failure chip"),
			py = ui.raw(8, "gpui's failure chip"),
			gap = ui.raw(6, "gpui's failure chip"),
			w_full = true,
			rounded = ui.raw(12, "gpui's failure chip"),
			relative = true,
			border = 1,
			border_color = ink :: any,
			cursor = "pointer",
			on_click = on_dismiss,
			ui.div {
				absolute = true,
				left = 0,
				right = 0,
				top = 0,
				bottom = 0,
				rounded = ui.raw(12, "gpui's failure chip"),
				bg = ink :: any,
				opacity = ui.raw(0.05, "Zeron's notice wash"),
				pointer_events = "none",
			},
			ui.row {
				gap = ui.raw(8, "gpui's failure chip"),
				items = "center",
				color = ink :: any,
				Icon { name = "danger-triangle", size = 14 },
				ui.text {
					function(): string
						return locale.s(if now().tone == "warning" then "ds.composer.notice_warning" else "ds.composer.notice_error")
					end,
					grow = 1,
					text_size = "$font.size.sm",
					weight = "$font.weight.medium",
				},
				IconButton {
					icon = "copy",
					label = locale.t("ds.composer.notice_copy"),
					tooltip = locale.s("ds.composer.notice_copy"),
					size = "sm",
					tone = "quiet",
					on_click = function()
						ui.clipboard.write(now().text)
					end,
				},
			},
			ui.text {
				function(): string
					return now().text
				end,
				wrap = true,
				text_size = "$font.size.sm",
				line_height = "$font.line.sm",
				color = ink :: any,
			},
		}
		return ui.show(failure, card, ui.row {
			h = 24,
			pl = "$zeron.composer.chip.inset",
			pr = "$space.2",
			gap = "$space.2",
			items = "center",
			role = "status",
			ui.show(function(): boolean
				return now().spinner == true
			end, status.StatusIcon { kind = "working", size = 14 }),
			FadeText {
				function(): string
					return now().text
				end,
				grow = 1,
				text_size = "$font.size.sm",
				color = function(): string
					local tone = now().tone or "neutral"
					return if tone == "danger" or tone == "warning" then `$zeron.tone.{tone}.ink` else "$text.secondary"
				end,
			},
			ui.show(
				function(): boolean
					return now().action ~= nil
				end,
				Button {
					function(): string
						return now().action or ""
					end,
					size = "sm",
					tone = "ghost",
					h = 22,
					px = "$space.2",
					text_size = "$font.size.xs",
					on_click = on_action,
				}
			),
			ui.show(function(): boolean
				return now().dismissable == true
			end, IconButton {
				icon = "close",
				label = locale.t("ds.composer.dismiss"),
				tooltip = locale.s("ds.composer.dismiss"),
				size = "sm",
				tone = "quiet",
				on_click = on_dismiss,
			}),
		})
	end, {
		w_full = true,
		enter = { opacity = { 0, 1 }, y = { 4, 0 }, duration = 180, ease = "outCubic" },
		exit = { opacity = 0, duration = 120 },
	})
end

-- Composers' lightbox groups, each its own.
local lightboxes = 0

return ui.component(function(props: Props): Node
	-- Opened out, the controls' inset: from the theme's corner, as this one
	-- is built.
	local INSET_OPEN = open_inset()
	local value = props.value
	local running = props.running
	local field = props.field or ui.ref()
	local pill = ui.ref()

	-- What it is laid out from, as layout last measured it (0: not yet):
	-- written as each node is laid out (`on_resize`), so what reads them
	-- follows, and kept under `keep` so a composer made again starts from
	-- them rather than from nothing.
	local function kept<T>(key: string, initial: T): State<T>
		return if props.keep then ui.state(`composer:{props.keep}:{key}`, initial) else ui.use_state(initial)
	end
	local pill_w = kept("pill_w", 0)
	local line_w = kept("line_w", 0)
	local wrapped_h = kept("wrapped_h", 0)
	local head_h_seen = kept("head_h", 0)
	local trailing_seen = kept("trailing_w", 0)
	local leading_seen = kept("leading_w", 0)
	local access_seen = kept("access_w", 0)
	local actions_seen = kept("actions_w", 0)
	-- gpui's compact capacity: the draft's room as it last stood on one line and
	-- the pill's width then. Opened out, the words and the model take more, so
	-- what is measured there can't say whether the draft fits back on a line.
	local line_room = kept("line_room", 0)
	local line_room_w = kept("line_room_w", 0)
	local function measure(into: State<number>, axis: "w" | "h"): (size: Size, _el: Element) -> ()
		return function(size: Size, _el: Element)
			local v = if axis == "w" then size.w else size.h
			if math.abs(into:peek() - v) >= 0.5 then
				into:set(v)
			end
		end
	end

	local function has_text(): boolean
		return string.find(value:get(), "%S") ~= nil
	end
	local function available(): boolean
		return props.plan ~= nil and (props.plan_available == nil or props.plan_available())
	end
	local function disabled(): boolean
		return read(props.disabled) == true
	end
	local function dictating(): boolean
		return props.dictating ~= nil and props.dictating:get()
	end
	local function question(): ask.Question?
		return if props.question then props.question:get() else nil
	end
	local function approval(): ask.Approval?
		return if props.approval then props.approval:get() else nil
	end
	local function asking(): boolean
		return question() ~= nil or approval() ~= nil
	end
	local function has_cargo(): boolean
		return props.cargo ~= nil and #props.cargo:get() > 0
	end
	local function picked(): number
		local n = 0
		if props.chosen then
			for _, on in props.chosen:get() do
				if on then
					n += 1
				end
			end
		end
		return n
	end
	-- A question can go on once something is picked or typed.
	local function can_answer(): boolean
		return picked() > 0 or has_text()
	end
	local function answer()
		local q = question()
		if not q or not can_answer() then
			return
		end
		local values: { string } = {}
		if props.chosen then
			for _, o in q.options do
				if props.chosen:peek()[o.value] then
					table.insert(values, o.value)
				end
			end
		end
		local typed = value:peek()
		if props.on_answer then
			props.on_answer({
				values = values,
				text = typed,
				keep = function()
					if value:peek() == "" then
						value:set(typed)
					end
				end,
			})
		end
		value:set("")
	end
	local function go_back()
		if props.on_back then
			props.on_back(function(text: string)
				value:set(text)
			end)
		end
	end
	local function send(opts: { inverted: boolean? }?)
		local text = value:peek()
		if asking() then
			answer()
			return
		end
		if not string.find(text, "%S") and not has_cargo() then
			return
		end
		-- (`false`: it took the draft itself, or left it because nothing went.)
		if props.on_send and props.on_send(text, opts) == false then
			return
		end
		value:set("")
	end
	local function stopping(): boolean
		return running ~= nil and running:get() and not has_text() and not has_cargo() and not asking()
	end

	local measured_text: () -> string
	-- Mentions at the caret.
	local m = mentions.use {
		field = field,
		value = value,
		sources = props.mentions or {},
		enabled = props.mention_enabled,
		empty = props.mention_empty,
		on_choose = props.on_mention,
		failed = props.mention_failed,
		on_token = props.mention_token,
		in_view = function(): boolean
			return (pill_w:get() or 0) > 0
		end,
	}
	-- Asked to: the keys, the caret at the end.
	if props.focus then
		local focus = props.focus
		ui.effect(function()
			if focus:get() <= 0 then
				return
			end
			local el = field:peek()
			if el then
				el:focus()
				local n = #value:peek()
				el:set_selection_bytes(n, n)
			end
		end)
	end

	-- Short of room: the caller's word, else the pill's width.
	local narrow = props.narrow or function(): boolean
		local w = pill_w:get()
		return (if w > 0 then w else 1000) < NARROW
	end

	-- One line or opened out. On one line the text has the pill less the
	-- leading control and the trailing ones; opened out, it folds back only
	-- once comfortably short again. Anything over the draft opens it out.
	-- The controls at the right, as measured; until they are (a composer
	-- just made, a reload's), their usual two (the microphone and Send),
	-- so the text is laid out where it rests from the first frame rather
	-- than easing in from the full width.
	local function plan_on(): boolean
		return props.plan ~= nil and props.plan:get() and available()
	end
	local function has_limit(): boolean
		return props.limit ~= nil and props.limit() ~= nil
	end
	local function trailing_w(): number
		local w = trailing_seen:get()
		return if w > 0 then w else CONTROL * 2 + 8
	end
	local function expanded(): boolean
		return read(props.expanded) == true
	end
	-- The controls at the left: + and, where there is one, the plan switch.
	local function leading_w(): number
		local w = leading_seen:get()
		return if w > 0 then w else CONTROL
	end
	local was_open = expanded()
	local open = ui.computed(function(): boolean
		-- Dictating, the words land in the draft over the track.
		if expanded() or has_cargo() or asking() or dictating() or plan_on() or has_limit() then
			return true
		end
		local text = value:get()
		local width = pill_w:get()
		local need = line_w:get()
		if width <= 0 then
			return false
		end
		if width < LINE_MIN then
			was_open = true
			return true
		end
		local at = line_room_w:get()
		local room = if at > 0
			then line_room:get() + width - at
			else width - HAIRLINES - (INSET_LINE + leading_w() + TEXT_SIDE) - (TEXT_SIDE + trailing_w() + INSET_LINE)
		local want
		if string.find(text, "\n") then
			want = true
		elseif was_open then
			want = need > room - HYSTERESIS
		else
			want = need > room
		end
		was_open = want
		return want
	end)

	ui.effect(function()
		local _ = pill_w:get(), leading_seen:get(), trailing_seen:get()
		-- (Next tick: each side's measure lands on its own, and half of them
		-- from opened out would read as far less room than there is.)
		ui.after(0, function()
			local width, lead, tail = pill_w:peek(), leading_seen:peek(), trailing_seen:peek()
			if open:peek() or width <= 0 or lead <= 0 or tail <= 0 then
				return
			end
			line_room:set(width - HAIRLINES - (INSET_LINE + lead + TEXT_SIDE) - (TEXT_SIDE + tail + INSET_LINE))
			line_room_w:set(width)
		end)
	end)
	local labelled = ui.computed(function(): boolean
		local width = pill_w:get()
		return width <= 0 or (width >= LABELS_OPEN and (open:get() or width >= LABELS_LINE))
	end)
	-- What is over the draft: nothing, or its measured height.
	local head_h = ui.computed(function(): number
		if not has_cargo() and not asking() and not has_limit() then
			return 0
		end
		return head_h_seen:get()
	end)
	-- The text box opened out: the wrapped text with its insets, between
	-- the floor and the cap.
	local box = ui.computed(function(): number
		local h = math.max(wrapped_h:get(), LINE)
		return math.clamp(h + TEXT_TOP + TEXT_BOTTOM, TEXT_MIN, TEXT_MAX)
	end)
	local HEAD_GAP = head_gap()
	local height = ui.computed(function(): number
		if open:get() then
			local head = head_h:get()
			-- With something over it, the draft's box starts under that
			-- with the strip's own gap rather than the pill's top.
			return head + (if head > 0 then box:get() - TEXT_TOP + HEAD_GAP else box:get()) + actions() + HAIRLINES
		end
		return ONE_LINE + HAIRLINES
	end)
	-- Where the text sits: beside the controls on one line, over them
	-- opened out, under whatever is over it.
	local text_left = ui.computed(function(): number
		return if open:get() then TEXT_OPEN_SIDE else INSET_LINE + leading_w() + TEXT_SIDE
	end)
	local text_right = ui.computed(function(): number
		return if open:get() then TEXT_OPEN_SIDE else TEXT_SIDE + trailing_w() + INSET_LINE
	end)
	local text_top = ui.computed(function(): number
		if not open:get() then
			return (ONE_LINE - LINE) / 2
		end
		local head = head_h:get()
		return if head > 0 then head + HEAD_GAP else TEXT_TOP
	end)
	local text_h = ui.computed(function(): number
		return if open:get() then box:get() - TEXT_TOP - TEXT_BOTTOM else LINE
	end)
	local EASE: TweenOptions = { duration = 180, ease = "out(3)" }
	-- Each of the pill's measures as it shows: eased (EASE) as it grows or
	-- shrinks in place; at once for a moment after `snap` moves (what
	-- settles its size, measured, lands over a frame or two), and with
	-- motion reduced.
	local SNAP_MS = 400
	local snap_seen = if props.snap then props.snap:peek() else nil
	local snap_until = 0
	local function at_once(): boolean
		if props.snap then
			local n = props.snap:get()
			if n ~= snap_seen then
				snap_seen = n
				snap_until = ui.now() + SNAP_MS
			end
		end
		return motion.reduced_now() or ui.now() < snap_until
	end
	local function eased(src: ReadOnlyState<number>): State<number>
		local value = ui.use_state(src:peek())
		local run: any = nil
		ui.effect(function()
			local v = src:get()
			local now = at_once()
			if run then
				run:cancel()
				run = nil
			end
			if now or math.abs(value:peek() - v) < 0.01 then
				value:set(v)
				return
			end
			run = anime.animate(value, { value = v, duration = EASE.duration, ease = EASE.ease })
		end)
		return value
	end
	-- (One on a line, another opened out.)
	local function by_open(line: number, opened: number): State<number>
		return eased(ui.computed(function(): number
			return if open:get() then opened else line
		end))
	end

	-- The paperclip. Dictating, it is the way out.
	local attach = ui.show(dictating, Button {
		tone = "secondary",
		shape = "pill",
		size = "sm",
		icon = true,
		label = locale.t("ds.composer.cancel_dictation"),
		on_click = props.on_cancel_dictation,
		Icon { name = "close", size = 16 },
	}, Button {
		tone = "ghost",
		shape = "pill",
		size = "sm",
		icon = true,
		label = locale.t("ds.composer.attach"),
		disabled = disabled,
		on_click = props.on_attach,
		color = "$text.secondary",
		Icon { name = "paperclip", size = 18 },
	})
	-- The plan switch beside the paperclip: its word when there's room, the accent as it
	-- turns on over 300 ms.
	local function plan_chip(): Node
		local plan = props.plan :: State<boolean>
		local on = ui.computed(function(): boolean
			return plan:get()
		end)
		local PLAN_FLIP = { duration = 300, ease = motion.EASE }
		local fill = ui.tween(ui.computed(function(): number
			return if plan:get() then 0.12 else 0
		end), PLAN_FLIP)
		local function tip(): string
			return locale.s(if props.plan_asks and props.plan_asks() then "ds.composer.plan.tip_asks" else "ds.composer.plan.tip")
		end
		return ui.button {
			id = "composer-plan",
			role = "switch",
			checked = on,
			label = locale.t("ds.composer.plan.mode"),
			tooltip = Tooltip { tip },
			h = CONTROL,
			px = ui.raw(9, "gpui's plan chip"),
			gap = ui.raw(6, "gpui's plan chip"),
			items = "center",
			shrink = 0,
			relative = true,
			rounded = ui.raw(8, "gpui's plan chip"),
			bg = "#00000000",
			color = ui.tween(on, { from = "$text.secondary", to = "$accent", duration = PLAN_FLIP.duration, ease = PLAN_FLIP.ease }),
			hover = { bg = "$hover" },
			focus = control.ring,
			on_click = function()
				plan:toggle()
				local el = field:peek()
				if el then
					el:focus()
				end
			end,
			ui.div {
				absolute = true,
				left = 0,
				right = 0,
				top = 0,
				bottom = 0,
				rounded = ui.raw(8, "gpui's plan chip"),
				bg = "$accent",
				opacity = fill,
				pointer_events = "none",
			},
			ui.div {
				relative = true,
				w = 22,
				h = 13,
				shrink = 0,
				rounded = "$radius.full",
				bg = ui.tween(on, { from = "$zeron.switch.track", to = "$accent", duration = PLAN_FLIP.duration, ease = PLAN_FLIP.ease }),
				ui.div {
					absolute = true,
					top = 2,
					left = ui.tween(ui.computed(function(): number
						return if plan:get() then 11 else 2
					end), PLAN_FLIP),
					w = 9,
					h = 9,
					rounded = "$radius.full",
					bg = "$zeron.composer.plan_knob",
					shadow = "$zeron.switch.knob_lift",
				},
			},
			ui.show(function(): boolean
				return labelled:get()
			end, ui.text { locale.t("ds.composer.plan.short"), relative = true, shrink = 0, text_size = "$font.size.sm" }),
		}
	end
	-- Up from the footer: the composer sits at the window's foot, so the
	-- menu opens (and opens out, browsing) upward from a still bottom edge,
	-- never flipping across the chip as it grows. With room below
	-- (`menus_below`), down from it, its top edge still. (A placement is read
	-- as the picker is built: one following the composer is built again as
	-- it changes sides, its chip the same either way.)
	local function side(): string
		local below = props.menus_below
		local on = if type(below) == "function" then below() else below == true
		return if on then "bottom" else "top"
	end
	-- A profile applied: the chip's profile half shows (gpui's split chip).
	local profiles = props.profiles
	local function applied(): any?
		return if profiles then profiles.applied() else nil
	end
	local function profiled(): boolean
		return applied() ~= nil
	end
	-- The model's chip at most: MODEL_SHARE of the pill, and on one line no more
	-- than leaves the draft its LINE_ROOM; opened out, no more than the row
	-- holds. Under that its words fade, the effort's first.
	local model_cap = ui.computed(function(): number
		local width = pill_w:get()
		if width <= 0 then
			return MODEL_SHARE * 1000
		end
		local others = leading_seen:get() + access_seen:get() + actions_seen:get() + 8
		local room = if open:get()
			then width - HAIRLINES - INSET_OPEN * 2 - others - 4
			else width - HAIRLINES - INSET_LINE * 2 - TEXT_SIDE * 2 - others - LINE_ROOM
		local share = if profiled() then PROFILE_SHARE else MODEL_SHARE
		return math.max(CONTROL, math.min(share * width, room))
	end)

	-- Closed, the keys go back to the draft, as Zeron's picker gives them back;
	-- closed by a press outside, they stay where it landed (Zeron's `dismiss`).
	local picker_open = props.model_open or ui.use_state(false)
	local picker_outside = ui.use_state(false)
	local picker_was = picker_open:peek()
	ui.effect(function()
		local now = picker_open:get()
		if picker_was and not now then
			ui.after(0, function()
				local el = field:peek()
				if el and not picker_outside:peek() then
					el:focus()
				end
			end)
		end
		picker_was = now
	end)
	-- The profiles card: its keys back to the draft as it closes, as the
	-- model picker's.
	local profile_open = ui.use_state(false)
	local profile_outside = ui.use_state(false)
	local profile_was = false
	ui.effect(function()
		local now = profile_open:get()
		if profile_was and not now then
			ui.after(0, function()
				local el = field:peek()
				if el and not profile_outside:peek() and not picker_open:peek() then
					el:focus()
				end
			end)
		end
		profile_was = now
	end)
	local open_profiles = props.open_profiles or ui.use_state(0)
	local model_close = props.model_close or ui.use_state(0)
	-- The full picker's Profiles tab: the profiles card in its place.
	local function to_profiles()
		model_close:set(model_close:peek() + 1)
		ui.after(0, function()
			open_profiles:set(open_profiles:peek() + 1)
		end)
	end
	-- The chip's profile half (gpui's `picker-model-profile`): its brand's mark,
	-- its name in bold held to NAME_MAX, the warning's dot when drifted.
	local function profile_half(menu: Dropdown.Menu): Node
		local function name(): string
			local a = applied()
			return if a then a.name else ""
		end
		return ui.row {
			shrink = 0,
			items = "center",
			ui.button {
				id = "picker-model-profile",
				label = function(): string
					return locale.s("ds.profiles.change", { name = name() })
				end,
				expanded = function(): boolean
					return menu.open:get()
				end,
				h = 32,
				px = "$zeron.space.tight",
				gap = "$zeron.space.tight",
				items = "center",
				rounded = "$radius.lg",
				text_size = "$font.size.sm",
				bg = function(): string
					return if menu.open:get() then "$hover" else "#00000000"
				end,
				hover = { bg = "$hover" },
				active = { bg = "$pressed" },
				on_click = menu.toggle,
				-- (The chat's own agent, as gpui's chip icon; the profile's
				-- until the catalog lists that.)
				ui.presence(function(): string
					local g = profiles and profiles.agent and profiles.agent()
					if g then
						return `{g.id}\0{g.mark or ""}\0{g.icon or ""}\0{g.label or g.id}`
					end
					local a = applied()
					return if a then `{a.harness}\0{a.mark or ""}\0{a.icon or ""}\0{a.name}` else ""
				end, function(key: string): Node
					if key == "" then
						return ui.div {}
					end
					local id, mark, icon, label = string.match(key, "^([^%z]*)%z([^%z]*)%z([^%z]*)%z(.*)$")
					return ui.div {
						shrink = 0,
						class = `profile-half-mark-{(string.gsub(id or "", "[^%w]", "-"))}`,
						AgentMark { label = label or "", mark = if mark ~= "" then mark else nil, icon = if icon ~= "" then icon else nil, seed = id, size = 16, bare = true },
					}
				end, { shrink = 0 }),
				ui.text {
					name,
					mb = LIFT,
					max_w = profile_look.NAME_MAX,
					truncate = true,
					shrink = 0,
					weight = "$font.weight.semibold",
					color = "$text.primary",
				},
				ui.show(function(): boolean
					local a = applied()
					return a ~= nil and a.edited
				end, ui.div { id = "picker-model-profile-drift", w = 6, h = 6, shrink = 0, rounded = "$radius.full", bg = "$warning" }),
			},
			-- The hairline between the halves (gpui's 1×12).
			ui.div { w = 1, h = 12, shrink = 0, bg = "$border" },
		}
	end
	local model = ui.presence(side, function(at: string): Node
		local picker = ModelPicker {
			model = props.model,
			effort = props.effort,
			fast = props.fast,
			options = props.options,
			harnesses = props.harnesses :: any,
			models = props.models :: any,
			accounts = props.accounts,
			on_account = props.on_account,
			on_accounts_open = props.on_accounts_open,
			placement = { side = at, align = "end", offset = 6 },
			open_with = props.open_model,
			close_with = model_close,
			showing = picker_open,
			outside = picker_outside,
			committed = props.model_committed,
			take = props.model_take,
			looking = props.model_looking,
			on_retry = props.model_retry,
			context_tokens = props.context_tokens,
			tone = "model",
			mark_when = if profiles
				then function(): boolean
					return not profiled()
				end
				else nil,
			on_profiles = if profiles then to_profiles else nil,
			profile_dot = if profiles
				then function(): string?
					local a = applied()
					return if a == nil then nil elseif a.edited then "warning" else "accent"
				end
				else nil,
		}
		if not profiles then
			return picker
		end
		local p = profiles
		return ProfilePicker {
			list = p.list,
			applied = p.applied,
			can_save = p.can_save,
			switching = p.switching,
			agents = p.agents,
			models = p.models,
			apply = p.apply,
			save_drift = p.save_drift,
			current = p.current,
			save = p.save,
			more = p.more,
			on_open = p.on_open,
			open_with = open_profiles,
			close_with = p.close_with,
			showing = profile_open,
			outside = profile_outside,
			placement = { side = at, align = "end", offset = 6 },
			trigger = function(menu: Dropdown.Menu): Node
				return ui.row {
					items = "center",
					min_w = 0,
					shrink = 1,
					ui.show(profiled, profile_half(menu)),
					picker,
				}
			end,
		}
	end, { min_w = 0, shrink = 1 })

	-- The access mode: its icon and name; with plan mode on, "Plan" after it
	-- and the icon in the accent (all that tells of it, folded).
	local access: Node? = nil
	if props.access then
		local mode = props.access
		local function access_tip(): string?
			if labelled:get() then
				return nil
			end
			for _, item in ACCESS() do
				if item.value == mode:get() then
					return item.label
				end
			end
			return nil
		end
		access = Dropdown {
			fold = function(): number
				return if labelled:get() then 0 else 2
			end,
			tip = access_tip,
			value = props.access,
			accent = function(): boolean
				return (props.access :: State<string?>):get() == "full"
			end,
			accent_ink = "$warning",
			items = function(): { Dropdown.Item }
				local offered = if props.access_offered then props.access_offered() else nil
				if not offered then
					return ACCESS()
				end
				local out: { Dropdown.Item } = {}
				for _, item in ACCESS() do
					if table.find(offered, item.value) then
						table.insert(out, item)
					end
				end
				return out
			end,
			tone = "access",
			label = locale.t("ds.composer.access.label") :: any,
			search = false,
			width = 300,
			extra = if props.access_note
				then ui.text {
					function(): string
						return (props.access_note :: () -> string?)() or ""
					end,
					wrap = true,
					px = "$space.2",
					py = "$space.1",
					text_size = "$font.size.xs",
					color = "$text.tertiary",
				}
				else nil,
			extra_when = if props.access_note
				then function(): boolean
					return (props.access_note :: () -> string?)() ~= nil
				end
				else nil,
			placement = { side = "bottom", align = "end", offset = 6 },
		}
	end
	local mic = if props.on_dictate == nil then nil else ui.show(dictating, Button {
		tone = "accent",
		shape = "pill",
		size = "sm",
		icon = true,
		label = locale.t("ds.composer.stop_dictating"),
		on_click = props.on_dictate,
		ui.div { w = 10, h = 10, rounded = ui.raw(3, "the stop glyph's corner"), bg = "$accent.text" },
	}, Button {
		tone = "ghost",
		shape = "pill",
		size = "sm",
		icon = true,
		label = locale.t("ds.composer.dictate"),
		disabled = disabled,
		on_click = props.on_dictate,
		Icon { name = "microphone", size = 18 },
	})
	local function send_blocked(): string?
		return if props.send_block then props.send_block() else nil
	end
	local send_button = Button {
		shape = "pill",
		size = "sm",
		icon = true,
		label = function()
			return locale.s(if running and running:get() then "ds.composer.queue_message" else "ds.composer.send_message")
		end,
		tooltip = Tooltip {
			function(): string
				return send_blocked() or locale.s(if running and running:get() then "ds.composer.queue_message" else "ds.composer.send_message")
			end,
		},
		opacity = function(): number
			return if send_blocked() then 0.35 else 1
		end,
		disabled = disabled,
		on_click = function()
			send()
		end,
		Icon { name = "arrow-up", size = 14 },
	}
	local function resuming(): boolean
		return props.resume ~= nil and props.resume() and not (running and running:get()) and not has_text() and not has_cargo() and not asking()
	end
	local resume_button = Button {
		id = "composer-resume",
		shape = "pill",
		size = "sm",
		icon = true,
		label = locale.t("ds.composer.resume"),
		on_click = props.on_resume,
		Icon { name = "action-play", size = 12 },
	}
	-- Plan on, with an agent that waits for your OK: the round Send opens
	-- into "Start plan", overshooting a little as Zeron's does.
	local start_plan_on = ui.computed(function(): boolean
		return plan_on() and props.plan_asks ~= nil and props.plan_asks() and not (running and running:get())
	end)
	local start_label_w = ui.use_state(60)
	local function start_can(): boolean
		return not disabled() and (has_text() or has_cargo())
	end
	local start_t = ui.use_state(if start_plan_on:peek() then 1 else 0)
	local start_run: any = nil
	ui.effect(function()
		local to = if start_plan_on:get() then 1 else 0
		if start_run then
			start_run:cancel()
			start_run = nil
		end
		if motion.reduced_now() then
			start_t:set(to)
		elseif start_t:peek() ~= to then
			start_run = anime.animate(start_t, { value = to, duration = 280, ease = "cubicBezier(.3,1.25,.5,1)" })
		end
	end)
	local send_morph = ui.button {
		id = "composer-start-plan",
		label = locale.t("ds.composer.plan.start"),
		h = CONTROL,
		w = function(): number
			return CONTROL + start_t:get() * (start_label_w:get() + 16)
		end,
		shrink = 0,
		overflow = "hidden",
		items = "center",
		justify = "center",
		rounded = "$radius.full",
		bg = "$zeron.primary.bg",
		color = "$zeron.primary.text",
		hover = { bg = "$zeron.primary.hover" },
		active = { bg = "$zeron.primary.active" },
		focus = control.ring,
		opacity = function(): number
			return if start_can() then 1 else 0.35
		end,
		on_click = function()
			if start_can() then
				send()
			end
		end,
		Icon { name = "arrow-up", size = 14 },
		ui.text {
			locale.t("ds.composer.plan.start"),
			on_resize = function(size: { w: number, h: number })
				local t = math.clamp(start_t:peek(), 0, 1)
				if t >= 1 and math.abs(start_label_w:peek() - size.w) >= 0.5 then
					start_label_w:set(size.w)
				end
			end,
			ml = function(): any
				return ui.raw(6 * math.clamp(start_t:get(), 0, 1), "gpui's Start plan morph")
			end :: any,
			max_w = function(): number
				return 200 * math.clamp(start_t:get(), 0, 1)
			end :: any,
			opacity = function(): number
				return math.clamp(start_t:get(), 0, 1)
			end,
			shrink = 0,
			text_size = "$font.size.sm",
			weight = "$font.weight.medium",
		},
	}
	local function showing_morph(): boolean
		return start_plan_on:get() or math.abs(start_t:get()) > 0.001
	end
	local stop_button = Button {
		shape = "pill",
		size = "sm",
		icon = true,
		label = locale.t("ds.composer.stop"),
		tooltip = locale.s("ds.composer.stop"),
		on_click = props.on_stop,
		ui.div { w = 10, h = 10, rounded = ui.raw(3, "the stop glyph's corner"), bg = "$zeron.primary.text" },
	}
	-- A question's way through: Back (from the second page), then Submit
	-- (or Next, with pages to come), dimmed until something is picked or
	-- typed. An approval's: Deny, Always allow, Allow.
	local function ask_controls(): Node
		return ui.row {
			gap = "$space.2",
			items = "center",
			ui.show(function()
				local q = question()
				return q ~= nil and (q.page or 1) > 1
			end, Button { locale.t("ds.composer.back"), size = "sm", tone = "ghost", on_click = go_back }),
			ui.show(function()
				return question() ~= nil
			end, Button {
				size = "sm",
				shape = "pill",
				label = function()
					local q = question()
					return locale.s(if q and q.pages and (q.page or 1) < q.pages then "ds.composer.next" else "ds.composer.submit")
				end,
				disabled = function()
					return not can_answer()
				end,
				on_click = answer,
				ui.text {
					function()
						local q = question()
						return locale.s(if q and q.pages and (q.page or 1) < q.pages then "ds.composer.next" else "ds.composer.submit")
					end,
				},
			}),
			ui.show(function()
				return approval() ~= nil
			end, ui.row {
				-- What an agent may do is the person's to say: no plugin
				-- reaches these.
				hook = false,
				gap = "$space.2",
				items = "center",
				Button { locale.t("ds.composer.deny"), size = "sm", tone = "ghost", on_click = props.on_deny },
				ui.show(
					function(): boolean
						local always = props.can_always
						if type(always) == "function" then
							return always()
						end
						return always ~= false
					end,
					Button {
						locale.t("ds.composer.always_allow"),
						size = "sm",
						tone = "secondary",
						on_click = function()
							if props.on_approve then
								props.on_approve("always")
							end
						end,
					}
				),
				Button {
					locale.t("ds.composer.allow"),
					size = "sm",
					shape = "pill",
					on_click = function()
						if props.on_approve then
							props.on_approve("once")
						end
					end,
				},
			}),
		}
	end

	-- Dictation's clock: seconds since it began; warning in its last ten.
	local voice_seconds = ui.use_state(0)
	if props.dictating then
		local since = 0
		local stop: (() -> ())? = nil
		ui.effect(function()
			if props.dictating:get() then
				since = os.time()
				voice_seconds:set(0)
				if stop then
					stop()
				end
				stop = ui.every(500, function()
					voice_seconds:set(os.time() - since)
				end)
			elseif stop then
				stop()
				stop = nil
			end
		end)
	end
	local function clock(): string
		local s = voice_seconds:get()
		return `{s // 60}:{string.format("%02d", s % 60)}`
	end
	-- The track, unrolling from the microphone over the controls (and, on
	-- one line, the draft): the room it has is from the attach's right edge
	-- to the microphone's left; it is revealed from the right.
	-- The track is there only while dictating (its dot's pulse and its
	-- voice would otherwise ask for frames forever, unseen): it comes in
	-- from the microphone's side and leaves the same way.
	local function voice_track(): Node
		local track_w = ui.ref()
		return ui.div {
			absolute = true,
			-- (Insets take points: the control, the sides, and a breath.)
			left = by_open(INSET_LINE + CONTROL + 8, INSET_OPEN + CONTROL + 8),
			right = by_open(INSET_LINE + CONTROL * 2 + 8 + 8, INSET_OPEN + CONTROL * 2 + 8 + 8),
			bottom = by_open(INSET_LINE, INSET_OPEN),
			h = "$zeron.composer.voice.h",
			ref = track_w,
			overflow = "hidden",
			rounded = "$radius.full",
			pointer_events = "none",
			ui.presence(function(): string?
				return if dictating() then "on" else nil
			end, function(_k: string): Node
				return ui.row {
					absolute = true,
					right = 0,
					top = 0,
					bottom = 0,
					w = function(): number
						return track_w.width or 0
					end,
					pl = "$space.3",
					pr = "$space.3",
					gap = "$space.2",
					items = "center",
					rounded = "$radius.full",
					bg = "$zeron.composer.voice.bg",
					ui.div {
						w = 8,
						h = 8,
						shrink = 0,
						rounded = "$radius.full",
						bg = "$zeron.composer.voice.ink",
						opacity = ui.loop { from = 0.35, to = 1, duration = 900, fps = 30 },
					},
					ui.text {
						locale.t("ds.composer.listening"),
						shrink = 0,
						text_size = "$font.size.sm",
						weight = "$font.weight.medium",
						color = "$text.primary",
					},
					ui.div {
						grow = 1,
						h = 16,
						min_w = 24,
						bg_shader = VOICE,
						shader_params = { "$zeron.composer.voice.ink", { 0, 0, 0, 0 }, { 0, 0, 0, 0 }, { 0, 0, 0, 0 } },
					},
					ui.text {
						clock,
						shrink = 0,
						tabular = true,
						font = "$font.mono",
						text_size = "$font.size.xs",
						color = function()
							return if VOICE_LIMIT - voice_seconds:get() <= VOICE_WARN then "$warning" else "$text.secondary"
						end,
					},
				}
			end, {
				enter = { opacity = { 0, 1 }, x = { 40, 0 }, duration = 320, ease = "out(3)" },
				exit = { opacity = 0, x = 40, duration = 200, ease = "out(3)" },
			}),
		}
	end

	-- Files over the pill: its own sense of it, or the caller's.
	local dropping = props.dropping or ui.use_state(false)

	-- What sits over the draft: the agent's ask, or the cargo strip.
	local head: Node = ui.column {
		on_resize = measure(head_h_seen, "h"),
		absolute = true,
		left = 0,
		right = 0,
		top = 0,
		w_full = true,
		ui.show(has_limit, limit_notice.Notice {
			limit = function(): any
				return if props.limit then props.limit() else nil
			end,
		}),
		ui.presence(function(): string?
			local q = question()
			if q then
				-- A new question (or its next page) is built afresh.
				return `question:{q.text}:{q.page or 1}:{if q.multi then 1 else 0}`
			elseif approval() then
				return "approval"
			elseif has_cargo() then
				return "cargo"
			end
			return nil
		end, function(key: string): Node
			local kind = string.match(key, "^(%a+)")
			if kind == "question" then
				local q = question() :: ask.Question
				return ask.Question {
					question = q,
					chosen = props.chosen or ui.use_state({} :: { [string]: boolean }),
					on_pick = props.on_pick,
					on_close = props.on_close_question,
					closes_on_escape = function()
						return (q.page or 1) <= 1 and not can_answer()
					end,
				}
			elseif kind == "approval" then
				return ask.Approval { approval = approval() :: ask.Approval }
			end
			-- An image chip opens its picture in the lightbox (unless the
			-- caller opens chips itself), growing from the chip; ← → go
			-- through the chips' pictures.
			lightboxes += 1
			local group = `composer{lightboxes}`
			local items = props.cargo :: State<{ cargo.Item }>
			local function open_image(id: string)
				local images: { Lightbox.Item } = {}
				local at = nil
				for _, c in items:peek() do
					if (c.kind == "image" or c.kind == "appshot") and c.src and c.src ~= "" then
						table.insert(images, { src = c.src, name = c.name, path = (c :: any).path, key = c.id })
						if c.id == id then
							at = #images
						end
					end
				end
				if at then
					Lightbox.open(images, at, group)
				end
			end
			return ui.column {
				w_full = true,
				cargo.Strip {
					items = items,
					on_remove = props.on_remove_cargo,
					on_retry = props.on_retry_cargo,
					on_open = props.on_open_cargo or open_image,
					on_open_pasted = if props.on_open_pasted
						then function(id: string)
							(props.on_open_pasted :: any)(id, function(text: string): boolean
								local el = field:peek()
								if el then
									el:replace_selection(text)
								end
								return el ~= nil
							end)
						end
						else nil,
					source = function(item: cargo.Item): { [string]: any }?
						if props.on_open_cargo then
							return nil
						end
						return Lightbox.source(group, item.id, 12)
					end,
				},
				Lightbox.Host(),
			}
		end, {
			w_full = true,
			enter = { opacity = { 0, 1 }, duration = 200, ease = "outCubic" },
			exit = { opacity = 0, duration = 120 },
		}),
	}

	-- What's predicted, while it may show.
	local function shown_prediction(): string?
		if not props.prediction or asking() or plan_on() or m.open() then
			return nil
		end
		local p = props.prediction()
		if not p or p == "" then
			return nil
		end
		local text = value:get()
		return if #text < #p and string.sub(p, 1, #text) == text then p else nil
	end
	-- What the pill is measured by: the draft, or the prediction over an
	-- empty one, which opens the pill out as a draft would.
	function measured_text(): string
		local text = value:get()
		if text == "" then
			return shown_prediction() or ""
		end
		return text
	end
	local function placeholder(): string
		if question() then
			return locale.s(if picked() > 0 then "ds.composer.placeholder.answer_picked" else "ds.composer.placeholder.answer")
		elseif approval() then
			return locale.s("ds.composer.placeholder.instead")
		end
		local whole = shown_prediction()
		if whole then
			return whole
		end
		if plan_on() then
			return locale.s(if props.plan_asks and props.plan_asks() then "ds.composer.placeholder.plan_asks" else "ds.composer.placeholder.plan")
		end
		-- (A message follows the language: read here, as it's drawn.)
		local own = props.placeholder :: any
		if own == nil then
			return locale.s("ds.composer.placeholder.default")
		end
		return if type(own) == "string" then own else own:get()
	end

	if props.toolbar_id then
		composer_surface.field = field
		ui.effect(function()
			local _ = pill.width, pill.height
			local el = pill:get()
			if el then
				composer_surface.element = el
				composer_surface.moved:set(composer_surface.moved:peek() + 1)
			end
		end)
	end
	-- The rest of a prediction the draft starts with, at the caret, while the
	-- caret is at the draft's end with nothing selected.
	local caret_moved = ui.use_state(0)
	local ghost = ui.use_state(nil :: { x: number, y: number, text: string }?)
	ui.effect(function()
		caret_moved:get()
		local text = value:get()
		local whole = shown_prediction()
		local el = field:peek() :: any
		if not whole or text == "" or not el then
			if ghost:peek() then
				ghost:set(nil)
			end
			return
		end
		local a, b = el:selection_bytes()
		if a ~= b or b ~= #text then
			if ghost:peek() then
				ghost:set(nil)
			end
			return
		end
		ui.after(0, function()
			local ok, at = pcall(function(): any
				return el:caret_rect()
			end)
			if not ok then
				return
			end
			if at then
				ghost:set { x = at.x - el.x, y = at.y - el.y, text = string.sub(whole, #text + 1) }
			end
		end)
	end)
	local function accept_prediction(): boolean
		local whole = shown_prediction()
		local el = field:peek()
		if not whole or not el or #whole == #value:peek() then
			return false
		end
		local a, b = el:selection_bytes()
		if a ~= b or b ~= #value:peek() then
			return false
		end
		value:set(whole)
		el:set_selection_bytes(#whole, #whole)
		return true
	end
	local surface = ui.div {
		ref = pill,
		on_resize = measure(pill_w, "w"),
		relative = true,
		w_full = true,
		h = eased(height),
		overflow = "hidden",
		rounded = "$zeron.composer.radius",
		border = 1,
		border_color = if props.plan
			then ui.tween(ui.computed(plan_on), { from = "$zeron.composer.edge", to = "$accent", duration = 300, ease = motion.EASE })
			else "$zeron.composer.edge",
		bg = function()
			return if material.see_through() then "$zeron.composer.tint" else "$zeron.composer.solid"
		end,
		backdrop_blur = function()
			return if material.see_through() then 16 else 0
		end,
		shadow = function()
			return if material.see_through() then "$zeron.composer.rest" else "$zeron.composer.shadow"
		end,
		on_file_hover = function(inside: boolean)
			dropping:set(inside)
		end,
		on_file_drop = function(files: { any })
			dropping:set(false)
			if props.on_files then
				props.on_files(files)
			end
		end,
		-- A press anywhere but a control goes to the draft.
		ui.div {
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			bottom = 0,
			cursor = "text",
			on_press = function()
				local el = field:peek()
				if el then
					el:focus()
				end
			end,
		},
		-- The measures: the draft on one line, and wrapped as it would be
		-- opened out (a mark after it, so a last empty line counts).
		ui.text {
			on_resize = measure(line_w, "w"),
			absolute = true,
			left = 0,
			top = 0,
			opacity = 0,
			pointer_events = "none",
			text_size = "$zeron.composer.text",
			line_height = "$zeron.composer.line",
			function()
				return measured_text()
			end,
		},
		ui.text {
			on_resize = measure(wrapped_h, "h"),
			absolute = true,
			left = TEXT_OPEN_SIDE,
			right = TEXT_OPEN_SIDE,
			top = 0,
			opacity = 0,
			pointer_events = "none",
			wrap = true,
			text_size = "$zeron.composer.text",
			line_height = "$zeron.composer.line",
			function()
				return measured_text() .. "\u{200B}"
			end,
		},
		head,
		-- The draft.
		ui.div {
			absolute = true,
			left = eased(text_left),
			right = eased(text_right),
			top = eased(text_top),
			h = eased(text_h),
			-- Its placeholder, the composer's own (it changes with what is
			-- asked): over the empty draft, taking no pointer.
			ui.show(
				function()
					return value:get() == ""
				end,
				ui.presence(function(): string
					return if plan_on() then "plan" else "idle"
				end, function(_kind: string): Node
					return ui.text {
						placeholder,
						absolute = true,
						left = 0,
						top = 0,
						right = 0,
						wrap = function(): boolean
							return open:get() and shown_prediction() ~= nil
						end :: any,
						truncate = function(): boolean
							return not (open:get() and shown_prediction() ~= nil)
						end :: any,
						pointer_events = "none",
						aria_hidden = true,
						text_size = "$zeron.composer.text",
						line_height = "$zeron.composer.line",
						color = "$text.tertiary",
					}
				end, {
					absolute = true,
					left = 0,
					top = 0,
					right = 0,
					bottom = 0,
					pointer_events = "none",
					enter = { opacity = { 0, 1 }, y = { 4, 0 }, duration = 220, ease = motion.EASE },
					exit = { opacity = 0, y = -4, duration = 220, ease = motion.EASE },
				})
			),
			ui.show(function(): boolean
				return ghost:get() ~= nil
			end, ui.text {
				function(): string
					local g = ghost:get()
					return if g then g.text else ""
				end,
				id = "composer-ghost",
				absolute = true,
				left = function(): number
					local g = ghost:get()
					return if g then g.x else 0
				end :: any,
				top = function(): number
					local g = ghost:get()
					return if g then g.y else 0
				end :: any,
				pointer_events = "none",
				aria_hidden = true,
				text_size = "$zeron.composer.text",
				line_height = "$zeron.composer.line",
				color = "$text.tertiary",
			}),
			ui.textarea(EditorMenu.on({
				ref = field,
				value = value,
				label = locale.t("ds.composer.message"),
				placeholder = "",
				read_only = disabled,
				w_full = true,
				h_full = true,
				px = 0,
				py = 0,
				border = 0,
				rounded = 0,
				bg = "#00000000",
				focus = { border = 0, outline = 0 },
				wrap = true,
				text_size = "$zeron.composer.text",
				line_height = "$zeron.composer.line",
				color = "$text.primary",
				-- The caret: the accent, two points wide, as Zeron's.
				caret_color = "$zeron.composer.caret",
				caret_width = ui.theme.get("zeron.composer.caret_width"),
				-- Markdown as it is typed: its markers greyed, bold, italic
				-- and struck text in their faces, code washed; and a wash
				-- behind each file, command or skill mentioned.
				highlight = mentions.highlight,
				active_descendant = m.descendant,
				on_caret = function(line: number, column: number, anchor: number, focus: number)
					m.on_caret(line, column, anchor, focus)
					caret_moved:set(caret_moved:peek() + 1)
				end,
				on_paste = function(e)
					-- (Files first: a copied file puts its name in `text` too.)
					if e.files and #e.files > 0 and props.on_files then
						props.on_files(e.files)
						return true
					end
					if e.text and props.on_paste_text then
						return props.on_paste_text(e.text)
					end
					if e.text == nil and props.on_paste_image then
						return props.on_paste_image()
					end
					return false
				end,
				-- Enter sends; Shift+Enter breaks the line, continuing a
				-- list (or undoing an empty item). The mentions' keys come
				-- first, then a question's (1 to 9 pick, Escape goes back
				-- or closes), then ⌘Enter sending the newest queued row.
				on_key = function(e)
					if dictating() and e.key == "Escape" then
						if props.on_cancel_dictation then
							props.on_cancel_dictation()
						end
						return true
					end
					if e.key == "Tab" and e.shift and not e.cmd and not e.alt then
						if m.open() or not available() or asking() then
							return m.open()
						end
						(props.plan :: State<boolean>):toggle()
						return true
					end
					if m.keys(e) then
						return true
					end
					if e.key == "Tab" and not e.shift and not e.cmd and not e.alt and accept_prediction() then
						return true
					end
					local q = question()
					if q then
						local d = string.match(e.key, "^(%d)$")
						if d and not has_text() and not e.cmd and not e.ctrl and not e.alt then
							local o = q.options[tonumber(d) :: number]
							if o and props.on_pick then
								props.on_pick(o.value)
							end
							return true
						end
						if e.key == "Escape" then
							if (q.page or 1) > 1 and props.on_back then
								go_back()
							elseif not can_answer() and props.on_close_question then
								props.on_close_question()
							end
							return true
						end
					end
					-- A bare Escape: the caller's first (stopping the agent, where
					-- settings say Escape does); else the keyboard goes back to
					-- the window, whose bare letters (the panel's B, F, T…, I
					-- to come back) work again, as vim's normal mode.
					if e.key == "Escape" and not e.shift and not e.cmd and not e.alt then
						if props.on_escape and props.on_escape() then
							return true
						end
						local el = field:peek()
						if el then
							el:blur()
							return true
						end
						return false
					end
					-- (⌘↩ under "mod_enter"; plain ↩ there breaks the line.)
					local enter_breaks = props.send_with ~= nil and props.send_with() == "mod_enter" and not e.cmd
					if e.key == "Enter" and not e.shift and not enter_breaks then
						if e.cmd and not has_text() and not has_cargo() and props.queue and props.on_queue_send then
							local list = props.queue:peek()
							local newest = list[#list]
							if newest then
								props.on_queue_send(newest.id)
								return true
							end
						end
						if stopping() or resuming() then
							return true
						end
						-- (⌘↩ with something written, where plain ↩ sends: the
						-- other of queue and steer.)
						send(if e.cmd and not enter_breaks and (props.send_with == nil or props.send_with() == "enter") then { inverted = true } else nil)
						return true
					end
					if e.key == "Enter" and (e.shift or enter_breaks) then
						local el = field:peek()
						if el then
							local a, b = el:selection_bytes()
							local from, to, text = nil, nil, nil
							if a and b then
								from, to, text = markdown.newline(value:peek(), a, b)
							end
							if from and to and text then
								el:set_selection_bytes(from, to)
								el:replace_selection(text)
								return true
							end
						end
					end
					return false
				end,
			}, { takes_paste = props.on_paste_image ~= nil or props.on_files ~= nil })),
		},
		-- The controls, on the pill's foot.
		ui.div {
			absolute = true,
			left = by_open(INSET_LINE, INSET_OPEN),
			bottom = by_open(INSET_LINE, INSET_OPEN),
			ui.row {
				on_resize = measure(leading_seen, "w"),
				gap = "$space.1",
				items = "center",
				ui.row {
					hook = "chat.composer.toolbar.start",
					gap = "$space.1",
					items = "center",
					ui.show(function()
						return not asking()
					end, attach),
					if props.review
						then ui.show(function(): boolean
							return not asking() and not dictating()
						end, props.review)
						else nil,
					if props.project and props.projects
						then ui.show(function(): boolean
							return not asking() and not dictating() and not (props.branch_fixed and props.branch_fixed() ~= nil)
						end, project_chip(props.project, props.projects :: any, props.on_new_project, function(): number
							return if labelled:get() then math.min(PROJECT_MAX, pill_w:get() * 0.25) else 0
						end))
						else nil,
					if props.plan
						then ui.show(function(): boolean
							return available() and not asking() and not dictating()
						end, plan_chip())
						else nil,
				},
			},
		},
		voice_track(),
		ui.row {
			on_resize = measure(trailing_seen, "w"),
			absolute = true,
			right = by_open(INSET_LINE, INSET_OPEN),
			-- (Its row is 32: the control centred in it.)
			bottom = by_open(INSET_LINE - 2, INSET_OPEN - 2),
			h = 32,
			items = "center",
			gap = "$space.2",
			ui.show(asking, ask_controls()),
			ui.show(function()
				return not asking() and not dictating()
			end, ui.row {
				hook = "chat.composer.toolbar.end",
				items = "center",
				min_w = 0,
				-- (Out of plugins' reach: it says what the agent may do unasked.)
				if access then ui.div { on_resize = measure(access_seen, "w"), shrink = 0, hook = false, access } else nil,
				ui.row {
					items = "center",
					min_w = 0,
					max_w = function(): number
						return model_cap:get()
					end :: any,
					model,
				},
			}),
			ui.show(function()
				return not asking()
			end, ui.row {
				on_resize = measure(actions_seen, "w"),
				items = "center",
				shrink = 0,
				gap = "$space.2",
				mic,
				ui.show(stopping, stop_button),
				ui.show(resuming, resume_button),
				ui.show(function()
					return not stopping() and not resuming() and not showing_morph()
				end, send_button),
				ui.show(function()
					return not stopping() and not resuming() and showing_morph()
				end, send_morph),
			}),
		},
		-- Files over it: the pill says what a drop does, over everything, as
		-- Zeron's drop overlay does (shell.rs `attachment_drop_overlay`): the
		-- scrim, the words in the text's ink, there while files are over it.
		-- (Not when the caller covers more than the pill with its own.)
		if props.drop_overlay ~= false
			then DropOverlay {
				shown = function(): boolean
					return dropping:get()
				end,
			}
			else nil,
	}

	local footer: Node? = nil
	do
		local left: { Child } = {}
		local fixed = props.branch_fixed
		local function new_chat(): boolean
			return not (fixed and fixed() ~= nil)
		end
		local project = props.project
		local function in_project(): boolean
			return project == nil or project:get() ~= NO_PROJECT
		end
		local draft: { Child } = {}
		-- A new chat's, as gpui's: the device, then the branch and the checkout, a
		-- project's alone. (Cloud's machine checks the repository out itself.)
		if props.device and props.devices then
			local device = props.device :: State<string?>
			local devices = props.devices
			local function choices(): { Choice }
				local out: { Choice } = {}
				for _, d in list_of(devices) :: { Device } do
					local icon: Icon.Name = if d.cloud then "cloud" else "monitor"
					table.insert(out, {
						value = d.value,
						label = d.label,
						icon = icon,
						tag = if d.you then locale.s("ds.composer.device.you") else nil,
						warn = d.offline,
						description = d.description,
						disabled = d.disabled,
					})
				end
				return out
			end
			local function chosen(): Choice?
				local v = device:get()
				for _, c in choices() do
					if c.value == v then
						return c
					end
				end
				return choices()[1]
			end
			table.insert(draft, ui.show(function(): boolean
				return #choices() > 0
			end, Dropdown {
				value = device,
				items = choices,
				label = locale.t("ds.composer.device.label") :: any,
				search = locale.s("ds.composer.device.search"),
				width = 260,
				trigger = function(m: Dropdown.Menu): Node
					return footer_chip({
						label = locale.t("ds.composer.device.label"),
						icon = function(): string
							local c = chosen()
							return if c and c.icon then c.icon else "monitor"
						end,
						says = function(): string
							local c = chosen()
							return if c then c.label else ""
						end,
						warn = function(): boolean
							local c = chosen()
							return c ~= nil and c.warn == true
						end,
					}, m)
				end,
			}))
		end
		if props.branch then
			-- The branch's name as its row has it, else as it is (its list
			-- still coming), else "No ref".
			local branch_state = props.branch :: State<string?>
			local function branch_name(): string
				local v = branch_state:get()
				for _, item in list_of(props.branches) :: { Dropdown.Item } do
					if item.value == v then
						return item.label
					end
				end
				return v or locale.s("ds.composer.branch.no_ref")
			end
			table.insert(draft, ui.show(in_project, Dropdown {
				value = props.branch,
				label = locale.t("ds.composer.branch.label") :: any,
				-- (A string: Dropdown names its field by it only so.)
				search = locale.s("ds.composer.branch.search"),
				items = (props.branches or {}) :: any,
				empty = props.branches_empty,
				cap = REFS_SHOWN,
				cap_words = refs_cap_words,
				width = 320,
				trigger = function(m: Dropdown.Menu): Node
					return footer_chip({
						label = locale.t("ds.composer.branch.label"),
						icon = function(): string
							return "git-branch"
						end,
						says = branch_name,
						says_id = "composer-place-branch",
					}, m)
				end,
			}))
		end
		if props.checkout then
			local checkout = props.checkout :: State<string?>
			local function kind(): Choice
				local v = checkout:get()
				local all = CHECKOUTS()
				for _, c in all do
					if c.value == v then
						return c
					end
				end
				return all[2]
			end
			table.insert(draft, ui.show(function(): boolean
				return in_project() and not on_cloud(props.device, props.devices)
			end, Dropdown {
				value = checkout,
				items = CHECKOUTS,
				label = locale.t("ds.composer.checkout.label") :: any,
				search = false,
				width = 260,
				trigger = function(m: Dropdown.Menu): Node
					return footer_chip({
						label = locale.t("ds.composer.checkout.label"),
						icon = function(): string
							return kind().icon or "folder"
						end,
						says = function(): string
							return kind().label
						end,
					}, m)
				end,
			}))
		end
		local draft_row = ui.row { gap = "$space.1", items = "center", min_w = 0, table.unpack(draft) }
		if fixed then
			table.insert(left, ui.show(function(): boolean
				return not new_chat()
			end, session_footer(fixed, props.branch_menu), ui.show(new_chat, draft_row)))
		else
			table.insert(left, draft_row)
		end
		local stats, working = props.stats, props.working
		local function has_git(): boolean
			return props.footer_git == nil or props.footer_git()
		end
		if stats then
			table.insert(left, ui.show(function(): boolean
				return not new_chat() and has_git()
			end, SessionStats.Chip {
				stats = stats,
				working = function(): boolean
					return working ~= nil and working()
				end,
				side = side,
			}))
		end
		footer = ui.row {
			id = props.toolbar_id,
			h = 24,
			mt = "$space.2",
			px = "$zeron.space.snug",
			items = "center",
			gap = "$space.1",
			-- Clipped: its chips give way under what stays on the right.
			ui.row {
				gap = "$space.1",
				items = "center",
				min_w = 0,
				grow = 1,
				overflow = "hidden",
				table.unpack(left),
			},
			-- (Plugins': the GitHub plugin's pull request stands here, as gpui's before the ring.)
			if props.footer_status ~= false
				then ui.show(has_git, ui.row { class = "composer-footer-status", items = "center", shrink = 0 })
				else nil,
			ui.row {
				items = "center",
				shrink = 0,
				gap = "$space.1",
				if props.compaction
					then Compaction.CopyChip {
						plan = props.compaction.plan,
						working = function(): boolean
							return props.working ~= nil and props.working()
						end,
					}
					else nil,
				if props.usage
					then ui.show(function(): boolean
						local u = (props.usage :: () -> Usage?)()
						return u ~= nil and u.window ~= nil and u.window > 0
					end, ring(0, props.usage, side, props.compaction, props.stats))
					elseif props.context then ring(props.context, nil, side)
					else nil,
				if props.on_terminal
					then ui.show(function(): boolean
						return not new_chat()
					end, terminal_button(props.on_terminal, props.terminal_open, props.terminal_keys))
					else nil,
			},
		}
	end

	-- The trays, behind the pill.
	local trays: Node? = nil
	if props.queue or props.todos or props.tasks then
		trays = dock.Dock {
			queue = props.queue,
			can_steer = props.can_steer,
			on_queue_send = props.on_queue_send,
			on_queue_steer_all = props.on_queue_steer_all,
			composer_empty = function(): boolean
				return not has_text() and not has_cargo()
			end,
			pictures = props.queue_pictures,
			on_queue_edit = props.on_queue_edit,
			on_queue_remove = props.on_queue_remove,
			on_queue_reorder = props.on_queue_reorder,
			on_queue_save = props.on_queue_save,
			on_queue_cancel = props.on_queue_cancel,
			todos = props.todos,
			live = function()
				return running ~= nil and running:get()
			end,
			on_dismiss_todos = props.on_dismiss_todos,
			tasks = props.tasks,
			on_stop_task = props.on_stop_task,
			on_stop_all = props.on_stop_all,
			open = props.tray,
			narrow = narrow,
		}
	end

	-- The side chat's tab, behind the pill's (or the top tray's) top right
	-- corner: 16 further in for each tray under it, tucked 26 behind it.
	local function side_tab(): Node?
		local tab = props.side_tab
		if not tab then
			return nil
		end
		local function trays_under(): number
			local n = 0
			if props.queue and #props.queue:get() > 0 then
				n += 1
			end
			if props.todos and #props.todos:get() > 0 then
				n += 1
			end
			if props.tasks and #props.tasks:get() > 0 then
				n += 1
			end
			return n
		end
		return ui.show(function(): boolean
			return tab.shown()
		end, ui.row {
			w_full = true,
			justify = "end",
			shrink = 0,
			pointer_events = "none",
			pr = function(): any
				return ui.raw(16 * (trays_under() + 1), "gpui's side chat tab, a step in per tray")
			end :: any,
			-- (Taking no room: nothing under it moves as it comes.)
			h = 42,
			mb = ui.raw(-42, "its own height"),
			y = -24,
			ui.button {
				id = "composer-side-chat",
				label = locale.t("ds.composer.side_chat"),
				tooltip = locale.s("ds.composer.side_chat_tip"),
				pointer_events = "auto",
				h = 42,
				px = ui.raw(8, "gpui's side chat tab"),
				pb = ui.raw(18, "gpui's side chat tab"),
				gap = ui.raw(5, "gpui's side chat tab"),
				items = "center",
				shrink = 0,
				rounded_tl = ui.raw(10, "gpui's side chat tab"),
				rounded_tr = ui.raw(10, "gpui's side chat tab"),
				border = 1,
				border_sides = { 1, 1, 0, 1 },
				border_color = function(): string
					return if tab.open() then "$accent" else "$zeron.composer.tray.edge"
				end,
				bg = function(): string
					return if material.see_through() then "$zeron.composer.tray.tint" else "$zeron.composer.tray.solid"
				end,
				color = function(): string
					return if tab.open() then "$text.primary" else "$text.tertiary"
				end,
				hover = { opacity = 0.85 },
				on_click = tab.on_click,
				Icon { name = "chat-round-line", size = 12 },
				ui.show(function(): boolean
					return tab.status() ~= nil
				end, ui.div {
					w = 6,
					h = 6,
					shrink = 0,
					rounded = "$radius.full",
					bg = function(): string
						return tab.status() or "#00000000"
					end,
				}),
				ui.text { locale.t("ds.composer.side_chat"), shrink = 0, wrap = false, text_size = "$font.size.xs" },
				ui.text {
					function(): string
						return tab.hint() or ""
					end,
					shrink = 0,
					text_size = "$font.size.xs",
					color = "$text.disabled",
				},
			},
		})
	end
	local function pill_row(): Node?
		if not props.pills then
			return nil
		end
		return ui.row {
			w_full = true,
			shrink = 0,
			pointer_events = "none",
			h = 28,
			mb = ui.raw(-28, "its own height"),
			y = -34,
			pr = if props.side_tab
				then function(): any
					return if (props.side_tab :: any).shown() then ui.raw(112, "clear of the side chat's tab") else 0
				end :: any
				else nil,
			props.pills,
		}
	end
	local function caption_line(): Node?
		local caption = props.caption
		if not caption then
			return nil
		end
		return ui.show(function(): boolean
			return caption() ~= nil
		end, ui.text {
			function(): string
				return caption() or ""
			end,
			mx = ui.raw(8, "gpui's caption"),
			mb = ui.raw(6, "gpui's caption"),
			wrap = true,
			text_size = ui.raw(11, "gpui's caption"),
			line_height = 14,
			color = "$text.disabled",
		})
	end

	-- The pill (the mentions' popover anchored on it: its panel opens at
	-- the caret inside), its trays behind, the footer under.
	local body = ui.column {
		w_full = true,
		relative = true,
		opacity = function(): number
			return if disabled() then (ui.theme.get("zeron.disabled") :: number?) or 0.4 else 1
		end,
		pill_row(),
		side_tab(),
		trays,
		caption_line(),
		m.menu(ui.div {
			relative = true,
			w_full = true,
			ui.div {
				absolute = true,
				left = -3,
				right = -3,
				top = -3,
				bottom = -3,
				rounded = ui.raw(control.nest("$zeron.composer.radius", -3), "the pill's corner, 3 out"),
				border = 3,
				border_color = "$accent",
				pointer_events = "none",
				opacity = ui.tween(ui.computed(function(): number
					return if plan_on() then 0.14 else 0
				end), { duration = 300, ease = motion.EASE }),
			},
			ui.div {
				absolute = true,
				left = 0,
				right = 0,
				top = 0,
				bottom = 0,
				rounded = "$zeron.composer.radius",
				shadow = "$zeron.composer.plan_glow",
				pointer_events = "none",
				opacity = ui.tween(ui.computed(function(): number
					return if plan_on() then 1 else 0
				end), { duration = 300, ease = motion.EASE }),
			},
			surface,
		}),
		footer,
	}

	return ui.column {
		w_full = true,
		gap = 0,
		if props.notice then notice_line(props.notice, props.on_notice_action, props.on_dismiss_notice) else nil,
		props.review_findings,
		if props.compaction then Compaction.Chip { label = props.compaction.label } else nil,
		if props.compaction
			then Compaction.Resume {
				offer = props.compaction.resume,
				on_compact = props.compaction.on_resume_compact,
				on_keep = props.compaction.on_resume_keep,
			}
			else nil,
		body,
	}
end)
