-- Quick capture inside the window (crates/ui quick_capture.rs): no native panel here.

local ds = require("@ds")
local locale = require("@ds/locale")
local mentions = require("@ds/composer_mentions")
local store = require("@app/screens/tasks/store")
local card = require("@app/screens/tasks/card")
local glass = require("@ds/glass")
local chat_store = require("@app/screens/chat/store")
local Completions = require("@app/screens/chat/completions")
local Invocations = require("@app/screens/chat/invocations")

local M = {}

local CARD_W = 640
local TASK_LIMIT, TASK_IDLE_LIMIT, DEPENDS_MAX = 8, 5, 20
local TASK_COLUMNS = { todo = "tasks.column.todo", working = "tasks.column.working", needs = "tasks.column.needs", done = "tasks.column.done" }
local TASK_GLYPHS = { todo = "check-circle", working = "refresh", needs = "danger-triangle", done = "check" }

M.open = ui.state("tasks_capture_open", false)
M.text = ui.state("tasks_capture_text", "")
M.images = ui.state("tasks_capture_images", {} :: { string })
M.uploading = ui.state("tasks_capture_uploading", 0)
local save_waiting = false

ds.toast.action("tasks.capture.undo", function(id: any)
	if type(id) == "string" then
		store.delete(id)
	end
end)

function M.toggle()
	M.open:set(not M.open:peek())
end

-- The first line the title, the rest the notes (quick_capture.rs `split_capture`).
function M.split(text: string): (string, string)
	local title, rest = string.match(text, "^([^\n]*)\n?(.*)$")
	return string.match(title or "", "^%s*(.-)%s*$") or "", string.match(rest or "", "^%s*(.-)%s*$") or ""
end

local function depends(words: string): { string }
	local known = store.parts:peek().by_id
	local out: { string } = {}
	for id in string.gmatch(words, "%]%(zeron%-task:([^%)%s]+)%)") do
		if known[id] and not table.find(out, id) and #out < DEPENDS_MAX then
			table.insert(out, id)
		end
	end
	return out
end

local function token_of(title: string): string
	local words: { string } = {}
	local size = 0
	for word in string.gmatch(title, "[^%s%p]+") do
		if #words > 0 and size + #word > 32 then
			break
		end
		table.insert(words, word)
		size += #word + 1
	end
	return if #words > 0 then table.concat(words, "-") else "task"
end

local function task_rows(query: string): { any }
	local scoped = string.lower(string.sub(query, 1, 5)) == "task "
	local q = string.lower(string.match(if scoped then string.sub(query, 6) else query, "^%s*(.-)%s*$") or "")
	local p = store.parts:get()
	local found: { store.Task } = {}
	local function take(list: { store.Task })
		for _, t in list do
			if q == "" or string.find(string.lower(store.title_of(t)), q, 1, true) then
				table.insert(found, t)
			end
		end
	end
	take(p.open)
	local limit = TASK_IDLE_LIMIT
	if q ~= "" or scoped then
		take(p.done)
		limit = TASK_LIMIT
	end
	local out: { any } = {}
	local used: { [string]: boolean } = {}
	for i, t in found do
		if i > limit then
			break
		end
		local title = store.title_of(t)
		local word = token_of(title)
		if used[word] then
			word = `{word}-{string.sub(t.id, 1, 4)}`
		end
		used[word] = true
		local column = store.state(t).column
		local where = store.project_name(t.project) or locale.s("tasks.no_project")
		table.insert(out, {
			value = word,
			key = `task-{t.id}`,
			label = title,
			detail = if column == "todo" then where else `{locale.s(TASK_COLUMNS[column])} · {where}`,
			icon = TASK_GLYPHS[column],
			link = `[{(string.gsub(title, "([%[%]\\])", "\\%1"))}](zeron-task:{t.id})`,
			searched = true,
		})
	end
	return out
end

local function upload_done()
	M.uploading:set(M.uploading:peek() - 1)
	if M.uploading:peek() == 0 and save_waiting then
		save_waiting = false
		M.save()
	end
end

function M.add_files(files: { any })
	for _, f in files do
		local name = f.name or string.match(f.path or "", "[^/]+$") or "image"
		local ext = string.lower(string.match(name, "%.(%w+)$") or "")
		if not table.find({ "png", "jpg", "jpeg", "webp", "gif" }, ext) then
			ds.toast.show({ title = locale.s("tasks.capture.not_image", { name = name }), tone = "warning" })
		elseif #M.images:peek() + M.uploading:peek() >= store.IMAGES_MAX then
			ds.toast.show({ title = locale.s("tasks.card.images_max", { n = store.IMAGES_MAX }), tone = "warning" })
			return
		else
			local p = store.host.upload({ path = f.path })
			if p then
				M.uploading:set(M.uploading:peek() + 1)
				p:after(function(path: string)
					store.seed_image(path, f.path)
					local next = table.clone(M.images:peek())
					table.insert(next, path)
					M.images:set(next)
					upload_done()
				end, function()
					ds.toast.show({ title = locale.s("tasks.card.image_failed", { name = name }), tone = "danger" })
					upload_done()
				end)
			end
		end
	end
end

