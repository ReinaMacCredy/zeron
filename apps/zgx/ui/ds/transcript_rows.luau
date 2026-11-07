-- gpui's transcript rows that aren't prose (input_chip, secret_card.rs, claude_cards.rs, summary_row), each found by the chat's find.

local Icon = require("@self/icon")
local TextField = require("@self/text_field")
local control = require("@self/control")
local status = require("@self/status")
local locale = require("@self/locale")
local look = require("@self/look")
local MarkdownFind = require("@self/markdown_find")

local M = {}

local function s(key: string, args: { [string]: any }?): string
	return locale.s(`ds.transcript_rows.{key}`, args)
end

function M.line_words(x: any): string
	local title = x.title or ""
	if x.line == "resumed" then
		return locale.s("ds.transcript.resumed")
	elseif x.line == "fork" then
		return locale.s("ds.transcript.forked_from", { title = title })
	elseif x.line == "handoff" then
		return if x.title then locale.s("ds.transcript.continued_from", { title = title }) else locale.s("ds.transcript.context_handoff")
	elseif x.line == "switch" then
		return locale.s("ds.transcript.switched", { from = x.from or "?", to = x.to or "?" })
	end
	return x.text or ""
end

local function answer_words(a: any): string
	return `{a.label} → {a.answer}`
end

local function question_value(b: any): string?
	if b.outcome == "pending" then
		return s("awaiting")
	elseif b.outcome == "unrecorded" then
		return b.text
	end
	local answers = b.answers or {}
	return if #answers == 1 then answer_words(answers[1]) else nil
end

local function secret_label(st: string?): string
	return s(`secret.status.{st or "pending"}`)
end

-- What a find reads of a row, by the key each text is marked under (screens/chat/find reads it too).
function M.texts(m: any, b: any?): { { key: string, text: string } }
	local out: { { key: string, text: string } } = {}
	local x = b or m
	local base = if b then `{m.id}:{b.id}` else `{m.id}:row`
	if x.line == "input" then
		local v = question_value(x)
		if v then
			table.insert(out, { key = `{base}:0`, text = v })
		end
		local answers = x.answers or {}
		if #answers > 1 then
			for i, a in answers do
				table.insert(out, { key = `{base}:{i}`, text = answer_words(a) })
			end
		end
	elseif x.line == "secret" then
		table.insert(out, { key = `{base}:0`, text = secret_label(x.status) })
		if x.status == "pending" then
			table.insert(out, { key = `{base}:1`, text = x.title or "" })
			table.insert(out, { key = `{base}:2`, text = x.reason or "" })
		end
	elseif x.line == "claude_command" and x.card then
		local c = x.card
		table.insert(out, { key = `{base}:0`, text = c.command or "" })
		if c.error or (c.text and c.text ~= "") then
			table.insert(out, { key = `{base}:1`, text = c.error or c.text })
		end
	elseif x.line == "error" then
		table.insert(out, { key = `{base}:0`, text = x.text or "" })
	elseif x.line == "claude_config" then
		table.insert(out, { key = `{base}:0`, text = x.text or "" })
	elseif x.line and x.line ~= "classifier" and x.line ~= "compaction" then
		table.insert(out, { key = `{base}:0`, text = M.line_words(x) })
	end
	return out
end

function M.plain_block(text: string): MarkdownBlock
	return { t = "p", s = { { x = text } } } :: any
end

export type Find = (key: string) -> any

