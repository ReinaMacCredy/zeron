--!strict
-- gpui's composer review chip (composer/review_chip.rs) and its findings; its look, scopes and findings are the host's (src/chat/review.rs, zeron_proto's `ChatReview`).

local Icon = require("@ds/icon")
local Tooltip = require("@ds/tooltip")
local ContextMenu = require("@ds/context_menu")
local IconButton = require("@ds/icon_button").IconButton
local Button = require("@ds/button")
local locale = require("@ds/locale")

local M = {}

local reviews = ui.shared("review.chats", {} :: { [string]: any })

function M.of(chat: string?): any
	return if chat then reviews:get()[chat] else nil
end

-- The chat whose findings are open over its composer.
M.open = ui.state("review_findings_open", nil :: string?)

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
local function stage(_chat: string, list: { any })
	local ok, shared = pcall(function(): any
		return (require :: any)("@app/screens/chat/shared")
	end)
	if not (ok and shared and shared.attach_comment) then
		return
	end
	for _, c in list do
		shared.attach_comment({ file = c.path, line = c.line, body = c.body })
	end
end

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

local SEVERITY_TONE: { [string]: string } = { Bug = "$zeron.status.danger", Risk = "$zeron.tone.danger.ink", Nit = "$text.tertiary" }

M.Chip = ui.component(function(props: { chat: () -> string? }): Node
	local chat = props.chat
	local watched: string? = nil
	ui.effect(function()
		local id = chat()
		if id and id ~= watched then
			watched = id
			call("review.watch", { chat = id })
		end
	end)
	local function r(): any
		return M.of(chat())
	end
	local menu_open = ui.use_state(false)
	local open_at: ((x: number, y: number) -> boolean)? = nil
	local caret = ui.ref()
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
	local function click()
		local id, now = chat(), r()
		if not id or not now then
			return
		end
		if now.action == "open" then
			M.open:set(if M.open:peek() == id then nil else id)
		elseif now.action == "start" then
			M.open:set(nil)
			call("review.start", { chat = id })
		end
	end
	return ui.show(function(): boolean
		local now = r()
		return now ~= nil and now.label ~= nil
	end, ContextMenu {
		label = locale.t("chat.review.menu"),
		open = menu_open,
		hook = function(at: (x: number, y: number) -> boolean)
			open_at = at
		end,
		items = function(): { ContextMenu.Entry }?
			return entries()
		end,
		ui.row {
			id = "composer-review-chip",
			items = "center",
			shrink = 0,
			ui.button {
				id = "composer-review",
				label = function(): string
					local now = r()
					return if now then now.label or "" else ""
				end,
				tooltip = Tooltip {
					function(): string
						local now = r()
						return if now then now.tip or "" else ""
					end,
				},
				h = 28,
				px = ui.raw(8, "gpui's review chip"),
				gap = ui.raw(5, "gpui's review chip"),
				items = "center",
				rounded = ui.raw(8, "gpui's review chip"),
				color = "$text.secondary",
				hover = { bg = "$hover" },
				on_click = click,
				ui.presence(function(): string
					local now = r()
					return if now and now.busy then "busy" else "idle"
				end, function(k: string): Node
					return Icon { name = if k == "busy" then "refresh" else "checklist", size = 14 } :: any
				end),
				ui.text {
					function(): string
						local now = r()
						return if now then now.label or "" else ""
					end,
					text_size = "$font.size.sm",
					wrap = false,
				},
				ui.show(function(): boolean
					local now = r()
					return now ~= nil and now.add ~= nil
				end, ui.row {
					gap = ui.raw(3, "gpui's review chip"),
					ui.text {
						function(): string
							local now = r()
							return if now and now.add then `+{now.add}` else ""
						end,
						text_size = "$font.size.xs",
						color = "$zeron.status.success",
						wrap = false,
					},
					ui.text {
						function(): string
							local now = r()
							return if now and now.del then `−{now.del}` else ""
						end,
						text_size = "$font.size.xs",
						color = "$zeron.status.danger",
						wrap = false,
					},
				}),
			},
			IconButton {
				id = "composer-review-scope",
				ref = caret,
				icon = "alt-arrow-down",
				label = locale.t("chat.review.menu"),
				size = "sm",
				tone = "quiet",
				lit = menu_open,
				on_click = function()
					local el, at = caret:peek(), open_at
					if el and at then
						at(el.x or 0, (el.y or 0) + (el.height or 0) / 2)
					end
				end,
			},
		},
	})
end)

local function finding_row(chat: string, f: any): Node
	return ui.column {
		id = `review-finding-{f.id}`,
		class = "review-finding",
		w_full = true,
		gap = "$space.1",
		py = "$space.2",
		ui.row {
			w_full = true,
			gap = "$space.2",
			items = "center",
			ui.text { f.severity, class = "review-severity", text_size = "$font.size.xs", weight = "$font.weight.semibold", color = SEVERITY_TONE[f.severity] or "$text.secondary", shrink = 0 },
			ui.text { f.title, grow = 1, min_w = 0, truncate = true, text_size = "$font.size.sm", weight = "$font.weight.medium" },
			ui.text { f.state, class = "review-state", text_size = "$font.size.xs", color = if f.open then "$text.tertiary" else "$zeron.status.success", shrink = 0 },
			IconButton {
				id = `review-dismiss-{f.id}`,
				icon = "close",
				label = locale.t("chat.review.dismiss"),
				size = "sm",
				tone = "quiet",
				on_click = function()
					call("review.dismiss", { chat = chat, id = f.id })
				end,
			},
		},
		ui.text { f.location, text_size = "$font.size.xs", color = "$text.tertiary", literal = true },
		if f.body ~= "" then ui.text { f.body, text_size = "$font.size.sm", color = "$text.secondary", wrap = true } else nil,
		if f.reason then ui.text { f.reason, text_size = "$font.size.xs", color = "$text.tertiary", wrap = true } else nil,
	}
end

function M.Findings(props: { chat: () -> string? }): Node
	local chat = props.chat
	return ui.show(function(): boolean
		local id = chat()
		local now = M.of(id)
		return id ~= nil and M.open:get() == id and now ~= nil and #(now.findings or {}) > 0
	end, ui.column {
		id = "review-findings",
		w_full = true,
		max_h = ui.raw(320, "gpui's findings card"),
		overflow_y = "auto",
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
			ui.text { locale.t("chat.review.findings"), grow = 1, text_size = "$font.size.sm", weight = "$font.weight.semibold" },
			ui.text {
				function(): string
					local now = M.of(chat())
					return if now then now.outcomes or "" else ""
				end,
				text_size = "$font.size.xs",
				color = "$text.tertiary",
			},
			ui.show(function(): boolean
				local now = M.of(chat())
				return now ~= nil and now.fix ~= nil
			end, Button {
				id = "review-fix-remaining",
				tone = "secondary",
				size = "sm",
				label = function(): string
					local now = M.of(chat())
					return if now then now.fix or "" else ""
				end,
				on_click = function()
					local id = chat()
					if id then
						M.fix(id)
					end
				end,
				ui.text {
					function(): string
						local now = M.of(chat())
						return if now then now.fix or "" else ""
					end,
				},
			}),
		},
		ui.each(ui.computed(function(): { any }
			local now = M.of(chat())
			return if now then now.findings or {} else {}
		end), function(f: any): Node
			return finding_row(chat() or "", f)
		end),
	})
end

return M
