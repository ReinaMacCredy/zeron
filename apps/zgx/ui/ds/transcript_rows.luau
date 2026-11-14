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

-- Each request's answer under way or sent, outliving its row (gpui's `SecretCards`): scrolled away and back, a sent card stays sent.
local SECRETS = ui.state("transcript_secrets", {} :: { [string]: { busy: boolean?, sent: string?, failure: string? } })
-- Each computer card's answer under way, sent or refused, outliving its row (gpui's `consent_sent`, `foreground_sent`, `consent_failures`).
local COMPUTER = ui.state("transcript_computer", {} :: { [string]: { busy: boolean?, sent: string?, failure: string? } })

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
		local own = if x.request then SECRETS:peek()[x.request] else nil
		local st = (if own then own.sent else nil) or x.status
		table.insert(out, { key = `{base}:0`, text = secret_label(st) })
		if st == "pending" then
			table.insert(out, { key = `{base}:1`, text = x.title or "" })
			table.insert(out, { key = `{base}:2`, text = x.reason or "" })
		end
	elseif x.line == "claude_command" and x.context then
		-- (Drawn as ds/compaction_context's card, which marks no find.)
	elseif x.line == "claude_command" and x.card then
		-- (As ClaudeCard draws it: the notice's command, a running one's, the model's note, or the answer and its words.)
		local c = x.card
		local stale = c.pending and c.started and os.time() * 1000 - c.started >= (c.stale_ms or 120000)
		if c.terminal then
			table.insert(out, { key = `{base}:0`, text = `/{c.terminal}` })
		elseif c.pending and not stale then
			table.insert(out, { key = `{base}:0`, text = c.command or "" })
		elseif c.command == "/model" and not c.error and not c.pending then
			table.insert(out, { key = `{base}:1`, text = c.note or s("claude.model_note") })
		else
			table.insert(out, { key = `{base}:0`, text = c.command or "" })
			local body = if stale then s("claude.stale") else (c.error or c.text or "")
			if body ~= "" then
				table.insert(out, { key = `{base}:1`, text = body })
			end
		end
	elseif x.line == "computer_consent" or x.line == "computer_foreground" then
		local own = if x.request then COMPUTER:peek()[x.request] else nil
		local settled = x.text or (if own then own.sent else nil)
		if settled then
			table.insert(out, { key = `{base}:0`, text = settled })
		else
			table.insert(out, { key = `{base}:0`, text = x.title or "" })
			if (x.detail or "") ~= "" then
				table.insert(out, { key = `{base}:1`, text = x.detail })
			end
		end
	elseif x.line == "error" then
		table.insert(out, { key = `{base}:0`, text = x.text or "" })
	elseif x.line == "claude_config" then
		table.insert(out, { key = `{base}:0`, text = x.text or "" })
	elseif x.line and x.line ~= "compaction" then
		table.insert(out, { key = `{base}:0`, text = M.line_words(x) })
	end
	return out
end

function M.plain_block(text: string): MarkdownBlock
	return { t = "p", s = { { x = text } } } :: any
end

export type Find = (key: string) -> any

-- (`lead`: the found words' own style in a paragraph that goes on in `tail`, unfound.)
local function found(props: { key: string, text: () -> string, find: Find?, lead: { [string]: any }?, tail: string?, [string]: any }): Node
	local f = if props.find then props.find(props.key) else nil
	local ref = ui.ref()
	local style: { [string]: any } = {}
	for k, v in props :: any do
		if k ~= "key" and k ~= "text" and k ~= "find" and k ~= "lead" and k ~= "tail" then
			style[k] = v
		end
	end
	local function piece(words: string, bg: string?): Node
		local out: { [any]: any } = { words, literal = true, bg = bg }
		for k, v in props.lead or {} do
			out[k] = v
		end
		return ui.span(out)
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
		local hits = MarkdownFind.occurrences(text, q)
		for i, m in hits do
			if m[1] > at then
				table.insert(out, piece(string.sub(text, at, m[1] - 1)))
			end
			table.insert(out, piece(string.sub(text, m[1], m[2]), if i == cur then "$zeron.editor.match_current" else "$zeron.editor.match"))
			at = m[2] + 1
		end
		if at <= #text then
			table.insert(out, piece(string.sub(text, at)))
		end
		if props.tail then
			table.insert(out, ui.span { props.tail })
		end
		-- (What a find marks, by class: a test or a plugin can tell.)
		if #hits > 0 then
			out.class = if cur > 0 then "find-mark find-current" else "find-mark"
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
			border_color = "$border",
			bg = "$zeron.chip.bg",
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
			bg = "$zeron.chip.alert",
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
					color = "$zeron.tone.danger.ink",
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
					color = "$zeron.tone.danger.ink",
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
						color = "$zeron.tone.danger.ink",
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
				-- (Zeron's notice chip: a failure's words are for pasting, not a screenshot.)
				ui.button {
					id = `error-copy-{props.key}`,
					label = s("error.copy"),
					tooltip = locale.s("ds.composer.notice_copy"),
					w = 20,
					h = 20,
					p = 0,
					shrink = 0,
					items = "center",
					justify = "center",
					rounded = ui.raw(6, "gpui's error chip"),
					bg = "#00000000",
					hover = { bg = "$zeron.tone.danger.wash" },
					active = { bg = "$zeron.tone.danger.wash" },
					color = "$zeron.tone.danger.ink",
					focus = control.ring,
					on_click = function()
						ui.clipboard.write(props.now().text or "")
					end,
					Icon { name = "copy", size = 12 },
				},
			},
			found { key = `{props.key}:0`, text = function(): string
				return props.now().text or ""
			end, find = props.find, wrap = true, color = "$zeron.tone.danger.ink" },
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
local function secret_set(request: string, patch: { busy: boolean?, sent: string?, failure: string? })
	local all = table.clone(SECRETS:peek())
	all[request] = patch
	SECRETS:set(all)
end
M.Secret = ui.component(function(props: SecretProps): Node
	local value = ui.use_state("")
	local function mine(): { busy: boolean?, sent: string?, failure: string? }
		local b = props.now()
		return if b.request then SECRETS:get()[b.request] or {} else {}
	end
	local function busy(): boolean
		return mine().busy == true
	end
	local function failure(): string?
		return mine().failure
	end
	local function shown(): string
		return mine().sent or props.now().status or "pending"
	end
	-- (Settled from elsewhere, timed out or declined from the composer: what was typed goes.)
	ui.effect(function()
		if shown() ~= "pending" and value:peek() ~= "" then
			value:set("")
		end
	end)
	local function answer(save: boolean)
		local b = props.now()
		local request = b.request
		local secret = if save then string.match(value:peek(), "^%s*(.-)%s*$") else nil
		local was = if request then SECRETS:peek()[request] or {} else {}
		if was.busy or was.sent or not request or (save and secret == "") then
			return
		end
		-- (No way to answer here, or a host without `chat.answer_secret`: it says so, and the buttons stay.)
		local ok, p = false, nil
		if props.on_answer then
			ok, p = pcall(props.on_answer, request, secret)
		end
		if not ok or type(p) ~= "table" or not p.after then
			secret_set(request, { failure = s("secret.unsupported") })
			return
		end
		secret_set(request, { busy = true })
		p:after(function()
			value:set("")
			secret_set(request, { sent = if save then "saved" else "declined" })
		end, function(e: any)
			secret_set(request, { failure = if type(e) == "table" and e.message == "offline" then s("secret.offline") else s("secret.failed") })
		end)
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
			return busy() or string.match(value:get(), "%S") == nil
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
				border_color = "$border",
				bg = "$zeron.chip.bg",
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
							reveal = false,
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
						bg = "$accent",
						hover = { bg = "$accent.hover" },
						active = { bg = "$zeron.accent.active" },
						color = "$accent.text",
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
							return if failure() then "$warning" else "$text.secondary"
						end,
						Icon { name = "shield-check", size = 13 },
						ui.text {
							function(): string
								return failure() or s("secret.kept_out")
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
						disabled = busy,
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

local function computer_set(request: string, patch: { busy: boolean?, sent: string?, failure: string? })
	local all = table.clone(COMPUTER:peek())
	all[request] = patch
	COMPUTER:set(all)
end

local function tone_ink(tone: string): string
	return if tone == "warning" then "$warning" elseif tone == "success" then "$success" else "$text.secondary"
end

local function card_button(id: string, label: string, primary: boolean, h: number, px: number, disabled: (() -> boolean)?, on_click: () -> ()): Node
	return ui.button {
		id = id,
		label = label,
		h = ui.raw(h, "gpui's computer card"),
		px = ui.raw(px, "gpui's computer card"),
		shrink = 0,
		rounded = ui.raw(8, "gpui's computer card"),
		bg = if primary then "$zeron.primary.bg" else "#00000000",
		hover = if primary then { bg = "$zeron.primary.hover", opacity = 0.9 } else { bg = "$hover" },
		active = if primary then { bg = "$zeron.primary.active" } else { bg = "$pressed" },
		border = if primary then 0 else 1,
		border_color = "$border.subtle",
		color = if primary then "$zeron.primary.text" else "$text.primary",
		weight = "$font.weight.medium",
		focus = control.ring,
		disabled = disabled,
		on_click = on_click,
		ui.text { label },
	}
end

export type ComputerProps = {
	key: string,
	now: () -> any,
	find: Find?,
	on_answer: ((request: string, question: string, answer: string) -> any)?,
	disclosure: (() -> boolean)?,
	on_approved: (() -> ())?,
}
-- gpui's `approval_card`: a consent card (Allow this conversation, Always allow, Deny) or a foreground one (Allow for this turn, No); once answered, the row it settled to.
M.ComputerApproval = ui.component(function(props: ComputerProps): Node
	local function mine(): { busy: boolean?, sent: string?, failure: string? }
		local b = props.now()
		return if b.request then COMPUTER:get()[b.request] or {} else {}
	end
	local function settled(): string?
		return props.now().text or mine().sent
	end
	local function busy(): boolean
		return mine().busy == true or mine().sent ~= nil
	end
	local function answer(c: any)
		local b = props.now()
		local request = b.request
		local was = if request then COMPUTER:peek()[request] or {} else {}
		if not request or was.busy or was.sent then
			return
		end
		local ok, p = false, nil
		if props.on_answer then
			ok, p = pcall(props.on_answer, request, b.question, c.answer)
		end
		if not ok or type(p) ~= "table" or not p.after then
			computer_set(request, { failure = s("computer.unsupported") })
			return
		end
		computer_set(request, { busy = true })
		p:after(function()
			-- (A foreground card stays until the engine's answer turns it into its row.)
			computer_set(request, { busy = true, sent = c.settled })
			if c.key ~= "deny" and b.line == "computer_consent" and props.on_approved then
				props.on_approved()
			end
		end, function(e: any)
			local why = if type(e) == "table" then e.message else e
			computer_set(request, { failure = s("computer.failed", { why = if type(why) == "string" then why else "failed" }) })
		end)
	end
	return ui.presence(function(): string
		return if settled() then "row" else "card"
	end, function(kind: string): Node
		if kind == "row" then
			return ui.column {
				class = "transcript-computer",
				w_full = true,
				event_row("computer-cursor", function(): string
					return settled() or ""
				end, function(): string
					return "$text.secondary"
				end, `{props.key}:0`, props.find),
			}
		end
		local function risk(): string?
			return props.now().risk
		end
		local choices = ui.computed(function(): { any }
			return props.now().choices or {}
		end)
		return ui.column {
			class = "transcript-computer",
			w_full = true,
			py = ui.raw(4, "gpui's approval card"),
			ui.column {
				id = `computer-approval-{props.key}`,
				role = "group",
				label = function(): string
					return props.now().title or ""
				end,
				w_full = true,
				overflow = "hidden",
				rounded = ui.raw(12, "gpui's approval card"),
				border = 1,
				border_color = function(): string
					return if risk() then "$zeron.tone.warning.edge" else "$border.subtle"
				end,
				bg = "$surface.raised",
				text_size = ui.raw(12.5, "gpui's approval card"),
				line_height = ui.raw(18, "gpui's approval card"),
				ui.show(function(): boolean
					return risk() ~= nil
				end, ui.column {
					id = `computer-risk-{props.key}`,
					w_full = true,
					px = ui.raw(12, "gpui's approval card"),
					py = ui.raw(8, "gpui's approval card"),
					gap = ui.raw(2, "gpui's approval card"),
					bg = "$zeron.tone.warning.wash",
					ui.row {
						gap = ui.raw(6, "gpui's approval card"),
						items = "center",
						color = "$warning",
						weight = "$font.weight.semibold",
						Icon { name = "danger-triangle", size = 13 },
						ui.text { s("computer.elevated_risk") },
					},
					ui.text { function(): string
						return risk() or ""
					end, wrap = true, color = "$text.secondary" },
				}),
				ui.row {
					w_full = true,
					p = ui.raw(12, "gpui's approval card"),
					gap = ui.raw(12, "gpui's approval card"),
					items = "center",
					ui.row {
						w = 32,
						h = 32,
						shrink = 0,
						items = "center",
						justify = "center",
						rounded = ui.raw(8, "gpui's approval card"),
						bg = "$hover",
						color = "$text.secondary",
						aria_hidden = true,
						Icon { name = "computer-cursor", size = 16 },
					},
					ui.column {
						grow = 1,
						min_w = 0,
						gap = ui.raw(2, "gpui's approval card"),
						found { key = `{props.key}:0`, text = function(): string
							return props.now().title or ""
						end, find = props.find, text_size = ui.raw(13.5, "gpui's approval card"), weight = "$font.weight.semibold", color = "$text.primary", wrap = true },
						ui.show(function(): boolean
							return (props.now().detail or "") ~= ""
						end, found { key = `{props.key}:1`, text = function(): string
							return props.now().detail or ""
						end, find = props.find, color = "$text.secondary", wrap = true }),
					},
				},
				ui.show(function(): boolean
					return props.now().line == "computer_consent" and (props.disclosure == nil or props.disclosure())
				end, ui.text {
					function(): string
						return props.now().disclosure_text or ""
					end,
					id = `computer-disclosure-{props.key}`,
					wrap = true,
					px = ui.raw(12, "gpui's approval card"),
					pb = ui.raw(10, "gpui's approval card"),
					text_size = ui.raw(11.5, "gpui's approval card"),
					color = "$text.secondary",
				}),
				ui.row {
					w_full = true,
					px = ui.raw(12, "gpui's approval card"),
					pb = ui.raw(12, "gpui's approval card"),
					gap = ui.raw(6, "gpui's approval card"),
					items = "center",
					justify = "end",
					flex_wrap = true,
					ui.show(function(): boolean
						return mine().failure ~= nil
					end, ui.text { function(): string
						return mine().failure or ""
					end, grow = 1, min_w = 0, wrap = true, weight = "$font.weight.medium", color = "$warning" }),
					ui.each(choices, function(c: any): Node
						return card_button(`computer-{c.key}-{props.key}`, c.label, c.primary == true, 28, 12, busy, function()
							answer(c)
						end)
					end),
				},
			},
		}
	end, { w_full = true })
end)

-- gpui's `computer_card`: a tile, its title, the lines under it and an action.
local function computer_card(key: string, tone: string, title: () -> string, lines: () -> { string }, action: { label: string, on_click: () -> () }?, find: Find?): Node
	local accent = tone_ink(tone)
	local list = ui.computed(lines)
	return ui.column {
		class = "transcript-computer",
		w_full = true,
		py = ui.raw(4, "gpui's computer card"),
		ui.row {
			id = `computer-card-{key}`,
			w_full = true,
			gap = ui.raw(10, "gpui's computer card"),
			items = "start",
			overflow = "hidden",
			rounded = ui.raw(10, "gpui's computer card"),
			border = 1,
			border_color = "$border.subtle",
			bg = "$surface.raised",
			p = ui.raw(10, "gpui's computer card"),
			text_size = ui.raw(12, "gpui's computer card"),
			line_height = ui.raw(17, "gpui's computer card"),
			ui.row {
				w = 28,
				h = 28,
				shrink = 0,
				items = "center",
				justify = "center",
				rounded = ui.raw(8, "gpui's computer card"),
				bg = if tone == "info" then "$hover" else `$zeron.tone.{tone}.wash`,
				color = accent,
				aria_hidden = true,
				Icon { name = "computer-cursor", size = 14 },
			},
			ui.column {
				grow = 1,
				min_w = 0,
				gap = ui.raw(2, "gpui's computer card"),
				found { key = `{key}:0`, text = title, find = find, weight = "$font.weight.medium", color = "$text.primary", wrap = true },
				ui.each(list, function(line: string): Node
					return ui.text { line, wrap = true, color = "$text.secondary" }
				end),
				if action
					then ui.row {
						pt = ui.raw(6, "gpui's computer card"),
						gap = ui.raw(6, "gpui's computer card"),
						flex_wrap = true,
						items = "center",
						card_button(`computer-action-{key}`, action.label, false, 26, 10, nil, action.on_click),
					}
					else nil,
			},
		},
	}
end

-- gpui's `render_computer_notice`: Revoked, Paused, Foreground and Left open as one quiet row; Unavailable, Control denied and Setup as a card.
M.ComputerNotice = ui.component(function(props: { key: string, now: () -> any, find: Find?, on_enable: (() -> ())? }): Node
	local b = props.now()
	local kind = b.notice
	local function words(): string
		return props.now().text or ""
	end
	if kind == "revoked" then
		return event_row("stop", words, function(): string
			return "$warning"
		end, `{props.key}:0`, props.find)
	elseif kind == "unavailable" or kind == "setup" or kind == "control_denied" then
		local action = nil
		if kind == "control_denied" and props.on_enable then
			local enable = props.on_enable
			action = { label = b.action or s("computer.use_for_request"), on_click = function()
				enable()
			end }
		end
		return computer_card(props.key, "warning", words, function(): { string }
			return props.now().lines or {}
		end, action, props.find)
	end
	return event_row("computer-cursor", words, function(): string
		return "$text.secondary"
	end, `{props.key}:0`, props.find)
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

local function pending_card(key: string, now: () -> any, find: Find?): Node
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
			found { key = `{key}:0`, text = function(): string
				return now().card.command or ""
			end, find = find, shrink = 0, font = "$font.mono" },
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
		found {
			key = `{key}:0`,
			text = function(): string
				return `/{now().card.terminal or ""}`
			end,
			find = find,
			lead = { color = "$text.primary", font = look.code_family() },
			tail = s("claude.terminal_only"),
			wrap = true,
			grow = 1,
			min_w = 0,
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
local function model_card(key: string, now: () -> any, find: Find?, current: (() -> string?)?, on_model: ((model: string) -> ())?): Node
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
			found {
				key = `{key}:1`,
				text = function(): string
					return now().card.note or s("claude.model_note")
				end,
				find = find,
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
			return pending_card(props.key, now, props.find)
		elseif kind == "model" then
			return model_card(props.key, now, props.find, props.current_model, props.on_model)
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
