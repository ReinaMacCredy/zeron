-- Settings → Playbooks, as the shipping page (crates/ui/src/shell/playbooks.rs):
-- each playbook the engine lists (`playbooks.list`: the built-in triage, this
-- Mac's folder, every local project's .zeron/playbooks) with where it comes
-- from and its file, Start (a new chat, in its project, its link first in the
-- draft), Edit and Delete (asked once more) for a file; New playbook in the
-- page's corner, the folder's note and Reveal folder under the card.

local ds = require("@ds")
local search = require("@app/screens/settings/search")
local route = require("@app/screens/shell/route")
local shared = require("@app/screens/chat/shared")
local sessions = require("@account/sessions")
local toast = require("@ds/toast")
local Button = require("@ds/button")

local M = {}

M.ID = "playbooks"
M.TITLE = "Playbooks"

search.index(M.ID, ui.t("settings.playbooks.title"), ui.t("settings.playbooks.terms"), {})

export type Playbook = {
	id: string,
	title: string,
	question: string,
	subtitle: string?,
	source: "builtIn" | "app" | "project",
	root: string?,
	path: string?,
	prompt: string?,
}
export type List = { state: "idle" | "loading" | "ready" | "failed", folder: string?, playbooks: { Playbook }, error: string? }

M.list = ui.state("playbooks_list", { state = "idle", playbooks = {} } :: List)
M.armed = ui.state("playbooks_armed", nil :: string?)

-- (src/playbooks.rs; a fixture answers them itself.)
M.host = {
	list = function(): any
		return ui.call("playbooks.list")
	end,
	create = function(): any
		return ui.call("playbooks.create")
	end,
	triage = function(): any
		return ui.call("playbooks.triage")
	end,
	open = function(path: string): any
		return ui.call("playbooks.open", path)
	end,
	reveal = function(folder: string): any
		return ui.call("playbooks.reveal", folder)
	end,
	trash = function(path: string): any
		return ui.call("playbooks.trash", path)
	end,
}

-- (An older host has no such action.)
local function call(name: string, arg: any?): any
	local ok, p = pcall((M.host :: any)[name], arg)
	return if ok then p else nil
end

local function says(e: any): string
	local why = if e and e.message then tostring(e.message) else "failed"
	return if why == "outdated" then ui.t("settings.playbooks.outdated"):get() else why
end

function M.load()
	local p = call("list")
	if p == nil then
		M.list:set({ state = "failed", playbooks = {}, error = ui.t("settings.playbooks.outdated"):get() })
		return
	end
	if M.list:peek().state ~= "ready" then
		M.list:set({ state = "loading", playbooks = {} })
	end
	p:after(function(v: any)
		M.list:set({ state = "ready", folder = v and v.folder, playbooks = (v and v.playbooks) or {} })
	end, function(e: any)
		M.list:set({ state = "failed", playbooks = {}, error = says(e) })
	end)
end

local function failed(title: string, e: any)
	toast.show { title = ui.t(title):get(), body = says(e), tone = "danger" }
end

-- Where it comes from: its project's name, else the shipping page's words.
function M.source(p: Playbook): string
	if p.source == "project" then
		for _, project in sessions.now:peek().projects or {} do
			if project.path == p.root then
				return project.name
			end
		end
		return p.root or ""
	end
	return ui.t(if p.source == "builtIn" then "settings.playbooks.built_in" else "settings.playbooks.everywhere"):get()
end

local function file_name(path: string?): string?
	return if path then string.match(path, "[^/]+$") else nil
end

local function seed(project: string?, prompt: string)
	route.new_chat(project)
	shared.set_draft_of("", "text", prompt .. shared.draft_peek("").text)
	ui.after(0, function()
		shared.focus:set(shared.focus:peek() + 1)
	end)
end

function M.start(p: Playbook)
	if p.source == "builtIn" then
		local started = call("triage")
		if started then
			started:after(function(prompt: any)
				seed(nil, tostring(prompt))
			end, function(e: any)
				failed("settings.playbooks.couldnt_start", e)
			end)
		end
		return
	end
	if not p.prompt then
		return
	end
	local project: string? = nil
	if p.source == "project" then
		for _, candidate in sessions.now:peek().projects or {} do
			if candidate.path == p.root then
				project = candidate.id
			end
		end
	end
	seed(project, p.prompt)
