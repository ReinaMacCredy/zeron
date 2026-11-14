--!strict
-- The Review built-in (screens/plugins/builtins): the chat header's Review button and the findings block in the transcript; its look, scopes and findings are the host's (src/chat/review.rs, zeron_proto's `ChatReview`, gpui's review chip's).

local Icon = require("@ds/icon")
local Tooltip = require("@ds/tooltip")
local ContextMenu = require("@ds/context_menu")
local Button = require("@ds/button")
local IconButton = require("@ds/icon_button")
local Reveal = require("@ds/reveal")
local locale = require("@ds/locale")

local M = {}

local reviews = ui.shared("review.chats", {} :: { [string]: any })
local Builtins = require("@app/screens/plugins/builtins")

function M.of(chat: string?): any
	return if chat then reviews:get()[chat] else nil
end

function M.on(): boolean
	return Builtins.on("review")
end

-- (The host's calls: a fixture stands in for them.)
function M.host(name: string, args: { [string]: any }): any
	return ui.call(name, args)
end

local function call(name: string, args: { [string]: any }): any
	local ok, p = pcall(M.host, name, args)
	if ok and p and type(p) == "table" and p.after then
		p:after(function() end, function() end)
	end
	return if ok then p else nil
end

-- The comments a fix carries, staged on the chat's composer as a diff's are (gpui's `stage_review_findings`).
local function stage(chat: string, list: { any })
	local ok, shared = pcall(function(): any
		return (require :: any)("@app/screens/chat/shared")
	end)
	if not (ok and shared and shared.attach_comment) then
		return
	end
	for _, c in list do
		shared.attach_comment({ file = c.path, line = c.line, body = c.body, session = chat })
	end
end

-- One finding's comment staged (`id`), or every open one's.
function M.fix(chat: string, id: string?)
	local p = call("review.fix", { chat = chat, id = id })
	if p and p.after then
		p:after(function(list: any)
			if type(list) == "table" then
				stage(chat, list)
			end
		end, function() end)
	end
end

-- gpui's `severity_colors`: a Bug red on red, a Risk muted red (never yellow), a Nit quiet.
local SEVERITY: { [string]: { ink: string, wash: string } } = {
	Bug = { ink = "$danger", wash = "$zeron.tone.danger.wash" },
	Risk = { ink = "$zeron.finding.risk.ink", wash = "$zeron.finding.risk.wash" },
	Nit = { ink = "$text.tertiary", wash = "$zeron.tone.neutral.wash" },
}

-- The chat header's Review: there while the built-in is on and the chat has something to review; it runs Settings → Code review's reviewer over the chat's changes. Reviewing, it says so and takes no press. Its scopes on a secondary click.
M.Button = ui.component(function(props: { chat: () -> string?, size: number }): Node
	local chat = props.chat
	local watched: string? = nil
	ui.effect(function()
		local id = chat()
		if id and id ~= watched and M.on() then
			watched = id
			call("review.watch", { chat = id })
		end
	end)
	local function r(): any
		return M.of(chat())
	end
	local function busy(): boolean
		local now = r()
		return now ~= nil and now.busy == true
	end
	local menu_open = ui.use_state(false)
	local function entries(): { ContextMenu.Entry }?
		local id, now = chat(), r()
		if not id or not now then
			return nil
		end
		local out: { ContextMenu.Entry } = { { heading = locale.t("chat.review.scope") } }
		for _, s in now.scopes or {} do
			local counts = if s.add ~= nil then ` +{s.add} −{s.del}` else ""
			table.insert(out, {
				label = s.label .. counts,
				checked = function(): boolean
					local again = r()
					return again ~= nil and again.scope == s.key
				end,
				disabled = s.note,
				on_select = function()
					call("review.scope", { chat = id, scope = s.key })
				end,
			})
		end
		table.insert(out, { separator = true })
		table.insert(out, {
			label = now.start_label,
			disabled = now.busy == true,
			on_select = function()
				call("review.start", { chat = id })
			end,
		})
		return out
	end
	local function label(): string
		return locale.s(if busy() then "chat.review.running" else "chat.review.button")
	end
	return Reveal {
		shown = function(): boolean
			local now = r()
			return M.on() and now ~= nil and now.reviewable == true
		end,
		ContextMenu {
			label = locale.t("chat.review.menu"),
			open = menu_open,
			items = function(): { ContextMenu.Entry }?
				return entries()
			end,
			ui.button {
				id = "chat-review",
				label = label,
				-- (Its tip says what a press does: the host's while it runs, else the review it starts.)
				tooltip = Tooltip {
					function(): string
						local now = r()
						if not now then
							return ""
						end
						return if now.busy then now.tip or "" else now.start_label or ""
					end,
				},
				tooltip_placement = IconButton.placement("bottom"),
				disabled = busy,
				h = props.size,
				px = "$zeron.space.tight",
				gap = ui.raw(5, "gpui's review chip"),
				items = "center",
				shrink = 0,
				rounded = "$radius.lg",
				color = "$text.secondary",
				hover = { bg = "$hover", color = "$text.primary" },
				on_click = function()
					local id = chat()
					if id and not busy() then
						call("review.start", { chat = id })
					end
				end,
				ui.presence(function(): string
					return if busy() then "busy" else "idle"
				end, function(k: string): Node
					return Icon { name = if k == "busy" then "refresh" else "checklist", size = 14 } :: any
				end),
				ui.text { label, text_size = "$font.size.sm", weight = "$font.weight.medium", wrap = false },
			},
		},
	}
end)

local OUTCOME_TONE: { [string]: string } = { fixed = "$success", skipped = "$warning", agent_on_it = "$accent" }

local function outcome_words(f: any): string
	if f.outcome == "fixed" then
		return if f.commit then locale.s("chat.review.fixed_at", { commit = f.commit }) else locale.s("chat.review.fixed")
	end
	return locale.s(`chat.review.{f.outcome or "open"}`)
end

local function finding_row(chat: () -> string?, f: any, open_link: ((target: string) -> ())?): Node
	local place = `{f.path}:{f.line}`
	-- (The finding as the host says it now: its outcome moves while its row stays.)
	local function cur(): any
		local now = M.of(chat())
		for _, x in (if now then now.findings or {} else {}) :: { any } do
			if x.id == f.id then
				return x
			end
		end
		return f
	end
	local function fixable(): boolean
		return cur().fixable == true and M.on()
	end
	return ui.row {
		id = `review-finding-{f.id}`,
		class = "review-finding",
		w_full = true,
		gap = "$space.2",
		items = "start",
		py = "$space.1",
		-- (An outdated finding at gpui's .5: its lines are gone from the changes.)
		opacity = function(): number
			local now = cur()
			if now.outdated then
				return 0.5
			end
			return if now.outcome == "fixed" or now.outcome == "no_change" then 0.6 else 1
		end,
		ui.row {
			class = "review-severity",
			shrink = 0,
			h = 18,
			px = ui.raw(5, "gpui's severity pill"),
			items = "center",
			rounded = "$radius.sm",
			bg = (SEVERITY[f.severity] or SEVERITY.Nit).wash,
			ui.text { f.severity, text_size = "$font.size.xs", weight = "$font.weight.semibold", color = (SEVERITY[f.severity] or SEVERITY.Nit).ink, wrap = false },
		},
		ui.column {
			grow = 1,
			min_w = 0,
			gap = ui.raw(2, "a finding's title over its place"),
			ui.text { f.title, text_size = "$font.size.sm", color = "$text.primary", wrap = true },
			ui.show(function(): boolean
				return cur().reason ~= nil
			end, ui.text {
				function(): string
					return cur().reason or ""
				end,
				text_size = "$font.size.xs",
				color = "$text.tertiary",
				wrap = true,
			}),
			-- (A file link, as a reply's: its words alone, brighter under the pointer. Gone from the changes, it says so after, as gpui's meta.)
			ui.row {
				self = "start",
				gap = ui.raw(4, "a place and its note"),
				items = "center",
				ui.button {
					class = "review-place",
					label = place,
					h = 18,
					p = 0,
					items = "center",
					bg = "#00000000",
					hover = { bg = "#00000000", color = "$text.secondary" },
					active = { bg = "#00000000" },
					color = "$text.tertiary",
					cursor = if open_link then "pointer" else nil,
					on_click = if open_link then function()
						open_link(place)
					end else nil,
					ui.text { place, font = "$font.mono", text_size = "$font.size.xs", literal = true, wrap = false },
				},
				ui.show(function(): boolean
					return cur().outdated == true
				end, ui.text { "· " .. locale.s("chat.review.outdated"), class = "review-outdated", font = "$font.mono", text_size = "$font.size.xs", color = "$text.tertiary", wrap = false }),
			},
		},
		ui.show(fixable, Button {
			id = `review-fix-{f.id}`,
			tone = "secondary",
			size = "xs",
			label = locale.s("chat.review.fix_named", { title = f.title }),
			on_click = function()
				local id = chat()
				if id then
					M.fix(id, f.id)
				end
			end,
			ui.text { locale.t("chat.review.fix") },
		}),
		ui.show(function(): boolean
			return not fixable()
		end, ui.row {
			class = "review-outcome",
			shrink = 0,
			gap = ui.raw(3, "a glyph and its word"),
			items = "center",
			h = 24,
			ui.show(function(): boolean
				return cur().outcome == "fixed"
			end, ui.row { color = OUTCOME_TONE.fixed, Icon { name = "check", size = 12 } :: any }),
			ui.text {
				function(): string
					return outcome_words(cur())
				end,
				text_size = "$font.size.xs",
				weight = "$font.weight.medium",
				color = function(): string
					return OUTCOME_TONE[cur().outcome] or "$text.tertiary"
				end,
				wrap = false,
			},
		}),
	}
end

-- What the block's head says beside "Review": its findings and who reviewed, or how the run ended.
local function head_words(now: any): string
	local b = now.block or {}
	local who = now.reviewer
	local function by(words: string): string
		return if who and who ~= "" then `{words} · {who}` else words
	end
	if b.status == "done" then
		local n = #(now.findings or {})
		local words = by(locale.s(if n == 1 then "chat.review.one_finding" else "chat.review.findings_count", { n = n }))
		local stale = tonumber(now.outdated) or 0
		return if stale > 0 then `{words} · {locale.s("chat.review.outdated_count", { n = stale })}` else words
	elseif b.status == "clean" then
		return by(locale.s("chat.review.clean"))
	elseif b.status == "failed" then
		return if b.message and b.message ~= "" then locale.s("chat.review.failed_why", { why = b.message }) else locale.s("chat.review.failed")
	end
	return locale.s(`chat.review.{b.status or "failed"}`)
end

-- The findings, a block of the transcript where the review was asked (the host's `review` row): each finding's severity, title, place (a file link) and how it was settled, Fix on an open one, Fix all open in its head. With the built-in off it stays readable, its actions gone.
function M.Block(props: { chat: () -> string?, open_link: ((target: string) -> ())? }): Node
	local chat = props.chat
	local function r(): any
		return M.of(chat())
	end
	return ui.show(function(): boolean
		local now = r()
		return now ~= nil and now.block ~= nil
	end, ui.column {
		id = "review-block",
		class = "review-block",
		w_full = true,
		gap = "$space.1",
		px = "$space.3",
		py = "$space.2",
		rounded = "$radius.lg",
		bg = "$surface.raised",
		border = 1,
		border_color = "$border.subtle",
		ui.row {
			w_full = true,
			items = "center",
			gap = "$space.2",
			min_h = 24,
			ui.text { locale.t("chat.review.title"), text_size = "$font.size.sm", weight = "$font.weight.semibold", shrink = 0 },
			ui.text {
				function(): string
					local now = r()
					return if now then head_words(now) else ""
				end,
				class = "review-head",
				grow = 1,
				min_w = 0,
				truncate = true,
				text_size = "$font.size.xs",
				color = "$text.tertiary",
			},
			ui.show(function(): boolean
				local now = r()
				return M.on() and now ~= nil and (now.fix_open or 0) > 0
			end, Button {
				id = "review-fix-all",
				tone = "ghost",
				size = "xs",
				label = locale.t("chat.review.fix_all"),
				on_click = function()
					local id = chat()
					if id then
						M.fix(id)
					end
				end,
				ui.text { locale.t("chat.review.fix_all") },
			}),
		},
		ui.each(ui.computed(function(): { any }
			local now = r()
			return if now and now.block and now.block.status == "done" then now.findings or {} else {}
		end), function(f: any): Node
			return finding_row(chat, f, props.open_link)
		end),
	})
end

return M
