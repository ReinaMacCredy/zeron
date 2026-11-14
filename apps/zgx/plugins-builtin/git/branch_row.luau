--!strict
-- The panel home's branch row, Git's part of it: what the branch adds over
-- its base, `+N −M`, at the row's end (core's `slot:zeron.panel.branch`,
-- which provides `panel_branch`: { session, device, cwd, branch }). A
-- click opens the Changes tab at the Branch scope. Asked of the service on
-- the session's device (`branch_totals`) as the home shows the session, and
-- again as its folder, branch or work moves (a turn ending). Nothing to say
-- (no base, nothing added), or the device not answering: nothing shows,
-- and the row keeps its branch's name alone, as it does with Git off.

local PanelHome = require("@ds/panel_home")
local Tooltip = require("@ds/tooltip")
local control = require("@ds/control")
local Reveal = require("@ds/reveal")

return function(zeron: any, Repo: any)
	-- Each session's totals as last heard (kept while the window is: the
	-- home shows them at once when it's back).
	local heard = ui.state("git.branch_totals", {} :: { [string]: any })

	local function counts(branch: () -> any, panel_of: () -> any, id: string, text_size: any, corner: any, gap: any): (() -> boolean, Node)
		local function slot_of(b: any): string?
			return if b and type(b.session) == "string" then b.session else nil
		end
		local function where(b: any): string
			return `{b.device or ""}|{b.cwd or ""}`
		end
		-- (Totals heard in another folder or device stand only while this one's answer is on its way.)
		local asking = ui.use_state(false)
		-- What to ask again on: the folder, the branch, the device and the
		-- session's status (a turn ending moves the work).
		local asked: string? = nil
		ui.effect(function()
			local b = branch()
			if type(b) ~= "table" or not b.cwd or (b.branch or "") == "" then
				asked = nil
				if asking:peek() then
					asking:set(false)
				end
				return
			end
			local record = zeron.context_of("session", b.session)
			local key = `{b.session}|{b.cwd}|{b.branch}|{b.device or ""}|{record.status or ""}`
			if key == asked then
				return
			end
			asked = key
			asking:set(true)
			local slot = slot_of(b) :: string
			local at = where(b)
			zeron.service(b.device):call("branch_totals", { cwd = b.cwd }, { timeout = 60 }):after(function(t: any)
				if asked ~= key then
					return
				end
				local now = table.clone(heard:peek())
				now[slot] = if type(t) == "table" and t.adds then { adds = t.adds, dels = t.dels or 0, at = at } else false
				heard:set(now)
				asking:set(false)
			end, function(err: any)
				-- (Not known: the row keeps its branch alone.)
				print(`git: branch totals: {err}`)
				if asked == key then
					local was = heard:peek()[slot]
					if type(was) == "table" and was.at ~= at then
						local now = table.clone(heard:peek())
						now[slot] = nil
						heard:set(now)
					end
					asking:set(false)
				end
			end)
		end)
		local function totals(): { adds: number, dels: number }?
			local b = branch()
			local slot = slot_of(b)
			local t = if slot then heard:get()[slot] else nil
			if type(t) ~= "table" or t.adds + t.dels == 0 then
				return nil
			end
			if t.at ~= where(b) and not asking:get() then
				return nil
			end
			return t
		end
		local held = ui.use_state(totals())
		ui.effect(function()
			local t = totals()
			if t then
				held:set(t)
			end
		end)
		local function open()
			local b, panel = branch(), panel_of()
			if panel and b then
				Repo.open_tab(panel, b.session, "changes", "branch")
			end
		end
		return function(): boolean
			return totals() ~= nil
		end,
		ui.row {
			id = id,
			role = "button",
			label = function(): string
				local t = held:get() or { adds = 0, dels = 0 }
				return zeron.s("home.diff_label", { adds = t.adds, dels = t.dels })
			end,
			tooltip = Tooltip { zeron.t("home.branch_changes") },
			focusable = true,
			cursor = "pointer",
			shrink = 0,
			h = 24,
			px = "$space.1",
			gap = gap,
			items = "center",
			rounded = corner,
			hover = { bg = "$hover" },
			focus = control.ring,
			on_click = open,
			on_key = function(e: any): boolean?
				if e.key == "Enter" or e.key == " " then
					open()
					return true
				end
				return nil
			end,
			ui.text {
				function(): string
					local t = held:get()
					return if t then `+{t.adds}` else ""
				end,
				literal = true,
				font = "$font.mono",
				tabular = true,
				text_size = text_size,
				color = "$zeron.diff.added",
			},
			ui.text {
				function(): string
					local t = held:get()
					return if t then `−{t.dels}` else ""
				end,
				literal = true,
				font = "$font.mono",
				tabular = true,
				text_size = text_size,
				color = "$zeron.diff.removed",
			},
		}
	end

	return {
		row = function(): Node
			local branch = zeron.live("panel_branch")
			local first = branch()
			if type(first and first.session) ~= "string" then
				return ui.div {}
			end
			local panel = zeron.context("panel")
			return ui.show(counts(branch, function(): any
				return panel
			end, "git-branch-totals", "$font.size.xs", PanelHome.inner(6), "$space.1"))
		end,
		toggle = function(): Node
			-- (The title bar provides no session: the window's page's, read as it changes.)
			local page = ui.shared("zeron.page_session", nil :: any)
			local function branch(): any
				local session = page:get()
				if type(session) ~= "string" then
					return nil
				end
				local record = zeron.context_of("session", session)
				return { session = session, device = record.device, cwd = record.cwd, branch = record.branch }
			end
			local shown, node = counts(branch, function(): any
				return zeron.context("panel")
			end, "git-toggle-totals", "$font.size.xs", "$radius.md", ui.raw(6, "Zeron's gap in the toggle"))
			return Reveal { shown = shown, node }
		end,
	}
end
