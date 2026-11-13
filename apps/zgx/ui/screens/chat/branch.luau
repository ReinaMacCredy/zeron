local ds = require("@ds")
local motion = require("@ds/motion")
local locale = require("@ds/locale")
local store = require("@app/screens/chat/store")
local shared = require("@app/screens/chat/shared")
local route = require("@app/screens/shell/route")
local sessions = require("@account/sessions")

local M = {}

M.ring = ui.state("chat_branch_ring", 0)

local RING_IN_MS, RING_HOLD_MS, RING_OUT_MS = 120, 500, 200
local CROSSFADE_MS = 180
local EXCERPT = 72

local known = ui.state("chat_branch_known", {} :: { [string]: boolean })
local asked: { [string]: boolean } = {}

local function can_branch(s: any): boolean?
	if not s.harness then
		return nil
	end
	local key = `{s.harness}|{s.device or ""}`
	if not asked[key] then
		asked[key] = true
		local ok, call = pcall(sessions.branching, s.harness, s.device)
		if ok and call then
			call:after(function(can: any)
				if type(can) == "boolean" then
					known:set_key({ key }, can)
				end
			end, function() end)
		end
	end
	return known:get()[key]
end

local function agent_name(harness: string): string
	for _, h in store.catalog:peek().harnesses do
		if h.id == harness then
			return h.label
		end
	end
	return harness
end

local function switched_after(id: string): boolean
	local at = store.messages:index_of("id", id)
	if not at then
		return false
	end
	for i = at + 1, store.messages:len() do
		local m = store.messages:at(i)
		if m and m.role == "switch" then
			return true
		end
	end
	return false
end

local function words(err: any): string
	return if type(err) == "table" and err.message then tostring(err.message) else tostring(err)
end

local function flash()
	if motion.reduced_now() then
		M.ring:set(1)
		ui.after(RING_HOLD_MS, function()
			M.ring:set(0)
		end)
		return
	end
	anime.animate(M.ring, { value = 1, duration = RING_IN_MS, ease = "out(2)" })
	ui.after(RING_HOLD_MS, function()
		anime.animate(M.ring, { value = 0, duration = RING_OUT_MS, ease = "in(2)" })
	end)
end

local function excerpt(text: string): string
	local one = string.gsub(text, "%s+", " ")
	one = string.match(one, "^%s*(.-)%s*$") or one
	if utf8.len(one) and (utf8.len(one) :: number) > EXCERPT then
		one = string.sub(one, 1, (utf8.offset(one, EXCERPT + 1) :: number) - 1) .. "…"
	end
	return one
end

local rewinding = ui.state("chat_branch_rewinding", nil :: { chat: string, id: string, excerpt: string }?)
local rewind_open = ui.state("chat_branch_rewind_open", false)
local pending_fold: { start: (done: () -> ()) -> (), back: () -> () }? = nil

local function rewind_now()
	local r = rewinding:peek()
	local fold = pending_fold
	if not r or not fold then
		return
	end
	pending_fold = nil
	fold.start(function()
		sessions.rewind(r.chat, r.id):after(function(out: any)
			local prompt = if type(out) == "table" and type(out.prompt) == "string" then out.prompt else ""
			shared.set_draft_of(r.chat, "text", prompt)
			shared.focus:add(1)
			flash()
		end, function(err: any)
			fold.back()
			ds.toast.show { title = words(err), tone = "danger" }
		end)
	end)
end

function M.blocked(m: any, kind: "edit" | "rewind" | "fork"): string?
	local s = store.session:get()
	if not s then
		return nil
	end
	if can_branch(s) == false then
		return locale.s("chat.branch.cannot", { agent = agent_name(s.harness) })
	end
	if kind == "fork" then
		if m.streaming then
			return locale.s("chat.branch.wait_reply")
		end
		if switched_after(m.id) then
			return locale.s("chat.branch.previous_provider")
		end
		return nil
	end
	if s.running then
		return locale.s("chat.branch.stop_turn")
	end
	return nil
end

function M.props(chat: () -> string?): any
	return {
		blocked = M.blocked,
		fork = function(m: any)
			local source = chat()
			if not source then
				return
			end
			local id = store.new_id()
			sessions.fork_from(source, m.id, id):after(function(made: any)
				local to = if type(made) == "string" and made ~= "" then made else id
				local function arrive()
					route.open_session(to)
					ui.after(16, function()
						anime.animate("#transcript", { opacity = { 0, 1 }, duration = motion.ms(CROSSFADE_MS), ease = "out(2)" })
					end)
				end
				if motion.reduced_now() then
					arrive()
					return
				end
				anime.animate("#transcript", { opacity = 0, duration = CROSSFADE_MS, ease = "in(2)", onComplete = arrive })
			end, function(err: any)
				ds.toast.show { title = words(err), tone = "danger" }
			end)
		end,
		rewind = function(m: any, fold: { start: (done: () -> ()) -> (), back: () -> () })
			local id = chat()
			if not id then
				return
			end
			pending_fold = fold
			rewinding:set { chat = id, id = m.id, excerpt = excerpt(m.copy or m.text or "") }
			rewind_open:set(true)
		end,
		edit = function(m: any, text: string, back: () -> ())
			local id = chat()
			if not id then
				back()
				return
			end
			local mid = store.new_id()
			local kept = {}
			for _, a in m.attachments or {} do
				if (a.kind == "image" or a.kind == "file") and (a.path or a.src) then
					table.insert(kept, { kind = a.kind, path = a.path or a.src, name = a.name, on_host = true })
				end
			end
			store.rewind_send({ session = id, entry = m.id, text = text, message_id = mid, attachments = kept }):after(function() end, function(err: any)
				back()
				ds.toast.show { title = words(err), tone = "danger" }
			end)
		end,
	}
end

function M.Dialog(): Node
	return ds.Dialog {
		open = rewind_open,
		title = locale.t("ds.transcript.rewind_here"),
		body = function(): string
			local r = rewinding:get()
			return if r then locale.s("chat.branch.rewind_before", { excerpt = r.excerpt }) else ""
		end,
		confirm = {
			label = locale.t("chat.branch.rewind"),
			on_click = rewind_now,
		},
		ui.text {
			locale.t("chat.branch.rewind_detail"),
			wrap = true,
			text_size = "$font.size.sm",
			color = "$text.secondary",
		},
	}
end

return M