end

function M.create()
	local p = call("create")
	if p == nil then
		return
	end
	p:after(function()
		M.load()
	end, function(e: any)
		failed("settings.playbooks.couldnt_create", e)
	end)
end

function M.delete(p: Playbook)
	if p.path == nil then
		return
	end
	if M.armed:peek() ~= p.path then
		M.armed:set(p.path)
		return
	end
	M.armed:set(nil)
	local done = call("trash", p.path)
	if done == nil then
		return
	end
	done:after(function()
		M.load()
	end, function(e: any)
		failed("settings.playbooks.couldnt_delete", e)
	end)
end

local function action(id: string, words: any, tone: Button.Tone, run: () -> ()): Node
	return ds.Button {
		id = id,
		tone = tone,
		size = "sm",
		on_click = run,
		ui.text { words },
	}
end

local function row(p: Playbook, i: State<number>): Node
	local key = string.gsub(p.id, "[^%w]", "-")
	local meta = M.source(p)
	local name = file_name(p.path)
	if name then
		meta = `{meta} · {name}`
	end
	local controls: { Node } = {}
	if p.path then
		table.insert(controls, action(`playbook-edit-{key}`, ui.t("settings.playbooks.edit"), "ghost", function()
			local opened = call("open", p.path)
			if opened then
				opened:after(function() end, function(e: any)
					failed("settings.playbooks.couldnt_open", e)
				end)
			end
		end))
		table.insert(controls, ds.Button {
			id = `playbook-delete-{key}`,
			tone = "ghost",
			size = "sm",
			on_click = function()
				M.delete(p)
			end,
			ui.text {
				function(): string
					return ui.t(if M.armed:get() == p.path then "settings.playbooks.confirm_delete" else "settings.playbooks.delete"):get()
				end,
				color = function(): string
					return if M.armed:get() == p.path then "$danger" else "$text.primary"
				end,
			},
		})
	end
	if p.prompt or p.source == "builtIn" then
		table.insert(controls, action(`playbook-start-{key}`, ui.t("settings.playbooks.start"), "secondary", function()
			M.start(p)
		end))
	end
	return ds.Settings.Row {
		label = p.title,
		description = meta,
		icon = if p.source == "builtIn" then "action-debug" else "document",
		truncate = true,
		control = ui.row { gap = "$space.2", items = "center", controls },
	}
end

function M.view(): Node
	if M.list:peek().state == "idle" then
		M.load()
	end
	return ds.Settings.Page {
		title = ui.t("settings.playbooks.title"),
		description = ui.t("settings.playbooks.description"),
		trail = ds.Button {
			id = "new-playbook",
			tone = "primary",
			size = "sm",
			on_click = M.create,
			ui.text { ui.t("settings.playbooks.new") },
		},
		ds.Settings.Section {
			ui.show(function(): boolean
				return M.list:get().state == "failed"
			end, ui.text {
				id = "playbooks-error",
				function(): string
					return M.list:get().error or ""
				end,
				color = "$text.secondary",
			}),
			ds.Settings.Each(ui.computed(function(): { Playbook }
				return M.list:get().playbooks
			end), function(p: Playbook, i: State<number>): Node
				return row(p, i)
			end, { key = function(p: Playbook): string
				return p.id
			end }),
		},
		ds.Settings.Section {
			ui.row {
				w_full = true,
				gap = "$space.3",
				items = "start",
				ui.text {
					id = "playbooks-note",
					grow = 1,
					min_w = 0,
					wrap = true,
					color = "$text.secondary",
					function(): string
						return ui.t("settings.playbooks.note", { folder = M.list:get().folder or "" }):get()
					end,
				},
				action("playbooks-reveal-folder", ui.t("settings.playbooks.reveal"), "secondary", function()
					local folder = M.list:peek().folder
					local shown = folder and call("reveal", folder)
					if shown then
						shown:after(function() end, function(e: any)
							failed("settings.playbooks.couldnt_open", e)
						end)
					end
				end),
			},
		},
	}
end

return M