local function found(props: { key: string, text: () -> string, find: Find?, [string]: any }): Node
	local f = if props.find then props.find(props.key) else nil
	local ref = ui.ref()
	local style: { [string]: any } = {}
	for k, v in props :: any do
		if k ~= "key" and k ~= "text" and k ~= "find" then
			style[k] = v
		end
	end
	local function now(): (string, string, number)
		local text = props.text()
		if not f then
			return text, "", 0
		end
		local q = string.lower(f.query())
		local c = f.current()
		return text, q, if c and c.key == props.key and c.block == 1 then c.k else 0
	end
	return ui.presence(function(): string
		local text, q, cur = now()
		return `{text}\0{q}\0{cur}`
	end, function(): Node
		local text, q, cur = now()
		local out: { [any]: any } = { ref = ref, literal = true }
		for k, v in style do
			out[k] = v
		end
		local at = 1
		for i, m in MarkdownFind.occurrences(text, q) do
			if m[1] > at then
				table.insert(out, ui.span { string.sub(text, at, m[1] - 1), literal = true })
			end
			table.insert(out, ui.span { string.sub(text, m[1], m[2]), literal = true, bg = if i == cur then "$zeron.editor.match_current" else "$zeron.editor.match" })
			at = m[2] + 1
		end
		if at <= #text then
			table.insert(out, ui.span { string.sub(text, at), literal = true })
		end
		if cur > 0 and f and f.reveal then
			ui.after(0, function()
				f.reveal(ref)
			end)
		end
		return ui.rich(out)
	end, { min_w = 0, shrink = 1 })
end
M.Found = found

local function ink(a: number): string
	local dark = string.find(ui.theme.kind(), "dark", 1, true) ~= nil
	local base = if dark then "#ffffff" else "#000000"
	return string.format("%s%02x", base, math.floor(a * 255 + 0.5))
end

