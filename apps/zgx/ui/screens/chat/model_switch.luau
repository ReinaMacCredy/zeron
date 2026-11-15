-- Picking another model on a chat with history asks first, as Zeron's picker
-- does: "Switch model?" (the next response rereads the chat) or "Switch
-- provider?" (the new agent starts from a text copy of it). Yes lands the pick
-- on the chat, with its divider in the transcript; 1 or Enter says yes (Enter
-- on a focused option, that option), 2 or Escape no.

local ds = require("@ds")
local locale = require("@ds/locale")
local store = require("@app/screens/chat/store")

local M = {}

-- What's being asked: the pick, its words, and the divider it leaves.
export type Ask = {
	value: string,
	title: string,
	lead: string,
	body: string,
	yes: string,
	yes_detail: string,
	no: string,
	no_detail: string,
	note: string,
	note_hint: string,
}

local function t(key: string, args: { [string]: any }?): string
	return locale.s(`chat.composer.switch.{key}`, args)
end

-- Models the engine names new for the first time (src/chat.rs `announce`),
-- posted as Zeron's banner while its notifications are on: "New model
-- available", the agent and the names. Those standing when this window first
-- looked were another window's to post. (`told`: the last posted, and how far.)
local told = ui.state("chat_models_announced", { seen = -1 } :: { seen: number, title: string?, body: string? })
ui.effect(function()
	local list: { any } = (store.catalog:get() :: any).announced or {}
	local was = told:peek()
	local top = 0
	for _, a in list do
		top = math.max(top, a.seq)
	end
	if top == was.seen then
		return
	end
	local now: { seen: number, title: string?, body: string? } = { seen = top, title = was.title, body = was.body }
	for _, a in list do
		if was.seen >= 0 and a.seq > was.seen then
			local title = locale.s("chat.composer.new_models.title", { n = #a.models })
			local body = `{a.agent} · {table.concat(a.models, ", ")}`
			now.title, now.body = title, body
			local ok, asked = pcall(ui.call, "notifications.get")
			if ok and asked then
				asked:after(function(v: any)
					if v and v.desktop ~= false then
						pcall(function()
							ui.notify({ title = title, body = body, tag = `zeron-new-models-{a.seq}` }):after(function() end, function() end)
						end)
					end
				end, function() end)
			end
		end
	end
	told:set(now)
end)

local function model_of(value: string?): any
	for _, m in store.catalog:peek().models do
		if m.value == value then
			return m
		end
	end
	return nil
end
local function agent_name(id: string?): string
	for _, h in store.catalog:peek().harnesses do
		if h.id == id then
			return h.label
		end
	end
	return id or ""
end

-- Whether a chat holds anything an agent would read again: more than the
-- transcript's own dividers, a handoff counting (Zeron's `chat_has_history`).
-- (`loaded` false: its transcript not come yet, which counts too.)
function M.has_history(messages: { any }, running: boolean?, loaded: boolean?): boolean
	if running or loaded == false then
		return true
	end
	for _, m in messages do
		if (m.role ~= "system" and m.role ~= "switch") or m.line == "handoff" or m.handoff == true then
			return true
		end
	end
	return false
end

local function ask_for(from_value: string?, value: string): Ask
	local from, to = model_of(from_value), model_of(value)
	-- (An agent alone, "<agent>/": a profile with no model of its own, or a
	-- chat with none.)
	local from_label = if from then from.label elseif string.sub(from_value or "", -1) == "/" then agent_name(string.sub(from_value :: string, 1, -2)) else (from_value or "")
	local to_label = if to then to.label elseif string.sub(value, -1) == "/" then agent_name(string.sub(value, 1, -2)) else value
	local from_agent = if from then from.harness else string.match(from_value or "", "^([^/]+)/")
	local to_agent = if to then to.harness else string.match(value, "^([^/]+)/")
	if from_agent ~= to_agent then
		local a, b = agent_name(from_agent), agent_name(to_agent)
		return {
			value = value,
			title = t("provider_title"),
			lead = t("provider_lead", { to = to_label }),
			body = t("provider_body", { from = a, to = b }),
			yes = t("yes", { to = to_label }),
			yes_detail = t("provider_yes_detail", { from = a }),
			no = t("provider_no", { from = a }),
			no_detail = t("no_detail", { from = a }),
			note = `Provider changed from {a} to {b}.`,
			note_hint = `{a} session closed. The messages above were handed to {b} as text on the next turn; {a}'s tool results and file state were not carried.`,
		}
	end
	local body = if to_agent == "claude-code"
		then t("body_claude", { to = to_label })
		elseif to_agent == "codex" then t("body_codex")
		else t("body_other", { to = to_label })
	local hint = if to_agent == "claude-code"
		then "Changing models mid-conversation drops the prompt cache. The full history is re-read on the next message."
		elseif to_agent == "codex" then "Changing models mid-conversation will degrade performance. Context may automatically compact."
		else "Changing models mid-conversation may degrade performance. The full history is re-read on the next message."
	return {
		value = value,
		title = t("model_title"),
		lead = t("model_lead"),
		body = body,
		yes = t("yes", { to = to_label }),
		yes_detail = t("yes_detail"),
		no = t("no"),
		no_detail = t("no_detail", { from = from_label }),
		note = `Model changed from {from_label} to {to_label}.`,
		note_hint = hint,
	}
end

function M.note(from_value: string?, value: string): { note: string, note_hint: string }
	local a = ask_for(from_value, value)
	return { note = a.note, note_hint = a.note_hint }
end

-- For a composer: `current()`, the model it has; `history()`, whether its chat
-- has anything to reread (nil on a new chat: nothing asks); `land(value, ask)`,
-- a pick answered yes; `back()`, answered no (2 or Escape), the picker again,
-- as Zeron's card stays under its dialog. Gives the picker's `take`, and the
-- dialog to mount. (`key`: whose it is, a pane's or the chat page's.)
function M.use(key: string, opts: { current: () -> string?, history: () -> boolean?, land: (value: string, ask: Ask?) -> (), back: (() -> ())? }): ((value: string) -> boolean, Node)
	local open = ui.state(`chat_model_switch_open_{key}`, false)
	local asking = ui.state(`chat_model_switch_{key}`, nil :: Ask?)
	local answered = false
	local was_open = open:peek()
	ui.effect(function()
		local now = open:get()
		if now then
			answered = false
		elseif was_open and not answered and opts.back then
			-- (Out of the dialog's own close: the picker opening inside an Escape there hangs.)
			ui.after(0, opts.back)
		end
		was_open = now
	end)
	local function take(value: string): boolean
		local from = opts.current()
		if opts.history() ~= true or from == value or from == nil then
			return false
		end
		asking:set(ask_for(from, value))
		open:set(true)
		return true
	end
	local function now(): Ask
		return asking:get() or ask_for(nil, "")
	end
	local function yes()
		local a = asking:peek()
		answered = true
		open:set(false)
		if a then
			opts.land(a.value, a)
		end
	end
	local function no()
		open:set(false)
	end
	local function keys(e: any): boolean
		if e.key == "1" or e.key == "Enter" then
			yes()
			return true
		elseif e.key == "2" then
			no()
			return true
		end
		return false
	end
	local function option(id: string, number: string, title: () -> string, detail: () -> string, on_click: () -> ()): Node
		local function own_keys(e: any): boolean
			if e.key == "Enter" then
				on_click()
				return true
			end
			return keys(e)
		end
		return ui.button {
			id = id,
			w_full = true,
			px = "$space.2",
			py = "$space.2",
			gap = "$space.2",
			items = "start",
			rounded = "$zeron.menu.row_radius",
			bg = "#00000000",
			hover = { bg = "$hover" },
			focus = { bg = "$hover", outline = 0 },
			label = title,
			on_click = on_click,
			on_key = own_keys,
			ui.text { number, w = 16, shrink = 0, color = "$text.disabled" },
			ui.column {
				grow = 1,
				min_w = 0,
				gap = "$zeron.space.hair",
				ui.text { title, color = "$text.primary", wrap = true, text_size = "$font.size.md" },
				ui.text { detail, color = "$text.tertiary", wrap = true, text_size = "$font.size.sm" },
			},
		}
	end
	return take,
		ds.Dialog {
			open = open,
			title = function(): string
				return now().title
			end,
			cancel = false,
			size = "sm",
			ui.column {
				id = "model-switch",
				w_full = true,
				on_key = keys,
				ui.text {
					function(): string
						return now().lead
					end,
					mt = "$zeron.space.tight",
					color = "$warning",
					wrap = true,
					text_size = "$font.size.md",
				},
				ui.text {
					function(): string
						return now().body
					end,
					mt = "$space.2",
					color = "$text.secondary",
					wrap = true,
					text_size = "$font.size.md",
				},
				ui.div { w_full = true, h = 1, mt = "$space.3", bg = "$border" },
				ui.column {
					w_full = true,
					mt = "$space.2",
					gap = "$zeron.space.hair",
					option("model-switch-yes", "1.", function(): string
						return now().yes
					end, function(): string
						return now().yes_detail
					end, yes),
					option("model-switch-no", "2.", function(): string
						return now().no
					end, function(): string
						return now().no_detail
					end, no),
				},
				ui.text {
					t("keys"),
					mt = "$space.2",
					color = "$text.disabled",
					text_size = "$font.size.xs",
				},
			},
		}
end

return M
