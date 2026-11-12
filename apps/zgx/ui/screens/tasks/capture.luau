-- Quick capture inside the window (crates/ui quick_capture.rs): no native panel here.

local ds = require("@ds")
local locale = require("@ds/locale")
local store = require("@app/screens/tasks/store")
local card = require("@app/screens/tasks/card")
local glass = require("@ds/glass")
local references = require("@ds/references")
local field_kit = require("@app/screens/tasks/field")

local M = {}

local CARD_W = 640

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
	local words = references.encode("capture", typed)
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
	store.create({ title = title, notes = notes, images = images, depends = field_kit.depends(words) }, function(id: string)
		if M.text:peek() == "" then
			references.clear("capture")
		end
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

local function capture_key(): string
	return "capture"
end

local Body = ui.component(function(): Node
	local f = field_kit.use {
		key = "tasks_capture",
		refs = capture_key,
		text = M.text,
		field = ui.ref(),
		line = 24,
		on_files = M.add_files,
	}
	local input = f.textarea {
		id = "quick-capture-input",
		label = locale.s("tasks.capture.placeholder"),
		placeholder = locale.s("tasks.capture.placeholder"),
		placeholder_color = "$text.tertiary",
		on_key = function(e: any): boolean
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
		input,
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
