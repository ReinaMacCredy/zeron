-- The Skills page's data (crates/ui/src/shell/skills.rs): every agent's skills, one card a skill, from the host (src/skills.rs).

local locale = require("@ds/locale")
local Invocations = require("@app/screens/chat/invocations")

local M = {}

export type Entry = {
	name: string,
	title: string,
	path: string,
	paths: { string },
	shown: string,
	description: string,
	harnesses: { string },
	on_disk: boolean,
	link: string,
	links: { [string]: string },
}
export type Harness = { id: string, name: string, mark: string?, icon: string? }
export type List = { state: "idle" | "loading" | "ready" | "failed", harnesses: { Harness }, entries: { Entry }, error: string? }

-- The host's actions (a test puts its own here).
M.host = {
	list = function(): any
		return ui.call("skills.list")
	end,
	read = function(path: string): any
		return ui.call("skills.read", path)
	end,
	open = function(path: string): any
		return ui.call("skills.open", path)
	end,
	reveal = function(path: string): any
		return ui.call("skills.reveal", path)
	end,
	trash = function(paths: { string }): any
		return ui.call("skills.trash", paths)
	end,
}

M.list = ui.state("skills_list", { state = "idle", harnesses = {}, entries = {} } :: List)
-- The agent shown, nil for all.
M.filter = ui.state("skills_filter", nil :: string?)
M.disabled = Invocations.disabled

local asked = 0

function M.load()
	asked += 1
	local mine = asked
	local had = M.list:peek()
	if had.state ~= "ready" then
		M.list:set({ state = "loading", harnesses = {}, entries = {} })
	end
	local function failed(why: string)
		if mine == asked and M.list:peek().state ~= "ready" then
			M.list:set({ state = "failed", harnesses = {}, entries = {}, error = why })
		end
	end
	-- (An older host has no such action.)
	local ok, p = pcall(M.host.list)
	if not ok or p == nil then
		failed(locale.s("skills.old_host"))
		return
	end
	p:after(function(v: any)
		if mine ~= asked then
			return
		end
		local harnesses = (v and v.harnesses) or {}
		M.list:set({ state = "ready", harnesses = harnesses, entries = (v and v.entries) or {} })
		local f = M.filter:peek()
		if f then
			local still = false
			for _, h in harnesses do
				still = still or h.id == f
			end
			if not still then
				M.filter:set(nil)
			end
		end
	end, function(e: any)
		failed(if e and e.message then tostring(e.message) else "failed")
	end)
end

function M.shown(f: string?): { Entry }
	local out = {}
	for _, e in M.list:get().entries do
		if f == nil or table.find(e.harnesses, f) then
			table.insert(out, e)
		end
	end
	return out
end

function M.harness_name(id: string): string
	for _, h in M.list:peek().harnesses do
		if h.id == id then
			return h.name
		end
	end
	return id
end

function M.find(path: string?): Entry?
	for _, e in M.list:get().entries do
		if e.path == path then
			return e
		end
	end
	return nil
end

-- On, unless one of its copies is off: then each copy goes on, else each goes off.
function M.enabled(e: Entry, peek: boolean?): boolean
	local off = if peek then M.disabled:peek() else M.disabled:get()
	for _, p in e.paths do
		if off[p] then
			return false
		end
	end
	return true
end

function M.set_enabled(e: Entry, on: boolean)
	local off = table.clone(M.disabled:peek())
	for _, p in e.paths do
		off[p] = if on then nil else true
	end
	M.disabled:set(off)
end

-- SKILL.md's body, its front matter left out (zeron_engine::playbooks::parse).
function M.body(text: string): string
	local rest = string.match(text, "^%-%-%-\r?\n.-\r?\n%-%-%-\r?\n(.*)$") or string.match(text, "^%-%-%-\r?\n.-\r?\n%-%-%-$") and "" or text
	return (string.gsub(rest, "^%s+", ""))
end

return M
