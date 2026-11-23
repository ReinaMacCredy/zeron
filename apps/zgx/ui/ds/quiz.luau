-- `ui.quiz`, graded here with no turn of the agent's (spec: choose, then Check).
--   Quiz { questions = { { kind, question, hint?, options = { { text, correct?, why? } } } },
--     essay = { question, placeholder? }?, saved = fn, save = fn, on_send = fn }

local Button = require("@self/button")
local Choice = require("@self/choice")
local Icon = require("@self/icon")
local Stat = require("@self/stat")
local TextField = require("@self/text_field")
local locale = require("@self/locale")

export type Option = { text: string, correct: boolean?, why: string? }
export type Question = { kind: string, question: string, hint: string?, options: { Option } }
export type Props = {
	questions: { Question },
	essay: { question: string, placeholder: string? }?,
	saved: (() -> any)?,
	save: ((state: any) -> ())?,
	on_send: ((text: string) -> boolean)?,
}

local LETTERS = { "A", "B", "C", "D", "E", "F", "G", "H" }
local CLEAR = Choice.CLEAR

local function right(q: Question, chosen: { [number]: boolean }?): boolean
	for j, o in q.options do
		if ((chosen or {})[j] == true) ~= (o.correct == true) then
			return false
		end
	end
	return true
end

local function small(text: any, color: string?): Node
	return ui.text {
		text,
		literal = true,
		wrap = true,
		min_w = 0,
		text_size = "$zeron.visual.text.sm.size",
		line_height = "$zeron.visual.text.sm.line",
		color = color or "$text.tertiary",
	}
end

local function body(text: any, weight: string?): Node
	return ui.text {
		text,
		literal = true,
		wrap = true,
		min_w = 0,
		text_size = "$zeron.visual.text.body.size",
		line_height = "$zeron.visual.text.body.line",
		weight = weight,
		color = "$text.primary",
	}
end

local function ghost(props: { class: string, label: string?, name: string?, icon: Node?, disabled: (() -> boolean)?, on: (() -> boolean)?, on_click: () -> () }): Node
	local kids: { Node } = {}
	if props.icon then
		table.insert(kids, props.icon)
	end
	if props.label then
		table.insert(kids, ui.text { props.label, literal = true })
	end
	return ui.button {
		class = props.class,
		label = props.name or props.label,
		h = 24,
		px = ui.raw(7, "the quiz's quiet buttons, as the design's"),
		gap = "$space.1",
		items = "center",
		shrink = 0,
		rounded = ui.raw(7, "the quiz's quiet buttons, as the design's"),
		bg = function(): string
			return if props.on and props.on() then "$zeron.secondary.bg" else CLEAR
		end,
		color = "$text.tertiary",
		text_size = "$font.size.xs",
		hover = { bg = "$hover", color = "$text.primary" },
		disabled = props.disabled,
		focus = { border = 0 },
		on_click = props.on_click,
		kids,
	}
end

