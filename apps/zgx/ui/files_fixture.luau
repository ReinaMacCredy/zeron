-- The session page's files pane, mounted on its own over a disk made up
-- in ui/files_stub.luau: the pane tests/files_in_place.test.luau drives.
-- Here, beside app.luau, as a mounted script loads modules from its own
-- directory down.

ui.theme.strict("warn")
local ds = require("@ds")
ds.setup()
local stub = require("@app/files_stub")
local Files = require("@app/screens/files")
local Workspace = require("@app/screens/files/workspace")

-- The pane made again, as a reload of the scripts or a template replacing
-- it does (`test_files_generation` moved): its module's own tables gone,
-- its states kept.
local generation = ui.state("test_files_generation", 1)
local built_generation = generation:peek()

-- Text selected, as the host reads it (`read_selection`, which a fixture has
-- none of): a test sets `fake_selection` to `{ text, top, bottom }`.
local SelectionMenu = require("@ds/selection_menu")
local fake_selection = ui.state("fake_selection", nil :: any)
SelectionMenu.read = function(): any
	local s = fake_selection:peek()
	return if type(s) == "table" and type(s.text) == "string" and string.find(s.text, "%S") then s else nil
end
SelectionMenu.clear = function()
	fake_selection:set(nil)
end

-- The session open, as the shell's route gives it.
local session = ui.state("test_files_session", { id = "chat-1", cwd = "/proj" } :: any)

return ui.column {
	w_full = true,
	h_full = true,
	ui.presence(function(): string
		return tostring(generation:get())
	end, function(): Node
		if generation:peek() ~= built_generation then
			built_generation = generation:peek()
			Workspace.forget()
		end
		return Files.Pane {
			session = session,
			project = { id = "p1", path = "/proj", name = "proj" },
			-- (The chat page's Add to Chat, recorded with the host's calls.)
			on_add_to_chat = function(paths: { string }, root: string?)
				stub.note(`add:{table.concat(paths, "+")}@{root or ""}`)
			end,
			w_full = true,
			h_full = true,
		}
	end, { w_full = true, grow = 1, min_h = 0 }),
}
