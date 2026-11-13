-- A terminal tab: a shell on the session's device, in its folder, through
-- the engine's terminal calls (src/terminal.rs), drawn by zgx's terminal.
--
-- The tab opens its engine terminal once and keeps its id (and the
-- command zgx runs for it) with the tab (screens/panel/state), and zgx
-- keeps its session by the tab's id: switching tabs, sessions or reloading
-- shows the same shell, running, at once. Closing the tab asks first if a
-- program is running in it (the host says), then ends both.
--
--   Terminal.Terminal { key, tab, project, session }
--   Terminal.title(tab)            what the shell calls itself, if anything
--   Terminal.closing(tab, done)    done(busy): whether closing ends a program
--   Terminal.close(tab)            the tab's terminal ended

local ds = require("@ds")
local state = require("@app/screens/panel/state")
local api = require("@app/screens/panel/terminal_api")
local files_paths = require("@app/screens/files/paths")

type Tab = state.Tab

-- The chat page's composer, if it's there (a fixture mounts the panel alone): a terminal's selection rides its next message.
local chat_shared: any = (function(): any
	local ok, shared = pcall(function(): any
		return (require :: any)("@app/screens/chat/shared")
	end)
	return if ok and shared and type(shared.set_terminal_quote) == "function" then shared else nil
end)()

-- What's selected in it once a drag ends, nil when cleared: zgx's `term.selection` (nil on a zgx without it).
local function selection_of(term: any): string?
	local ok, text = pcall(function(): any
		return term.selection
	end)
	return if ok and type(text) == "string" then text else nil
end

local M = {}

-- Where the terminal runs (as the files pane works): the session's chat on
-- its device, or a project's folder with none open.
local target_of = files_paths.target_of

-- zgx's session for a tab (looked up, not started).
local function session_of(tab_id: string): any?
	local ok, t = pcall(api.term, tab_id)
	return if ok then t else nil
end

function M.title(tab: Tab): string?
	local t = session_of(tab.id)
	if not t then
		return nil
	end
	local ok, title = pcall(function()
		return t.title
	end)
	return if ok and type(title) == "string" and title ~= "" then title else nil
end

function M.closing(tab: Tab, done: (boolean) -> ())
	if not tab.terminal then
		-- (Still opening: nothing running in it yet.)
		done(false)
		return
	end
	api.status(tab.terminal, tab.device):after(function(s: any)
		done(s ~= nil and s.busy == true)
	end, function()
		-- (Not known: ask, rather than end something by surprise.)
		done(true)
	end)
end

function M.close(tab: Tab)
	local t = session_of(tab.id)
	if t then
		pcall(function()
			t:close()
		end)
	end
	if tab.terminal then
		api.close(tab.terminal, tab.device):after(function() end, function() end)
	end
end

local watching: { [string]: boolean } = {}

-- Only the action's own report settles its row, and the latest wins, as gpui's `exit_codes`.
local function watch_run(id: string, terminal: string, device: string?, after: string?, tail: string?)
	if not after and watching[id] then
		return
	end
	watching[id] = true
	local function over(unknown: boolean)
		watching[id] = nil
		local run = state.runs:peek()[id]
		if unknown and run and run.code == nil then
			state.runs:set_key(id, { unknown = true })
		end
	end
	api.run_report(terminal, device, after, tail):after(function(r: any)
		if not state.runs:peek()[id] then
			watching[id] = nil
		elseif r.code ~= nil then
			state.runs:set_key(id, { code = r.code })
		end
		if r.gone or r.exited then
			over(r.gone)
		elseif watching[id] then
			watch_run(id, terminal, device, r.after, r.tail)
		end
	end, function()
		over(true)
	end)
end

export type Props = { key: string, tab: string, project: any?, session: any?, [any]: any }

