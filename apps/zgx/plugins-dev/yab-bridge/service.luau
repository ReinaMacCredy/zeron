local zeron = require("./zeron")("yab-bridge")

local CLONE_PROMPT = "Read this repository and summarise what it does and how to build and run it. Do not run its build or install scripts without asking."
local root_store = zeron.store("clone_root", "")

local function text(value: any, name: string): string?
	if value == nil then
		return nil
	end
	if type(value) ~= "string" then
		error(`{name} must be text`, 0)
	end
	local trimmed = string.match(value, "^%s*(.-)%s*$") :: string
	return if trimmed == "" then nil else trimmed
end

local function good(part: string): boolean
	return string.match(part, "^[%w._-]+$") ~= nil and string.sub(part, 1, 1) ~= "-" and string.sub(part, 1, 1) ~= "."
end

local function parse_repo(repo: string): (string, string)
	local rest = string.match(repo, "^https://github%.com/(.+)$")
	local owner, name
	if rest then
		rest = string.gsub(rest, "/$", "")
		rest = string.gsub(rest, "%.git$", "")
		owner, name = string.match(rest, "^([^/]+)/([^/]+)$")
	end
	if not owner or not name or not good(owner) or not good(name) then
		error("Only a GitHub repository address is accepted, as https://github.com/<owner>/<name> (a .git ending or a trailing slash is fine)", 0)
	end
	return owner, name
end

local function names_of(projects: any): string
	local names = {}
	for _, p in projects do
		table.insert(names, p.name)
	end
	table.sort(names)
	return if #names == 0 then "(this device has no projects)" else table.concat(names, ", ")
end

local function find_project(projects: any, wanted: string?): any
	local list = names_of(projects)
	if not wanted then
		error(`Which Zeron project? Ask the person to pick one of: {list}`, 0)
	end
	if projects[wanted] then
		return projects[wanted]
	end
	local lower, named = string.lower(wanted), {}
	for _, p in projects do
		if string.lower(p.name) == lower then
			table.insert(named, p)
		end
	end
	if #named == 1 then
		return named[1]
	elseif #named > 1 then
		error(`More than one project is named "{wanted}". Ask the person to pick one of: {list}`, 0)
	end
	for _, p in projects do
		if p.path == wanted then
			return p
		end
	end
	error(`No project matches "{wanted}". Ask the person to pick one of: {list}`, 0)
end

local function clone(repo: string, given: string?): string
	local owner, name = parse_repo(repo)
	local root = string.gsub(given or root_store:get(), "/+$", "")
	if root == "" then
		error("No clone folder is set. Pass root (an absolute folder), or in Zeron open Settings, Yab bridge, and choose a Clone folder (a plugin cannot expand ~, so it must be absolute), then try again", 0)
	end
	local target = `{root}/{name}`
	local url = `https://github.com/{owner}/{name}`
	local ok, probe = pcall(function()
		return zeron.run("git", { "-C", target, "rev-parse", "--show-toplevel" }, { cwd = root, timeout = 20 }):await()
	end)
	if ok and probe.status == 0 and string.match(probe.stdout, "^%s*(.-)%s*$") == target then
		error(`{target} already exists, so nothing was cloned. Ask the person whether to use it as a project instead`, 0)
	end
	local ran = zeron.run("git", { "clone", "--", url, target }, { cwd = root, timeout = 240, env = { GIT_TERMINAL_PROMPT = "0" } }):await()
	if ran.status ~= 0 then
		error(`git clone failed: {ran.stderr}`, 0)
	end
	return target
end

zeron.serve("configure", function(args: any): any
	local root = if type(args) == "table" and type(args.clone_root) == "string" then args.clone_root else ""
	root_store:set(root)
	return { clone_root = root }
end, { safe = true })

zeron.serve("start", function(args: any, call: any): any
	args = args or {}
	local prompt, repo, wanted = text(args.prompt, "prompt"), text(args.repo, "repo"), text(args.project, "project")
	local given = text(args.root, "root")
	if given and string.sub(given, 1, 1) ~= "/" then
		error("root must be an absolute folder starting with /", 0)
	end
	if not repo then
		if not prompt then
			error("prompt is required when no repo is given", 0)
		end
		local project = find_project(zeron.projects():await(), wanted)
		local chat = zeron.new_chat({ project = project.id, sandbox = "read-only", prompt = prompt }):await()
		return { chat = chat.chat, project = project.id, cwd = project.path }
	end
	local target = clone(repo, given)
	local made, project = pcall(function()
		return zeron.new_project({ path = target }):await()
	end)
	if not made then
		error(`Cloned to {target}, but making it a project failed: {tostring(project)}. The folder is kept; open it in Zeron as a project`, 0)
	end
	local sent, chat = pcall(function()
		return zeron.new_chat({ project = project.id, sandbox = "read-only", prompt = prompt or CLONE_PROMPT }):await()
	end)
	if not sent then
		error(`Cloned to {target} and made it a project, but starting the chat failed: {tostring(chat)}`, 0)
	end
	return { chat = chat.chat, project = project.id, cwd = target }
end)