M.Question = ui.component(function(props: { key: string, now: () -> any, find: Find? }): Node
	local answers_ = ui.computed(function(): { any }
		local b = props.now()
		return if (b.answers and #b.answers > 1) then b.answers else {}
	end)
	local function answers(): { any }
		return answers_:get()
	end
	local function value_row(a: any, key: string): Node
		local quiet = a.skipped == true
		return ui.row {
			min_w = 0,
			grow = 1,
			gap = ui.raw(6, "gpui's answer line"),
			items = "center",
			found { key = key, text = function(): string
				return answer_words(a)
			end, find = props.find, truncate = true, color = if quiet then "$text.tertiary" else "$text.primary" },
		}
	end
	return ui.column {
		class = "transcript-question",
		w_full = true,
		py = ui.raw(4, "gpui's input chip"),
		ui.column {
			w_full = true,
			overflow = "hidden",
			rounded = ui.raw(10, "gpui's input chip"),
			border = 1,
			border_color = "$border.subtle",
			bg = "$surface.raised",
			px = ui.raw(8, "gpui's input chip"),
			pb = function(): any
				return if #answers() > 0 then ui.raw(6, "gpui's input chip") else 0
			end,
			text_size = ui.raw(12, "gpui's input chip"),
			selectable = false,
			ui.row {
				h = ui.raw(34, "gpui's input chip"),
				w_full = true,
				gap = ui.raw(8, "gpui's input chip"),
				items = "center",
				overflow = "hidden",
				ui.row {
					w = 20,
					h = 20,
					shrink = 0,
					items = "center",
					justify = "center",
					rounded = ui.raw(6, "gpui's input chip"),
					bg = "$hover",
					color = "$text.secondary",
					aria_hidden = true,
					Icon { name = "chat-round-line", size = 12 },
				},
				ui.text {
					function(): string
						local n = #answers()
						return if n > 0 then s("questions", { n = n }) else s("question")
					end,
					shrink = 0,
					weight = "$font.weight.medium",
					color = "$text.secondary",
				},
				found {
					key = `{props.key}:0`,
					text = function(): string
						return question_value(props.now()) or ""
					end,
					find = props.find,
					truncate = true,
					color = "$text.primary",
				},
			},
			ui.each(answers_, function(a: any, i: number): Node
				return ui.row {
					h = ui.raw(22, "gpui's answer line"),
					w_full = true,
					pl = ui.raw(28, "gpui's answer line"),
					items = "center",
					value_row(a, `{props.key}:{i}`),
				}
			end),
		},
	}
end)

M.Error = ui.component(function(props: { key: string, now: () -> any, find: Find?, on_sign_in: (() -> ())? }): Node
	local function problem(): string?
		return props.now().problem
	end
	return ui.column {
		class = "transcript-error",
		w_full = true,
		py = ui.raw(4, "gpui's error chip"),
		ui.column {
			w_full = true,
			overflow = "hidden",
			rounded = ui.raw(10, "gpui's error chip"),
			border = 1,
			border_color = "$zeron.tone.danger.edge",
			bg = "$surface.raised",
			px = ui.raw(8, "gpui's error chip"),
			pb = ui.raw(8, "gpui's error chip"),
			text_size = ui.raw(12, "gpui's error chip"),
			ui.row {
				h = ui.raw(34, "gpui's error chip"),
				w_full = true,
				gap = ui.raw(8, "gpui's error chip"),
				items = "center",
				selectable = false,
				ui.row {
					w = 20,
					h = 20,
					shrink = 0,
					items = "center",
					justify = "center",
					rounded = ui.raw(6, "gpui's error chip"),
					bg = "$zeron.tone.danger.wash",
					color = "$danger",
					aria_hidden = true,
					Icon { name = "danger-triangle", size = 12 },
				},
				ui.text {
					function(): string
						local p = problem()
						return if p == "signed_out" then s("error.signed_out") elseif p == "limit" then s("error.limit") else s("error.label")
					end,
					grow = 1,
					weight = "$font.weight.medium",
					color = "$danger",
				},
				ui.show(
					function(): boolean
						return problem() ~= nil and props.on_sign_in ~= nil
					end,
					ui.button {
						id = `error-sign-in-{props.key}`,
						label = s("error.log_in"),
						h = 24,
						px = ui.raw(10, "gpui's error chip"),
						shrink = 0,
						rounded = ui.raw(6, "gpui's error chip"),
						bg = "$zeron.tone.danger.wash",
						hover = { bg = "$zeron.tone.danger.edge" },
						active = { bg = "$zeron.tone.danger.edge" },
						color = "$danger",
						weight = "$font.weight.medium",
						focus = control.ring,
						on_click = function()
							if props.on_sign_in then
								props.on_sign_in()
							end
						end,
						ui.text { s("error.log_in") },
					}
				),
			},
			found { key = `{props.key}:0`, text = function(): string
				return props.now().text or ""
			end, find = props.find, wrap = true, color = "$text.primary" },
		},
	}
end)

local function event_row(icon: string, words: () -> string, tone: () -> string, key: string, find: Find?): Node
	return ui.row {
		h = ui.raw(26, "gpui's event row"),
		w_full = true,
		gap = ui.raw(8, "gpui's event row"),
		items = "center",
		text_size = ui.raw(12.5, "gpui's event row"),
		color = tone,
		selectable = false,
		Icon { name = icon :: any, size = 13 },
		found { key = key, text = words, find = find, truncate = true },
	}
end

export type SecretProps = { key: string, now: () -> any, find: Find?, on_answer: ((request: string, secret: string?) -> any)? }
M.Secret = ui.component(function(props: SecretProps): Node
	local value = ui.use_state("")
	local busy = ui.use_state(false)
	local sent = ui.use_state(nil :: string?)
	local failure = ui.use_state(nil :: string?)
	local function shown(): string
		return sent:get() or props.now().status or "pending"
	end
	local function answer(save: boolean)
		local b = props.now()
		local secret = if save then string.match(value:peek(), "^%s*(.-)%s*$") else nil
		if busy:peek() or sent:peek() or not props.on_answer or not b.request or (save and secret == "") then
			return
		end
		busy:set(true)
		failure:set(nil)
		local p = props.on_answer(b.request, secret)
		local function done(ok: boolean, why: string?)
			busy:set(false)
			if ok then
				sent:set(if save then "saved" else "declined")
				value:set("")
			else
				failure:set(why or s("secret.failed"))
			end
		end
		if p and p.after then
			p:after(function()
				done(true)
			end, function(e: any)
				done(false, if type(e) == "table" and e.message == "offline" then s("secret.offline") else nil)
			end)
		else
			done(true)
		end
	end
	return ui.presence(function(): string
		return if shown() == "pending" then "card" else "row"
	end, function(kind: string): Node
		if kind == "row" then
			return ui.column {
				class = "transcript-secret",
				w_full = true,
				py = ui.raw(4, "gpui's secret card"),
				event_row("shield-check", function(): string
					return secret_label(shown())
				end, function(): string
					return if shown() == "saved" then "$success" else "$info"
				end, `{props.key}:0`, props.find),
			}
		end
		local disabled = function(): boolean
			return busy:get() or string.match(value:get(), "%S") == nil
		end
		return ui.column {
			class = "transcript-secret",
			w_full = true,
			py = ui.raw(4, "gpui's secret card"),
			gap = ui.raw(6, "gpui's secret card"),
			event_row("shield-check", function(): string
				return secret_label("pending")
			end, function(): string
				return "$info"
			end, `{props.key}:0`, props.find),
			ui.column {
				id = `secret-card-{props.key}`,
				role = "group",
				label = function(): string
					return props.now().title or ""
				end,
				w_full = true,
				gap = ui.raw(12, "gpui's secret card"),
				rounded = ui.raw(14, "gpui's secret card"),
				border = 1,
				border_color = "$border.subtle",
				bg = "$surface.raised",
				p = ui.raw(16, "gpui's secret card"),
				ui.column {
					gap = ui.raw(4, "gpui's secret card"),
					found { key = `{props.key}:1`, text = function(): string
						return props.now().title or ""
					end, find = props.find, text_size = ui.raw(14, "gpui's secret card"), line_height = ui.raw(20, "gpui's secret card"), weight = "$font.weight.semibold", color = "$text.primary", wrap = true },
					found { key = `{props.key}:2`, text = function(): string
						return props.now().reason or ""
					end, find = props.find, text_size = ui.raw(13, "gpui's secret card"), line_height = ui.raw(19, "gpui's secret card"), color = "$text.secondary", wrap = true },
				},
				ui.row {
					w_full = true,
					gap = ui.raw(8, "gpui's secret card"),
					items = "center",
					ui.div {
						grow = 1,
						min_w = 0,
						TextField {
							value = value,
							password = true,
							w_full = true,
							name = s("secret.field"),
							placeholder = function(): string
								return props.now().placeholder or s("secret.placeholder")
							end,
							on_commit = function()
								answer(true)
							end,
						},
					},
					ui.button {
						id = `secret-save-{props.key}`,
						label = s("secret.save"),
						h = ui.raw(34, "gpui's secret card"),
						px = ui.raw(14, "gpui's secret card"),
						shrink = 0,
						rounded = ui.raw(8, "gpui's secret card"),
						bg = "$zeron.primary.bg",
						hover = { bg = "$zeron.primary.hover" },
						active = { bg = "$zeron.primary.active" },
						color = "$zeron.primary.text",
						text_size = ui.raw(13, "gpui's secret card"),
						weight = "$font.weight.medium",
						focus = control.ring,
						disabled = disabled,
						opacity = function(): number
							return if disabled() then 0.5 else 1
						end,
						on_click = function()
							answer(true)
						end,
						ui.text { s("secret.save") },
					},
				},
				ui.row {
					w_full = true,
					gap = ui.raw(12, "gpui's secret card"),
					items = "center",
					text_size = "$font.size.sm",
					ui.row {
						grow = 1,
						min_w = 0,
						gap = ui.raw(6, "gpui's secret card"),
						items = "center",
						color = function(): string
							return if failure:get() then "$warning" else "$text.secondary"
						end,
						Icon { name = "shield-check", size = 13 },
						ui.text {
							function(): string
								return failure:get() or s("secret.kept_out")
							end,
							wrap = true,
							min_w = 0,
						},
					},
					ui.button {
						id = `secret-decline-{props.key}`,
						label = s("secret.decline"),
						h = 24,
						px = ui.raw(6, "gpui's secret card"),
						shrink = 0,
						rounded = ui.raw(6, "gpui's secret card"),
						bg = "#00000000",
						hover = { bg = "#00000000", color = "$text.primary" },
						active = { bg = "#00000000" },
						color = "$text.secondary",
						weight = "$font.weight.medium",
						text_size = ui.raw(12.5, "gpui's secret card"),
						focus = control.ring,
						disabled = function(): boolean
							return busy:get()
						end,
						on_click = function()
							answer(false)
						end,
						ui.text { s("secret.decline") },
					},
				},
			},
		}
	end, { w_full = true })
end)

local function glyph_button(id: string, icon: () -> string, label: () -> string, tooltip: string, on_click: () -> ()): Node
	return ui.button {
		id = id,
		label = label,
		tooltip = tooltip,
		w = 26,
		h = 26,
		p = 0,
		shrink = 0,
		items = "center",
		justify = "center",
		rounded = ui.raw(6, "gpui's card header"),
		bg = "#00000000",
		hover = { bg = "$hover" },
		active = { bg = "$hover" },
		color = "$text.secondary",
		focus = control.ring,
		on_click = on_click,
		Icon { name = icon :: any, size = 13 },
	}
end

local function shell(key: string, children: { Node }): Node
	local out: { [any]: any } = {
		id = `claude-card-{key}`,
		class = "claude-card",
		w_full = true,
		overflow = "hidden",
		rounded = ui.raw(10, "gpui's Claude card"),
		border = 1,
		border_color = "$border.subtle",
		bg = "$surface.raised",
		text_size = "$font.size.md",
		color = "$text.primary",
	}
	for _, c in children do
		table.insert(out, c)
	end
	return ui.column(out)
end

local function pending_card(key: string, now: () -> any): Node
	local clock = ui.use_state(os.time())
	ui.every(1000, function()
		clock:set(os.time())
	end)
	return shell(key, {
		ui.row {
			px = ui.raw(12, "gpui's Claude card"),
			py = ui.raw(10, "gpui's Claude card"),
			gap = ui.raw(8, "gpui's Claude card"),
			items = "center",
			selectable = false,
			status.StatusIcon { kind = "working", size = 12 },
			ui.text { function(): string
				return now().card.command or ""
			end, literal = true, shrink = 0, font = "$font.mono" },
			ui.text { s("claude.running"), truncate = true, min_w = 0, text_size = ui.raw(12.5, "gpui's Claude card"), color = "$text.secondary" },
			ui.div { grow = 1 },
			ui.text { function(): string
				local started = now().card.started
				local secs = if started then math.max(0, clock:get() - math.floor(started / 1000)) else 0
				return s("claude.elapsed", { n = secs })
			end, tabular = true, shrink = 0, text_size = "$font.size.sm", color = "$text.tertiary" },
		},
	})
end

local function notice(key: string, now: () -> any, find: Find?): Node
	return ui.row {
		id = `claude-card-{key}`,
		class = "claude-card-notice",
		w_full = true,
		gap = ui.raw(10, "gpui's terminal notice"),
		items = "center",
		px = ui.raw(12, "gpui's terminal notice"),
		py = ui.raw(8, "gpui's terminal notice"),
		rounded = ui.raw(10, "gpui's terminal notice"),
		border = 1,
		border_style = "dashed",
		border_color = "$border.strong",
		text_size = ui.raw(12.5, "gpui's terminal notice"),
		color = "$text.secondary",
		Icon { name = "terminal", size = 15 },
		ui.rich {
			wrap = true,
			grow = 1,
			min_w = 0,
			ui.span { function(): string
				return `/{now().card.terminal or ""}`
			end, literal = true, color = "$text.primary", font = look.code_family() },
			ui.span { s("claude.terminal_only") },
		},
	}
end

local function generic_card(key: string, now: () -> any, find: Find?, on_retry: ((command: string) -> ())?, stale: () -> boolean): Node
	local copied = ui.use_state(false)
	local function body(): string
		local c = now().card
		if stale() then
			return s("claude.stale")
		end
		return c.error or c.text or ""
	end
	local function failed(): boolean
		return now().card.error ~= nil or stale()
	end
	return shell(key, {
		ui.row {
			px = ui.raw(12, "gpui's Claude card"),
			py = ui.raw(7, "gpui's Claude card"),
			gap = ui.raw(8, "gpui's Claude card"),
			items = "center",
			border_sides = { 0, 0, 1, 0 },
			border_color = "$border.subtle",
			selectable = false,
			ui.div { color = "$text.secondary", shrink = 0, Icon { name = "terminal", size = 15 } },
			found { key = `{key}:0`, text = function(): string
				return now().card.command or ""
			end, find = find, weight = "$font.weight.semibold", shrink = 0 },
			ui.text { s("claude.output"), truncate = true, min_w = 0, text_size = ui.raw(12.5, "gpui's Claude card"), color = "$text.secondary" },
			ui.div { grow = 1 },
			glyph_button(`claude-card-copy-{key}`, function(): string
				return if copied:get() then "check" else "copy"
			end, function(): string
				return if copied:get() then s("claude.copied") else s("claude.copy")
			end, s("claude.copy"), function()
				ui.clipboard.write(body())
				copied:set(true)
				ui.after(1200, function()
					copied:set(false)
				end)
			end),
		},
		ui.scroll {
			w_full = true,
			max_h = ui.raw(260, "gpui's Claude card"),
			ui.column {
				px = ui.raw(12, "gpui's Claude card"),
				py = ui.raw(10, "gpui's Claude card"),
				found { key = `{key}:1`, text = body, find = find, wrap = true, font = "$font.mono", text_size = "$font.size.sm", color = function(): string
					return if failed() then "$warning" else "$text.primary"
				end },
			},
		},
		ui.presence(function(): string?
			if failed() then
				return if on_retry then "retry" else nil
			end
			return "note"
		end, function(kind: string): Node
			if kind == "retry" then
				return ui.row {
					px = ui.raw(12, "gpui's Claude card"),
					py = ui.raw(9, "gpui's Claude card"),
					gap = ui.raw(6, "gpui's Claude card"),
					border_sides = { 1, 0, 0, 0 },
					border_color = "$border.subtle",
					ui.button {
						id = `claude-card-retry-{key}`,
						label = s("claude.retry"),
						h = 26,
						px = ui.raw(10, "gpui's Claude card"),
						rounded = ui.raw(7, "gpui's Claude card"),
						bg = "$zeron.tone.accent.wash",
						hover = { bg = "$hover" },
						active = { bg = "$hover" },
						color = "$accent",
						text_size = ui.raw(12.5, "gpui's Claude card"),
						focus = control.ring,
						on_click = function()
							if on_retry then
								on_retry(now().card.command)
							end
						end,
						ui.text { s("claude.retry") },
					},
				}
			end
			return ui.text {
				s("claude.local"),
				w_full = true,
				px = ui.raw(12, "gpui's Claude card"),
				py = ui.raw(8, "gpui's Claude card"),
				border_sides = { 1, 0, 0, 0 },
				border_color = "$border.subtle",
				text_size = ui.raw(11.5, "gpui's Claude card"),
				color = "$text.tertiary",
				selectable = false,
			}
		end, { w_full = true }),
	})
end

-- (The chat's model as it is now wins over the one the card was answered under.)
local function model_card(key: string, now: () -> any, current: (() -> string?)?, on_model: ((model: string) -> ())?): Node
	local function selected(): number?
		local c = now().card
		local want = (if current then current() else nil) or c.model
		if not want then
			return nil
		end
		local alias = string.lower(want)
		for i, m in c.models or {} do
			if m.id == want then
				return i
			end
		end
		for i, m in c.models or {} do
			if string.sub(string.lower(m.label), 1, #alias) == alias then
				return i
			end
		end
		return nil
	end
	local models = ui.computed(function(): { any }
		return now().card.models or {}
	end)
	return shell(key, {
		ui.row {
			px = ui.raw(12, "gpui's Claude card"),
			py = ui.raw(7, "gpui's Claude card"),
			gap = ui.raw(8, "gpui's Claude card"),
			items = "center",
			border_sides = { 0, 0, 1, 0 },
			border_color = "$border.subtle",
			selectable = false,
			ui.div { color = "$text.secondary", shrink = 0, Icon { name = "fast-tier", size = 15 } },
			ui.text { s("claude.model"), weight = "$font.weight.semibold", shrink = 0 },
			ui.text { s("claude.claude_code"), truncate = true, min_w = 0, text_size = ui.raw(12.5, "gpui's Claude card"), color = "$text.secondary" },
			ui.div { grow = 1 },
			ui.text {
				function(): string
					local i = selected()
					local c = now().card
					return if i then c.models[i].label else ((if current then current() else nil) or c.model or s("claude.default"))
				end,
				literal = true,
				shrink = 0,
				px = ui.raw(7, "gpui's Claude card pill"),
				rounded = "$radius.full",
				border = 1,
				border_color = "$border.subtle",
				text_size = ui.raw(11.5, "gpui's Claude card pill"),
				color = "$text.secondary",
			},
		},
		ui.column {
			p = ui.raw(12, "gpui's Claude card"),
			gap = ui.raw(8, "gpui's Claude card"),
			ui.row {
				wrap = true,
				gap = ui.raw(6, "gpui's Claude card"),
				ui.each(models, function(m: any, i: number): Node
					local function on(): boolean
						return selected() == i
					end
					return ui.button {
						id = `claude-card-model-{key}-{i}`,
						label = m.label,
						checked = on,
						px = ui.raw(10, "gpui's model chip"),
						py = ui.raw(3, "gpui's model chip"),
						h = "auto" :: any,
						rounded = "$radius.full",
						border = 1,
						border_color = function(): string
							return if on() then "$accent" else "$border.subtle"
						end,
						bg = "#00000000",
						hover = { bg = "$hover" },
						active = { bg = "$hover" },
						color = function(): string
							return if on() then "$accent" else "$text.primary"
						end,
						text_size = ui.raw(12.5, "gpui's model chip"),
						focus = control.ring,
						on_click = function()
							if on_model then
								on_model(m.id)
							end
						end,
						ui.text { m.label, literal = true },
					}
				end),
			},
			ui.text {
				function(): string
					return now().card.note or s("claude.model_note")
				end,
				wrap = true,
				text_size = ui.raw(11.5, "gpui's Claude card"),
				color = "$text.tertiary",
			},
		},
	})
end

-- (Pending past gpui's STALE_PENDING_MS: no answer came, the card says so.)
M.ClaudeCard = ui.component(function(props: { key: string, now: () -> any, find: Find?, on_retry: ((command: string) -> ())?, current_model: (() -> string?)?, on_model: ((model: string) -> ())? }): Node
	local now = props.now
	local clock = ui.use_state(os.time())
	local function stale(): boolean
		local c = now().card
		if not c.pending or not c.started then
			return false
		end
		return clock:get() * 1000 - c.started >= (c.stale_ms or 120000)
	end
	local c0 = now().card
	if c0.pending and c0.started then
		local left = (c0.stale_ms or 120000) - (os.time() * 1000 - c0.started)
		if left > 0 then
			ui.after(left + 50, function()
				clock:set(os.time())
			end)
		end
	end
	return ui.presence(function(): string
		local c = now().card
		if c.terminal then
			return "notice"
		end
		if c.pending and not stale() then
			return "pending"
		end
		return if c.command == "/model" and not c.error and not c.pending then "model" else "card"
	end, function(kind: string): Node
		if kind == "notice" then
			return notice(props.key, now, props.find)
		elseif kind == "pending" then
			return pending_card(props.key, now)
		elseif kind == "model" then
			return model_card(props.key, now, props.current_model, props.on_model)
		end
		return generic_card(props.key, now, props.find, props.on_retry, stale)
	end, { w_full = true })
end)

M.ConfigSummary = ui.component(function(props: { key: string, now: () -> any, find: Find? }): Node
	return ui.row {
		class = "claude-config-summary",
		w_full = true,
		gap = ui.raw(8, "gpui's summary row"),
		items = "center",
		px = ui.raw(10, "gpui's summary row"),
		py = ui.raw(6, "gpui's summary row"),
		rounded = ui.raw(8, "gpui's summary row"),
		border = 1,
		border_color = "$border.subtle",
		bg = "$surface.raised",
		text_size = ui.raw(12.5, "gpui's summary row"),
		color = "$text.secondary",
		ui.div { shrink = 0, Icon { name = "settings", size = 14 } },
		found { key = `{props.key}:0`, text = function(): string
			return props.now().text or ""
		end, find = props.find, wrap = true, grow = 1 },
	}
end)

M.ink = ink

return M
