-- The shell fixture with Settings > Playbooks' host answered here (src/playbooks.rs): triage, one of this Mac's, one of
-- a project's; what was asked is `playbooks_calls`.

local node = require("./shell_fixture")
local page = require("@app/screens/settings/playbooks")

local said: { string } = {}
local calls = ui.state("playbooks_calls", "")
local function note(what: string)
	table.insert(said, what)
	calls:set(table.concat(said, ","))
end

local function settled(value: any): any
	local p = {}
	function p.after(_self: any, ok: ((any) -> ())?, _bad: ((any) -> ())?)
		if ok then
			ok(value)
		end
	end
	return p
end

local FOLDER = "/home/ada/.zeron/playbooks"
local rows = {
	{ id = "triage", title = "Triage an issue", question = "What went wrong?", subtitle = "or run `zeron triage`", source = "builtIn" },
	{
		id = FOLDER .. "/release.md",
		title = "Cut a release",
		question = "Which version?",
		source = "app",
		path = FOLDER .. "/release.md",
		prompt = `Read [release.md]({FOLDER}/release.md) and follow it.\n\n`,
	},
	{
		id = "/code/app/.zeron/playbooks/deploy.md",
		title = "Deploy",
		question = "Where to?",
		source = "project",
		root = "/code/app",
		path = "/code/app/.zeron/playbooks/deploy.md",
		prompt = "Read [deploy.md](/code/app/.zeron/playbooks/deploy.md) and follow it.\n\n",
	},
}

page.host = {
	list = function(): any
		note("list")
		return settled({ folder = FOLDER, playbooks = table.clone(rows) })
	end,
	create = function(): any
		note("create")
		table.insert(rows, { id = FOLDER .. "/new-playbook.md", title = "New playbook", question = "What should we do?", source = "app", path = FOLDER .. "/new-playbook.md", prompt = "Read [new-playbook.md] and follow it.\n\n" })
		return settled(FOLDER .. "/new-playbook.md")
	end,
	triage = function(): any
		note("triage")
		return settled("Read [playbook.md](/home/ada/.zeron/triage/1/playbook.md) and follow it.\n\n")
	end,
	open = function(path: string): any
		note(`open:{path}`)
		return settled(nil)
	end,
	reveal = function(folder: string): any
		note(`reveal:{folder}`)
		return settled(nil)
	end,
	trash = function(path: string): any
		note(`trash:{path}`)
		for i, r in rows do
			if r.path == path then
				table.remove(rows, i)
				break
			end
		end
		return settled(nil)
	end,
}

return node