return ui.component(function(props: Props): Node
	local questions = props.questions or {}
	local count = #questions
	-- The quiz as it stands; `tick` tells what reads it that it changed.
	local S = {
		i = 1,
		view = "q",
		done = false,
		asked = false,
		sent = false,
		chosen = {} :: { [number]: { [number]: boolean } },
		checked = {} :: { [number]: boolean },
		hint = {} :: { [number]: boolean },
	}
	local kept = if props.saved then props.saved() else nil
	if type(kept) == "table" then
		S.i = math.clamp(tonumber(kept.i) or 1, 1, math.max(count, 1))
		S.view = if kept.view == "sum" then "sum" else "q"
		S.done, S.asked, S.sent = kept.done == true, kept.asked == true, kept.sent == true
		for key, picks in kept.answers or {} do
			local qi = tonumber(key)
			if qi and questions[qi] and type(picks) == "table" then
				local set = {}
				for _, j in picks do
					set[j] = true
				end
				S.chosen[qi], S.checked[qi] = set, true
			end
		end
	end
	local tick = ui.use_state(0)
	local function read()
		tick:get()
	end
	local function changed(keep: boolean?)
		tick:set(tick:peek() + 1)
		if keep and props.save then
			local answers = {}
			for qi in S.checked do
				local picks = {}
				for j in S.chosen[qi] or {} do
					table.insert(picks, j)
				end
				table.sort(picks)
				answers[tostring(qi)] = picks
			end
			props.save { i = S.i, view = S.view, done = S.done, asked = S.asked, sent = S.sent, answers = answers }
		end
	end
	local function graded(qi: number): boolean?
		if not S.checked[qi] then
			return nil
		end
		return right(questions[qi], S.chosen[qi])
	end
	local function first_open(): number?
		for qi = 1, count do
			if not S.checked[qi] then
				return qi
			end
		end
		return nil
	end
	local function send(text: string): boolean
		return props.on_send ~= nil and props.on_send(text) == true
	end

	local function progress(): Node
		local bars: { Node } = {}
		for qi = 1, count do
			table.insert(bars, ui.div {
				class = "quiz-bar",
				w = 18,
				h = 4,
				rounded = ui.raw(2, "the quiz's progress, as the design's"),
				bg = function(): string
					read()
					local g = graded(qi)
					if g ~= nil then
						return if g then "$success" else "$danger"
					end
					return if S.view == "q" and S.i == qi then "$text.primary" else "$border.strong"
				end,
			})
		end
		return ui.row { class = "quiz-progress", gap = ui.raw(3, "the quiz's progress, as the design's"), items = "center", bars }
	end

	local function head(words: () -> string, extra: { Node }?): Node
		local kids: { Node } = {
			ui.text {
				words,
				literal = true,
				text_size = "$zeron.visual.text.xs.size",
				line_height = "$zeron.visual.text.xs.line",
				color = "$text.tertiary",
				tabular = true,
			},
			progress(),
			ui.div { grow = 1 },
		}
		for _, node in extra or {} do
			table.insert(kids, node)
		end
		return ui.row { class = "quiz-top", w_full = true, min_h = 26, gap = "$space.2", items = "center", kids }
	end

	local function look(q: Question, qi: number, j: number): () -> (string, string, string)
		local multi = q.kind == "multi"
		local o = q.options[j]
		return function(): (string, string, string)
			read()
			local chosen = (S.chosen[qi] or {})[j] == true
			if not S.checked[qi] then
				if chosen then
					return "$zeron.primary.bg", CLEAR, if multi then "check:$zeron.primary.text" elseif q.kind == "single" then "letter:$zeron.primary.text" else ""
				end
				return CLEAR, "$border.strong", if q.kind == "single" then "letter:$text.tertiary" else ""
			end
			if o.correct and (chosen or not multi) then
				return "$success", CLEAR, "check:$zeron.destructive.text"
			elseif o.correct then
				return CLEAR, "$success", "check:$success"
			elseif chosen then
				return "$danger", CLEAR, "cross:$zeron.destructive.text"
			end
			return CLEAR, "$border.strong", if q.kind == "single" then "letter:$text.tertiary" else ""
		end
	end

	local function option(q: Question, qi: number, j: number): Node
		local o = q.options[j]
		local function checked(): boolean
			read()
			return S.checked[qi] == true
		end
		local function chosen(): boolean
			read()
			return (S.chosen[qi] or {})[j] == true
		end
		local function pick()
			if S.checked[qi] then
				return
			end
			local now = S.chosen[qi] or {}
			if q.kind == "multi" then
				now[j] = not now[j] or nil
			else
				now = { [j] = true }
			end
			S.chosen[qi] = now
			changed()
		end
		return Choice.Row {
			class = "quiz-option",
			mark_class = "quiz-mark",
			multi = q.kind == "multi",
			label = o.text,
			letter = LETTERS[j],
			chosen = chosen,
			on_pick = pick,
			look = look(q, qi, j),
			settled = checked,
			opacity = function(): number
				return if checked() and not chosen() and not o.correct then 0.5 else 1
			end,
			body = ui.column {
				grow = 1,
				min_w = 0,
				body(o.text),
				if o.why and o.why ~= ""
					then ui.show(function(): boolean
						return checked() and (chosen() or o.correct == true)
					end, ui.column { class = "quiz-why", w_full = true, pt = ui.raw(2, "feedback under its option, as the design's"), small(o.why) })
					else nil,
			},
		}
	end

	local function question(qi: number): Node
		local q = questions[qi]
		local multi = q.kind == "multi"
		local function checked(): boolean
			read()
			return S.checked[qi] == true
		end
		local function any_chosen(): boolean
			read()
			return next(S.chosen[qi] or {}) ~= nil
		end
		local function go_to(to: number)
			S.i = math.clamp(to, 1, count)
			changed(true)
		end
		local options: { Node } = {}
		for j in q.options do
			table.insert(options, option(q, qi, j))
		end
		local extra: { Node } = {
			ghost {
				class = "quiz-prev",
				name = locale.s("ds.quiz.previous"),
				icon = Icon { name = "chevron-left", size = 13 },
				disabled = function(): boolean
					return qi <= 1
				end,
				on_click = function()
					go_to(qi - 1)
				end,
			},
			ghost {
				class = "quiz-next",
				name = locale.s("ds.quiz.next_question"),
				icon = Icon { name = "chevron-right", size = 13 },
				disabled = function(): boolean
					return qi >= count
				end,
				on_click = function()
					go_to(qi + 1)
				end,
			},
		}
		if q.hint and q.hint ~= "" then
			table.insert(extra, ghost {
				class = "quiz-hint-toggle",
				label = locale.s("ds.quiz.hint"),
				icon = Icon { name = "lightbulb-minimalistic", size = 13 },
				on = function(): boolean
					read()
					return S.hint[qi] == true
				end,
				on_click = function()
					S.hint[qi] = not S.hint[qi] or nil
					changed()
				end,
			})
		end
		return ui.column {
			class = "quiz-question",
			w_full = true,
			min_w = 0,
			head(function(): string
				return locale.s("ds.quiz.question", { i = qi, n = count })
			end, extra),
			ui.column {
				w_full = true,
				mt = "$space.2",
				mb = "$space.1",
				body(q.question, "$font.weight.semibold"),
			},
			if q.hint and q.hint ~= ""
				then ui.show(function(): boolean
					read()
					return S.hint[qi] == true
				end, ui.column { class = "quiz-hint", w_full = true, pt = ui.raw(2, "a hint under its question, as the design's"), pb = ui.raw(6, "a hint under its question, as the design's"), small(q.hint) })
				else nil,
			ui.column {
				class = "quiz-options",
				role = if multi then "group" else "radiogroup",
				label = q.question,
				self = "stretch",
				mt = "$space.1",
				mx = ui.raw(-8, "options' washes hang past their words, as the design's"),
				gap = ui.raw(2, "between options, as the design's"),
				options,
			},
			ui.row {
				class = "quiz-foot",
				w_full = true,
				min_h = 26,
				mt = ui.raw(10, "the quiz's foot, as the design's"),
				gap = "$space.2",
				items = "center",
				ui.text {
					function(): string
						read()
						local g = graded(qi)
						if g == nil then
							return locale.s(if multi then "ds.quiz.choose_all" else "ds.quiz.choose_one")
						elseif g then
							return locale.s("ds.quiz.correct")
						end
						return locale.s(if multi then "ds.quiz.not_all" else "ds.quiz.not_quite")
					end,
					class = "quiz-result",
					literal = true,
					wrap = true,
					grow = 1,
					min_w = 0,
					text_size = "$zeron.visual.text.sm.size",
					line_height = "$zeron.visual.text.sm.line",
					color = function(): string
						read()
						local g = graded(qi)
						return if g == nil then "$text.tertiary" elseif g then "$success" else "$danger"
					end,
				},
				ui.show(
					function(): boolean
						return not checked()
					end,
					Button {
						class = "quiz-check",
						tone = "primary",
						size = "sm",
						disabled = function(): boolean
							return not any_chosen()
						end,
						on_click = function()
							if S.checked[qi] or next(S.chosen[qi] or {}) == nil then
								return
							end
							S.checked[qi] = true
							changed(true)
						end,
						ui.text { locale.s("ds.quiz.check"), literal = true },
					}
				),
				ui.show(
					checked,
					Button {
						class = "quiz-go",
						tone = "primary",
						size = "sm",
						on_click = function()
							local open = first_open()
							if S.done or open == nil then
								S.view, S.done = "sum", true
							else
								S.i = if qi < count then qi + 1 else open
							end
							changed(true)
						end,
						ui.text {
							function(): string
								read()
								if S.done then
									return locale.s("ds.quiz.back_to_results")
								elseif first_open() == nil then
									return locale.s("ds.quiz.see_results")
								end
								return locale.s(if qi >= count then "ds.quiz.to_unanswered" else "ds.quiz.next")
							end,
							literal = true,
						},
					}
				),
			},
		}
	end

	local function essay(): Node?
		local e = props.essay
		if not e or not e.question or e.question == "" then
			return nil
		end
		local typed = ui.use_state("")
		local field = ui.ref()
		local function go()
			read()
			if S.sent then
				return
			end
			local words = string.match(typed:peek(), "^%s*(.-)%s*$") or ""
			if words == "" then
				pcall(function()
					(field :: any):focus()
				end)
				return
			end
			if send(`{e.question}\n\n{words}`) then
				S.sent = true
				changed(true)
			end
		end
		local function sent(): boolean
			read()
			return S.sent or props.on_send == nil
		end
		return ui.column {
			class = "quiz-essay",
			w_full = true,
			mt = ui.raw(14, "the written answer under the results, as the design's"),
			gap = ui.raw(6, "the written answer under the results, as the design's"),
			ui.div { w_full = true, h = 1, shrink = 0, bg = "$border", mb = ui.raw(6, "the written answer under its rule: 12 in all, as the design's") },
			small(e.question, "$text.primary"),
			ui.row {
				w_full = true,
				gap = "$space.2",
				items = "center",
				TextField {
					value = typed,
					placeholder = e.placeholder or "",
					name = locale.s("ds.quiz.answer"),
					size = "sm",
					grow = 1,
					input_ref = field,
					disabled = sent,
					on_key = function(ev: any): boolean
						if ev.key == "Enter" then
							go()
							return true
						end
						return false
					end,
				},
				Button {
					class = "quiz-essay-send",
					tone = "secondary",
					size = "sm",
					disabled = sent,
					on_click = go,
					ui.text {
						function(): string
							read()
							return locale.s(if S.sent then "ds.quiz.sent" else "ds.quiz.send")
						end,
						literal = true,
					},
				},
			},
		}
	end

	local function summary(): Node
		local function score(): number
			read()
			local n = 0
			for qi = 1, count do
				if graded(qi) then
					n += 1
				end
			end
			return n
		end
		local function wrong(): { number }
			read()
			local out = {}
			for qi = 1, count do
				if not graded(qi) then
					table.insert(out, qi)
				end
			end
			return out
		end
		local rows: { Node } = {}
		for qi, q in questions do
			table.insert(rows, ui.row {
				class = "quiz-review",
				role = "button",
				label = `{qi}. {q.question}`,
				focusable = true,
				cursor = "pointer",
				w_full = true,
				min_w = 0,
				gap = ui.raw(10, "an option's mark and words, as the design's"),
				items = "start",
				px = "$space.2",
				py = ui.raw(6, "a reviewed question's room, as the design's"),
				rounded = ui.raw(10, "an option's wash, as the design's"),
				hover = { bg = "$hover" },
				on_click = function()
					S.view, S.i = "q", qi
					changed(true)
				end,
				ui.presence(function(): string
					return if graded(qi) then "ok" else "bad"
				end, function(g: string): Node
					return ui.row {
						w = 20,
						h = 20,
						shrink = 0,
						mt = ui.raw(1, "a mark level with its words' first line, as the design's"),
						items = "center",
						justify = "center",
						rounded = "$radius.full",
						bg = if g == "ok" then "$success" else "$danger",
						Icon { name = if g == "ok" then "check" else "close", size = 12, color = "$zeron.destructive.text" },
					}
				end),
				ui.column {
					grow = 1,
					min_w = 0,
					body(`{qi}. {q.question}`),
					small(function(): string
						read()
						local picks = {}
						for j, o in q.options do
							if (S.chosen[qi] or {})[j] then
								table.insert(picks, o.text)
							end
						end
						return locale.s("ds.quiz.you_chose", { choices = table.concat(picks, ", ") })
					end),
				},
			})
		end
		return ui.column {
			class = "quiz-summary",
			w_full = true,
			min_w = 0,
			head(function(): string
				return locale.s("ds.quiz.results")
			end),
			ui.row {
				mt = ui.raw(10, "the score under the results' head, as the design's"),
				mb = ui.raw(2, "the score under the results' head, as the design's"),
				gap = ui.raw(36, "the score's two figures, as the design's"),
				Stat {
					value = function(): string
						return `{score()}/{count}`
					end,
					label = locale.s("ds.quiz.correct_count"),
				},
				Stat {
					value = function(): string
						return `{math.round(score() / math.max(count, 1) * 100)}%`
					end,
					label = locale.s("ds.quiz.score"),
				},
			},
			ui.column {
				class = "quiz-reviews",
				self = "stretch",
				mt = ui.raw(10, "the reviewed questions under the score, as the design's"),
				mx = ui.raw(-8, "rows' washes hang past their words, as the design's"),
				gap = ui.raw(2, "between options, as the design's"),
				rows,
			},
			ui.row {
				class = "quiz-foot",
				w_full = true,
				min_h = 26,
				mt = ui.raw(10, "the quiz's foot, as the design's"),
				gap = "$space.2",
				items = "center",
				Button {
					class = "quiz-retry",
					tone = "secondary",
					size = "sm",
					on_click = function()
						S.i, S.view, S.done, S.asked, S.sent = 1, "q", false, false, false
						S.chosen, S.checked, S.hint = {}, {}, {}
						changed(true)
					end,
					ui.text { locale.s("ds.quiz.try_again"), literal = true },
				},
				ui.div { grow = 1 },
				Button {
					class = "quiz-ask",
					tone = "primary",
					size = "sm",
					disabled = function(): boolean
						read()
						return S.asked or props.on_send == nil
					end,
					on_click = function()
						if S.asked then
							return
						end
						local missed = wrong()
						local text = if #missed > 0
							then `Explain the quiz questions I got wrong: {table.concat(missed, ", ")}.`
							else "Give me 5 harder questions on this."
						if send(text) then
							S.asked = true
							changed(true)
						end
					end,
					ui.text {
						function(): string
							read()
							if S.asked then
								return locale.s("ds.quiz.asked")
							end
							local missed = wrong()
							return if #missed > 0
								then locale.s("ds.quiz.explain", { list = table.concat(missed, ", ") })
								else locale.s("ds.quiz.harder")
						end,
						literal = true,
					},
				},
			},
			essay(),
		}
	end

	return ui.column {
		class = "visual-quiz",
		w_full = true,
		min_w = 0,
		ui.presence(function(): string
			read()
			return if S.view == "sum" then "sum" else `q{S.i}`
		end, function(key: string): Node
			if key == "sum" then
				return summary()
			end
			return question(tonumber(string.sub(key, 2)) or 1)
		end, { w_full = true }),
	}
end)