function M.save(): boolean
	local typed = M.text:peek()
	local words = Invocations.encode(typed)
	local title, notes = M.split(words)
	if title == "" then
		return false
	end
	if M.uploading:peek() > 0 then
		save_waiting = true
		return true
	end
	local images = M.images:peek()
	ui.batch(function()
		M.text:set("")
		M.images:set({})
		M.open:set(false)
	end)
	store.create({ title = title, notes = notes, images = images, depends = depends(words) }, function(id: string)
		store.note_captured(id)
		ds.toast.show({
			title = locale.s("tasks.capture.added", { title = store.plain(title) }),
			tone = "success",
			action = { label = locale.s("tasks.capture.undo"), run = "tasks.capture.undo", value = id },
		})
	end, function()
		-- (Not saved: what was typed comes back, for another go.)
		ui.batch(function()
			if M.text:peek() == "" then
				M.text:set(typed)
			end
			if #M.images:peek() == 0 then
				M.images:set(images)
			end
			M.open:set(true)
		end)
	end)
	return true
end

local function Hint(key: string, word: string): Node
	return ui.row {
		gap = ui.raw(4, "gpui"),
		items = "center",
		ds.Kbd { text = key },
		ui.text { locale.s(word) },
	}
end

local function Images(): Node
	return ui.presence(function(): string?
		local list = M.images:get()
		return if #list > 0 then table.concat(list, "\n") else nil
	end, function(): Node
		local list = M.images:peek()
		local row: { [any]: any } = { id = "quick-capture-images", gap = ui.raw(6, "gpui"), wrap = true }
		for _, thumb in card.thumbs(list, "quick-capture", "quick-capture", function(i: number)
			local next = table.clone(M.images:peek())
			table.remove(next, i)
			M.images:set(next)
		end) do
			table.insert(row, thumb)
		end
		return ui.row(row)
	end, { w_full = true })
end

local Body = ui.component(function(): Node
	local field = ui.ref()
	local c = Completions.bind {
		key = "tasks_capture",
		text = M.text,
		agent = function(): string?
			local m = chat_store.new_chat_model()
			return if m then m.harness else nil
		end,
		target = function(): Invocations.Target?
			return { home = true }
		end,
		files = function(): any
			return nil
		end,
		in_chat = function(): boolean
			return false
		end,
		run = function() end,
		task = true,
	}
	local sources = ui.computed(function(): any
		local given = table.clone(c.lists:get() or {})
		local text = M.text:get()
		local t = mentions.token(text, #text)
		given["@"] = if t and t.trigger == "@" then task_rows(t.query) else {}
		return given
	end)
	local m = mentions.use {
		field = field,
		value = M.text,
		sources = sources,
		titles = { ["@"] = locale.s("tasks.capture.tasks") },
		empty = setmetatable({ ["@"] = locale.s("tasks.capture.what_tasks") }, { __index = c.empty }) :: any,
		enabled = c.enabled,
		on_choose = c.on_mention,
	}
	local input = ui.textarea {
		ref = field,
		id = "quick-capture-input",
		label = locale.s("tasks.capture.placeholder"),
		placeholder = locale.s("tasks.capture.placeholder"),
		placeholder_color = "$text.tertiary",
		value = M.text,
		highlight = mentions.highlight,
		active_descendant = m.descendant,
		on_caret = m.on_caret,
		on_blur = m.on_blur,
		on_press = m.on_press,
		on_paste = function(e: any): boolean
			if e.files and #e.files > 0 then
				M.add_files(e.files)
				return true
			end
			if e.text == nil then
				chat_store.paste_image():after(function(path: string?)
					if type(path) == "string" and path ~= "" then
						M.add_files({ { name = "image.png", path = path } })
					end
				end, function() end)
				return true
			end
			return false
		end,
		on_key = function(e: any): boolean
			if m.keys(e) then
				return true
			end
			if e.key == "Enter" and not e.shift then
				M.save()
				return true
			elseif e.key == "Escape" then
				M.open:set(false)
				return true
			end
			return false
		end,
		autofocus = true,
		wrap = true,
		w_full = true,
		min_h = 26,
		px = 0,
		border = 0,
		bg = "#00000000",
		focus = { border = 0, outline = 0 },
		text_size = ui.raw(17, "gpui"),
	}
	return ui.column {
		id = "quick-capture",
		role = "dialog",
		label = locale.s("tasks.capture.title"),
		w_full = true,
		px = ui.raw(18, "gpui"),
		py = ui.raw(16, "gpui"),
		gap = ui.raw(12, "gpui"),
		on_file_drop = M.add_files,
		ui.popover(m.popover(input)),
		Images(),
		ui.row {
			gap = ui.raw(14, "gpui"),
			items = "center",
			text_size = ui.raw(11.5, "gpui"),
			color = "$text.secondary",
			Hint("⏎", "tasks.capture.save"),
			Hint("⇧⏎", "tasks.capture.notes"),
			Hint("esc", "tasks.capture.close"),
			ui.div { grow = 1 },
			ui.text { locale.s("tasks.capture.where") },
		},
	}
end)

M.Overlay = ui.component(function(): Node
	return ui.modal(glass.style({
		open = function(): boolean
			return M.open:get()
		end,
		on_close = function()
			M.open:set(false)
		end,
		label = locale.s("tasks.capture.title"),
		placement = { x = "center", y = "start", inset = 150 },
		w = CARD_W,
		p = 0,
		rounded = ui.raw(18, "gpui"),
		build = function(): Node
			return Body {}
		end,
	}, "menu"))
end)

return M
