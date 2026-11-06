-- Where an open chat works, as its footer says (Zeron's pickers.rs
-- `render_footer`): Cloud, the worktree and its folder, or the local
-- checkout; the live branch; how the worktree stands against its base. Its
-- branch chip opens the project's refs: worktrees first, picking one moves
-- the chat there, the current branch back to the main checkout. A chat never
-- switches branches.

local store = require("@app/screens/chat/store")
local sessions = require("@account/sessions")
local route = require("@app/screens/shell/route")

local M = {}


export type Bind = {
	open: () -> any,
	cloud: string,
	live_branch: () -> string?,
	notice: (notice: any) -> (),
}

local function folder_of(path: string): string
	return string.match(path, "([^/]+)/?$") or path
end

function M.badge(t: any, base: string?): any
	if not t then
		return nil
	end
	local from = base or ui.t("chat.composer.footer.base_fallback"):get()
	if t.state == "ahead" then
		local n = t.commits or 0
		return {
			kind = "ahead",
			count = n,
			tip = if n == 1 then ui.t("chat.composer.footer.ahead_one", { base = from }):get() else ui.t("chat.composer.footer.ahead", { n = n, base = from }):get(),
		}
	elseif t.state == "merged" then
		local dirty = t.dirty or 0
		local tip = if dirty == 0
			then ui.t("chat.composer.footer.merged", { base = from }):get()
			elseif dirty == 1 then ui.t("chat.composer.footer.merged_dirty_one", { base = from }):get()
			else ui.t("chat.composer.footer.merged_dirty", { n = dirty, base = from }):get()
		return { kind = "merged", dirty = dirty, tip = tip }
	end
	return nil
end

function M.bind(b: Bind): { [string]: any }
	local refs = ui.state("chat_place_refs", { state = "idle", list = {} } :: any)
	local function project_path(s: any): string?
		for _, p in sessions.now:get().projects do
			if p.id == s.project then
				return p.path
			end
		end
		return nil
	end
	local function fixed(): any
		local s = b.open()
		if not s then
			return nil
		end
		local t = s.work_tree
		local path = project_path(s)
		local out: { [string]: any } = { worktree = false }
		if s.device == b.cloud then
			out.icon, out.label, out.read_only = "cloud", ui.t("chat.composer.footer.cloud"):get(), true
		elseif t then
			out.icon, out.worktree = "worktree", true
			out.label = ui.t("chat.composer.footer.worktree_folder", { folder = t.folder or folder_of(t.path) }):get()
			out.removed = t.removed == true
		elseif path and s.cwd and s.cwd ~= path then
			out.icon, out.worktree, out.label = "worktree", true, ui.t("chat.composer.footer.worktree"):get()
		else
			out.icon, out.label = "folder", ui.t("chat.composer.footer.local"):get()
		end
		if t then
			out.name = t.branch or ui.t("chat.composer.footer.detached"):get()
		else
			out.name = b.live_branch()
		end
		if s.device ~= b.cloud and not (t and t.removed) then
			out.badge = M.badge(t, s.branch)
		end
		out.project = path ~= nil
		return out
	end
	local function load()
		local s = b.open()
		local path = s and project_path(s)
		if not s or not path then
			refs:set { state = "none", list = {} }
			return
		end
		refs:set { state = "loading", list = refs:peek().list }
		local here = sessions.now:peek().here
		local ok, answer = pcall(store.refs, path, if s.device ~= here then s.device else nil)
		if not ok or answer == nil then
			refs:set { state = "error", list = {} }
			return
		end
		answer:after(function(list: any)
			refs:set { state = "ready", list = list or {} }
		end, function()
			refs:set { state = "error", list = {} }
		end)
	end
	local items = ui.computed(function(): { any }
		local list = refs:get().list
		local trees, plain = {}, {}
		for _, r in list do
			table.insert(if r.worktree then trees else plain, r)
		end
		local out = {}
		local function add(r: any, heading: string?)
			table.insert(out, {
				value = r.name,
				label = r.name,
				icon = "git-branch",
				heading = heading,
				tag = if r.worktree then folder_of(r.worktree) elseif r.current then ui.t("chat.composer.footer.main_checkout"):get() else nil,
			})
		end
		for i, r in trees do
			add(r, if i == 1 then ui.t("chat.composer.footer.section_worktrees"):get() else nil)
		end
		for i, r in plain do
			add(r, if i == 1 then ui.t("chat.composer.footer.section_branches"):get() else nil)
		end
		return out
	end)
	-- A row taken, only then: the chat moved where it has a worktree; else a word.
	local function on_pick(name: string)
		local s = b.open()
		if not s or name == b.live_branch() or name == (s.work_tree and s.work_tree.branch) then
			return
		end
		for _, r in refs:peek().list do
			if r.name == name then
				if r.worktree or r.current then
					local ok, answer = pcall(store.set_work_tree, s.id, r.worktree)
					if ok and answer then
						answer:after(function() end, function(e: any)
							print(`zeron-zgx: a chat wasn't moved: {e}`)
						end)
					end
				else
					b.notice { tone = "warning", text = ui.t("chat.composer.footer.no_worktree", { name = name }):get(), dismissable = true }
				end
				return
			end
		end
	end
	return {
		fixed = fixed,
		menu = {
			on_pick = on_pick,
			items = items,
			on_open = load,
			empty = function(query: string): any
				local state = refs:peek().state
				if state == "loading" and query == "" then
					return { title = ui.t("chat.where.branches_empty.loading") }
				end
				return { title = ui.t("chat.composer.footer.no_refs") }
			end,
			search = function(): string
				return ui.t("chat.composer.footer.search_refs"):get()
			end,
		},
	}
end

return M
