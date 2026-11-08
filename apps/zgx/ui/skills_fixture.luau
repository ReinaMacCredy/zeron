-- The shell fixture with the Skills page's host answered here (src/skills.rs): two agents, a skill both load, one each,
-- one of the harness's own; what was asked is `skills_calls`. `skills.refuse_next` refuses the next call.

local node = require("./shell_fixture")
local store = require("@app/screens/skills/store")
local chat_store = require("@app/screens/chat/store")

chat_store.catalog:set({
	state = "ready",
	harnesses = { { id = "claude-code", label = "Claude", mark = "claude" }, { id = "codex", label = "Codex", mark = "openai" } },
	models = { { value = "opus", label = "Opus", harness = "claude-code" }, { value = "gpt-5.5", label = "GPT-5.5", harness = "codex" } },
} :: any)

local said: { string } = {}
local calls = ui.state("skills_calls", "")
local refuse = false
ui.command {
	id = "skills.refuse_next",
	title = "Refuse the next skills call",
	run = function()
		refuse = true
	end,
}
local function note(what: string)
	table.insert(said, what)
	calls:set(table.concat(said, ","))
end

local function settled(value: any): any
	local word = if refuse then "refused" else nil
	refuse = false
	local p = {}
	function p.after(_self: any, ok: ((any) -> ())?, bad: ((any) -> ())?)
		if word then
			if bad then
				bad({ message = word })
			end
		elseif ok then
			ok(value)
		end
	end
	return p
end

local HOME = "/home/ada"
local function entry(name: string, title: string, path: string, harnesses: { string }, description: string): any
	local on_disk = string.sub(path, 1, 1) == "/"
	return {
		name = name,
		title = title,
		path = path,
		paths = { path },
		shown = if on_disk then "~" .. string.sub(string.gsub(path, "/SKILL%.md$", ""), #HOME + 1) else path,
		description = description,
		harnesses = harnesses,
		on_disk = on_disk,
		link = `[${name}](zeron-invoke:{name})`,
		links = {},
	}
end

local entries = {
	entry("code-review", "Code Review", HOME .. "/.agents/skills/code-review/SKILL.md", { "claude-code", "codex" }, "Review a diff with care"),
	entry("native", "Native", "harness-skill:native", { "codex" }, "Built into Codex"),
	entry("pdf", "Pdf", HOME .. "/.claude/skills/pdf/SKILL.md", { "claude-code" }, "Read and write PDF files"),
}
entries[1].paths = { entries[1].path, HOME .. "/.codex/skills/code-review/SKILL.md" }
-- A second `pdf`, Codex's own, its bytes not Claude's: two cards of one name.
ui.command {
	id = "skills.twin_pdf",
	title = "Codex has a pdf skill of its own",
	run = function()
		table.insert(entries, entry("pdf", "Pdf", HOME .. "/.codex/skills/pdf/SKILL.md", { "codex" }, "Codex's own PDF skill"))
	end,
}

store.host = {
	list = function(): any
		note("list")
		return settled({
			harnesses = { { id = "claude-code", name = "Claude Code", mark = "claude" }, { id = "codex", name = "Codex", mark = "openai" } },
			entries = table.clone(entries),
		})
	end,
	read = function(path: string): any
		note(`read:{path}`)
		return settled("---\nname: code-review\ndescription: Review a diff with care\n---\n\n# Code review\n\nRead the whole diff first.\n")
	end,
	open = function(path: string): any
		note(`open:{path}`)
		return settled(nil)
	end,
	reveal = function(path: string): any
		note(`reveal:{path}`)
		return settled(nil)
	end,
	trash = function(paths: { string }): any
		note(`trash:{table.concat(paths, "+")}`)
		local p = settled(nil)
		local gone = paths[1]
		local after = p.after
		function p.after(self: any, ok: any, bad: any)
			after(self, function(v: any)
				for i, e in entries do
					if e.paths[1] == gone then
						table.remove(entries, i)
						break
					end
				end
				ok(v)
			end, bad)
		end
		return p
	end,
}

return node