M.Terminal = ui.component(function(props: Props): Node
	local key, id = props.key, props.tab
	local function tab(): Tab?
		state.of(key)
		return state.find(key, id)
	end
	-- Why it didn't open: one of ours (a message key), or the engine's own
	-- words. (Plain strings: state holds no messages.)
	local failed = ui.use_state(nil :: { key: string?, text: string? }?)
	-- Opened once: its engine terminal and command kept with the tab. (A
	-- beat after it's made: nothing is written while it's built.)
	local t0 = tab()
	ui.after(0, function()
	if t0 and t0.terminal and state.runs:peek()[id] then
		watch_run(id, t0.terminal, t0.device)
	end
	if t0 and not t0.command then
		local target = target_of(props.project, props.session)
		-- A page's panel (a plugin's board) has no session: its terminal is
		-- this Mac's, opening where a new terminal opens (the engine's
		-- home folder for a terminal of no chat it knows).
		if not target and (string.sub(key, 1, 5) == "page:" or key == state.NO_PROJECT) then
			target = { chat = key }
		end
		if not target then
			failed:set({ key = "panel.terminal.no_session" })
		else
			local function fail(err: any)
				local m = if type(err) == "table" and err.message then tostring(err.message) else tostring(err)
				if m == "offline" then
					failed:set({ key = "panel.terminal.offline" })
				elseif string.find(string.lower(m), "unknown method", 1, true) then
					-- (An engine from before terminals: what to do, not why.)
					failed:set({ key = "panel.terminal.update" })
				else
					failed:set({ text = m })
				end
			end
			local known = t0.terminal
			local run = t0.run
			if not known and run then
				-- A run action (the panel home's): the engine opens its
				-- terminal, then the bridge attaches to it. (Its id kept with
				-- the tab first: a reload attaches, never runs it again.)
				local home_api = require("@app/screens/panel/home_api")
				state.runs:set_key(id, {})
				local function never_ran(err: any)
					state.runs:set_key(id, { code = -1 })
					fail(err)
				end
				home_api.run_action(run.space, run.chat, run.device, run.action):after(function(terminal: string)
					if not state.find(key, id) then
						api.close(terminal, run.device):after(function() end, function() end)
						return
					end
					state.set_terminal(key, id, terminal, run.device, nil)
					watch_run(id, terminal, run.device)
					api.command(terminal, run.device):after(function(command: string)
						if state.find(key, id) then
							state.set_terminal(key, id, terminal, run.device, command)
						end
					end, never_ran)
				end, never_ran)
			elseif known then
				-- (Its engine terminal known, its bridge not running: one
				-- that attaches.)
				api.command(known, target.device):after(function(command: string)
					state.set_terminal(key, id, known, target.device, command)
				end, fail)
			else
				-- A new one: its bridge opens it at the size zgx gives it
				-- (a shell started at another size draws its first prompt
				-- for that width), and leaves its id to be claimed.
				api.start(target):after(function(r: any)
					state.set_terminal(key, id, nil, target.device, r.command, r.claim)
					api.claimed(r.claim):after(function(terminal: string)
						local now = state.find(key, id)
						if now then
							state.set_terminal(key, id, terminal, now.device, now.command)
							if now.input then
								api.write(terminal, target.device, now.input):after(function() end, function() end)
								state.patch(key, id, { input = false })
							end
							-- (From now on, a bridge started for it attaches to
							-- this one rather than opening another.)
							api.command(terminal, target.device):after(function(command: string)
								local still = state.find(key, id)
								if still then
									state.set_terminal(key, id, terminal, still.device, command)
								end
							end, function() end)
						else
							-- (Closed before it was claimed: ended now.)
							api.close(terminal, target.device):after(function() end, function() end)
						end
					end, function() end)
				end, fail)
			end
		end
	end
	end)
	-- Which shows: the terminal, once there's one to show (a real swap).
	local function kind(): string
		if failed:get() then
			return "failed"
		end
		local t = tab()
		return if t and t.command then "ready" else "opening"
	end
	return ui.column {
		id = `panel-terminal-{id}`,
		w_full = true,
		grow = 1,
		min_h = 0,
		ui.presence(kind, function(which: string): Node
			if which == "failed" then
				local why = failed:peek()
				return ds.EmptyState {
					icon = "terminal",
					size = "sm",
					fill = true,
					title = ui.t("panel.terminal.failed") :: any,
					body = (if why and why.key then ui.t(why.key) else (if why then why.text else nil) or "") :: any,
				}
			elseif which == "opening" then
				return ui.column { w_full = true, grow = 1 }
			end
			local t = tab() :: Tab
			local term = api.term(id, {
				command = t.command,
				cwd = "~",
			})
			-- (Staged on the chat shown, as gpui's `share_selection` stages it on the chat focused.)
			local shared = chat_shared
			if shared then
				local was: string? = nil
				ui.effect(function()
					local text = selection_of(term)
					if text ~= was then
						was = text
						shared.set_terminal_quote(text)
					end
				end)
			end
			-- The keyboard, when a person opened or turned to this tab.
			ui.on_cleanup(state.focuser(props.key, id, function()
				if term and term.focus then
					term:focus()
				end
			end))
			-- (The room round it on a box of its own: zgx's terminal fits its
			-- grid to its node and lays its rows out from that node's corner,
			-- so it takes no padding itself. And `relative`: its rows are
			-- placed absolutely, and would otherwise hang from the nearest
			-- placed box above it, the window's top, under the tab strip.)
			return ui.column {
				w_full = true,
				grow = 1,
				min_h = 0,
				px = "$space.2",
				pt = "$space.2",
				api.view {
					term = term,
					label = ui.t("panel.terminal.label"),
					relative = true,
					w_full = true,
					grow = 1,
					min_h = 0,
				},
			}
		end, { w_full = true, grow = 1, min_h = 0 }),
	}
end)

return M
