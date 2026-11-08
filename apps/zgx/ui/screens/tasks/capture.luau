-- Quick capture inside the window (crates/ui quick_capture.rs): no native panel here.

local ds = require("@ds")
local locale = require("@ds/locale")
local store = require("@app/screens/tasks/store")

local M = {}

local CARD_W = 640

M.open = ui.state("tasks_capture_open", false)
M.text = ui.state("tasks_capture_text", "")

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

function M.save(): boolean
	local title, notes = M.split(M.text:peek())
	if title == "" then
		return false
	end
	ui.batch(function()
		M.text:set("")
		M.open:set(false)
	end)
	store.create({ title = title, notes = notes }, function(id: string)
		store.note_captured(id)
		ds.toast.show({
			title = locale.s("tasks.capture.added", { title = title }),
			tone = "success",
			action = { label = locale.s("tasks.capture.undo"), run = "tasks.capture.undo", value = id },
		})
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

M.Overlay = ui.component(function(): Node
	return ui.modal {
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
			return ui.column {
				id = "quick-capture",
				role = "dialog",
				label = locale.s("tasks.capture.title"),
				w_full = true,
				px = ui.raw(18, "gpui"),
				py = ui.raw(16, "gpui"),
				gap = ui.raw(12, "gpui"),
				ui.textarea {
					id = "quick-capture-input",
					label = locale.s("tasks.capture.placeholder"),
					placeholder = locale.s("tasks.capture.placeholder"),
					placeholder_color = "$text.tertiary",
					value = function(): string
						return M.text:get()
					end,
					on_change = function(v: string)
						M.text:set(v)
					end,
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
					w_full = true,
					min_h = 26,
					px = 0,
					border = 0,
					bg = "#00000000",
					focus = { border = 0, outline = 0 },
					text_size = ui.raw(17, "gpui"),
				},
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
		end,
	}
end)

return M
