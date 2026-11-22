-- A question the agent asks alone, drawn in its reply where it asked: a card
-- to choose in (with a note, when the question takes one), or chips one
-- press answers. An answer to a question, never a permission: approvals stay
-- the composer's. Its words are the agent's, drawn as plain text.
--
--   AskCard.Card { request = "r1", question = q, answer = function(a) return promise end }
--   AskCard.reveal("r1")

local Badge = require("@self/badge").Badge
local Button = require("@self/button")
local Choice = require("@self/choice")
local Icon = require("@self/icon")
local TextField = require("@self/text_field")
local locale = require("@self/locale")

local M = {}

local CLEAR = Choice.CLEAR

export type Option = { value: string, label: string, detail: string?, recommended: boolean? }
export type Question = {
	header: string?,
	text: string,
	options: { Option },
	multi: boolean?,
	note: boolean?,
	quick: boolean?,
}
export type Answer = { values: { string }, note: string? }
export type Props = {
	request: string,
	question: Question,
	answer: (Answer) -> any,
}

local cards: { [string]: any } = {}

function M.reveal(request: string): boolean
	local r = cards[request]
	local el = if r then r:peek() else nil
	return el ~= nil and el:scroll_into_view { block = "center" } == true
end

local function small(text: any, color: string?): Node
	return ui.text {
		text,
		literal = true,
		wrap = true,
		min_w = 0,
		text_size = "$font.size.sm",
		line_height = "$font.line.sm",
		color = color or "$text.tertiary",
	}
end

local function sent_by(props: Props, answer: Answer, busy: State<boolean>, failed: State<boolean>)
	busy:set(true)
	failed:set(false)
	local p = props.answer(answer)
	if p and p.after then
		p:after(function() end, function()
			busy:set(false)
			failed:set(true)
		end)
	end
end

local function Card(props: Props, ref: any): Node
	local q = props.question
	local chosen = ui.use_state({} :: { [string]: boolean })
	local note = ui.use_state("")
	local busy = ui.use_state(false)
	local failed = ui.use_state(false)
	local function any_chosen(): boolean
		return next(chosen:get()) ~= nil
	end
	local rows: { Node } = {}
	for _, o in q.options do
		local function on(): boolean
			return chosen:get()[o.value] == true
		end
		local words: { Node } = {
			ui.row {
				w_full = true,
				min_w = 0,
				gap = "$space.2",
				items = "center",
				flex_wrap = true,
				ui.text {
					o.label,
					literal = true,
					wrap = true,
					min_w = 0,
					text_size = "$zeron.visual.text.body.size",
					line_height = "$zeron.visual.text.body.line",
					color = "$text.primary",
				},
				if o.recommended then Badge { locale.s("ds.ask_card.recommended"), tone = "accent", size = "sm" } else nil,
			},
		}
		if o.detail and o.detail ~= "" then
			table.insert(words, ui.column { class = "ask-card-detail", w_full = true, pt = ui.raw(1, "a description under its option, as the design's"), small(o.detail) })
		end
		table.insert(rows, Choice.Row {
			class = "ask-card-option",
			multi = q.multi == true,
			label = o.label,
			chosen = on,
			on_pick = function()
				if busy:peek() then
					return
				end
				local now = table.clone(chosen:peek())
				if q.multi then
					now[o.value] = not now[o.value] or nil
				else
					now = { [o.value] = true }
				end
				chosen:set(now)
			end,
			look = function(): (string, string, string)
				if on() then
					return "$zeron.primary.bg", CLEAR, "check:$zeron.primary.text"
				end
				return CLEAR, "$border.strong", ""
			end,
			body = ui.column { grow = 1, min_w = 0, table.unpack(words) },
		})
	end
	local function send()
		if busy:peek() or not any_chosen() then
			return
		end
		local values: { string } = {}
		for _, o in q.options do
			if chosen:peek()[o.value] then
				table.insert(values, o.value)
			end
		end
		local words = string.match(note:peek(), "^%s*(.-)%s*$") or ""
		sent_by(props, { values = values, note = if q.note and words ~= "" then words else nil }, busy, failed)
	end
	return ui.column {
		class = "ask-card",
		ref = ref,
		hook = false,
		role = "group",
		label = q.text,
		w_full = true,
		min_w = 0,
		border = 1,
		border_color = "$border",
		rounded = ui.raw(12, "the visual's frame, as the design's"),
		pt = ui.raw(11, "the visual's frame, as the design's"),
		px = ui.raw(14, "the visual's frame, as the design's"),
		pb = ui.raw(12, "the visual's frame, as the design's"),
		ui.row {
			class = "ask-card-top",
			w_full = true,
			min_h = 22,
			gap = "$space.2",
			items = "center",
			ui.text {
				string.upper(if q.header and q.header ~= "" then q.header else locale.s("ds.composer_ask.question")),
				literal = true,
				truncate = true,
				min_w = 0,
				text_size = ui.raw(11, "a question's head, as the design's"),
				letter_spacing = 0.66,
				weight = "$font.weight.medium",
				color = "$text.tertiary",
			},
			ui.div { grow = 1 },
			ui.row {
				class = "ask-card-wait",
				gap = "$space.1",
				items = "center",
				shrink = 0,
				Icon { name = "clock-circle", size = 12, color = "$text.tertiary" },
				ui.text { locale.s("ds.ask_card.waiting"), text_size = "$font.size.xs", color = "$text.tertiary" },
			},
		},
		ui.column {
			w_full = true,
			mt = "$space.1",
			mb = "$space.1",
			ui.text {
				q.text,
				literal = true,
				wrap = true,
				min_w = 0,
				text_size = "$zeron.visual.text.body.size",
				line_height = "$zeron.visual.text.body.line",
				weight = "$font.weight.semibold",
				color = "$text.primary",
			},
		},
		ui.column {
			class = "ask-card-options",
			role = if q.multi then "group" else "radiogroup",
			label = q.text,
			self = "stretch",
			mt = "$space.1",
			mx = ui.raw(-8, "options' washes hang past their words, as the design's"),
			gap = ui.raw(2, "between options, as the design's"),
			rows,
		},
		if q.note
			then ui.row {
				class = "ask-card-note",
				w_full = true,
				mt = "$space.2",
				TextField {
					value = note,
					placeholder = locale.s("ds.ask_card.note"),
					name = locale.s("ds.ask_card.note"),
					size = "sm",
					grow = 1,
					disabled = function(): boolean
						return busy:get()
					end,
					on_key = function(ev: any): boolean
						if ev.key == "Enter" then
							send()
							return true
						end
						return false
					end,
				},
			}
			else nil,
		ui.row {
			class = "ask-card-foot",
			w_full = true,
			min_h = 26,
			mt = ui.raw(10, "the question's foot, as the design's"),
			gap = "$space.2",
			items = "center",
			ui.text {
				function(): string
					if failed:get() then
						return locale.s("ds.ask_card.failed")
					end
					return locale.s(if q.multi then "ds.ask_card.choose_many" else "ds.ask_card.choose_one")
				end,
				class = "ask-card-hint",
				literal = true,
				wrap = true,
				grow = 1,
				min_w = 0,
				text_size = "$font.size.sm",
				line_height = "$font.line.sm",
				color = function(): string
					return if failed:get() then "$danger" else "$text.tertiary"
				end,
			},
			Button {
				class = "ask-card-send",
				tone = "primary",
				size = "sm",
				busy = busy,
				disabled = function(): boolean
					return not any_chosen()
				end,
				on_click = send,
				ui.text { locale.s("ds.ask_card.send"), literal = true },
			},
		},
	}
end

local function Chips(props: Props, ref: any): Node
	local q = props.question
	local pressed = ui.use_state(nil :: string?)
	local failed = ui.use_state(false)
	local chips: { Node } = {}
	for _, o in q.options do
		local function busy(): boolean
			return pressed:get() == o.value
		end
		table.insert(chips, ui.button {
			class = "ask-chip",
			label = o.label,
			h = 28,
			px = "$space.3",
			items = "center",
			rounded = ui.raw(14, "a quick reply's chip, as the design's"),
			border = 1,
			border_color = "$border.strong",
			bg = function(): string
				return if busy() then "$zeron.secondary.bg" else CLEAR
			end,
			color = "$text.primary",
			text_size = "$font.size.sm",
			hover = { bg = "$hover" },
			opacity = function(): number
				return if pressed:get() ~= nil and not busy() then 0.4 else 1
			end,
			disabled = function(): boolean
				return pressed:get() ~= nil
			end,
			on_click = function()
				if pressed:peek() ~= nil then
					return
				end
				pressed:set(o.value)
				failed:set(false)
				local p = props.answer({ values = { o.value } })
				if p and p.after then
					p:after(function() end, function()
						pressed:set(nil)
						failed:set(true)
					end)
				end
			end,
			ui.text {
				function(): string
					return if busy() then locale.s("ds.ask_card.sending") else o.label
				end,
				literal = true,
				truncate = true,
			},
		})
	end
	return ui.column {
		class = "ask-quick",
		ref = ref,
		hook = false,
		role = "group",
		label = q.text,
		w_full = true,
		min_w = 0,
		ui.text {
			q.text,
			literal = true,
			wrap = true,
			min_w = 0,
			mb = "$space.2",
			text_size = "$zeron.visual.text.body.size",
			line_height = "$zeron.visual.text.body.line",
			color = "$text.primary",
		},
		ui.row { class = "ask-chips", w_full = true, flex_wrap = true, gap = ui.raw(6, "between chips, as the design's"), chips },
		ui.show(function(): boolean
			return failed:get()
		end, ui.column { w_full = true, pt = "$space.1", small(locale.s("ds.ask_card.failed"), "$danger") }),
	}
end

M.Card = ui.component(function(props: Props): Node
	local ref = ui.ref()
	cards[props.request] = ref
	ui.on_cleanup(function()
		if cards[props.request] == ref then
			cards[props.request] = nil
		end
	end)
	return ui.column {
		w_full = true,
		min_w = 0,
		pt = if props.question.quick then 0 else ui.raw(2, "a question in the reply, as the design's"),
		pb = ui.raw(14, "a question in the reply, as the design's"),
		if props.question.quick then Chips(props, ref) else Card(props, ref),
	}
end)

return M
