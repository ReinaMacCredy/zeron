-- The chat page (ui/chat_fixture.luau) with the host's file calls stubbed
-- (screens/chat/shared's `ask`, ds/pasted_text's `read`), for
-- tests/chat_paste.test.luau and the stage checks in tests/chat_drop.test.luau.
-- Asked through `test_paste_*` states and the commands below.

local page = require("./chat_fixture")
local shared = require("@app/screens/chat/shared")
local PastedText = require("@ds/pasted_text")
local References = require("@ds/references")

local host = ui.state("test_host", "new")
local stats = ui.state("test_stats", {} :: { any })
local read_fails = ui.state("test_read_fails", false)
local reads = ui.state("test_reads", 0)
local length = ui.state("test_paste_length", 0)
local unit = ui.state("test_paste_unit", "a")
local words = ui.state("test_paste_words", "")
local answer = ui.state("test_paste_answer", "")
local label = ui.state("test_paste_label", "")
local size = ui.state("test_size", "")
local chip = ui.state("test_chip", "")
local mention = ui.state("test_mention", "")
local files = ui.state("test_files", {} :: { any })
local written: { [string]: string } = {}

local function settled(value: any): any
	return {
		after = function(self: any, ok: (any) -> (), _err: any)
			ok(value)
		end,
	}
end
local function rejected(why: string): any
	return {
		after = function(self: any, _ok: any, err: (any) -> ())
			err(why)
		end,
	}
end

local late: ((any) -> ())? = nil
local read_late = ui.state("test_read_late", false)
local reads_late: { () -> () } = {}
local stub = shared :: any
stub.ask = function(action: string, arg: any): any?
	if host:peek() == "old" then
		return nil
	end
	if action == "chat.paste_text" then
		if host:peek() == "write_fails" then
			return rejected("failed")
		end
		local path = `/tmp/pasted/{arg.name}`
		written[path] = arg.text
		return settled(path)
	elseif action == "chat.stat_files" then
		if host:peek() == "late" then
			return {
				after = function(self: any, ok: (any) -> (), _err: any)
					late = ok
				end,
			}
		end
		return settled(stats:peek())
	elseif action == "chat.read_text" then
		reads:set(reads:peek() + 1)
		if read_fails:peek() or not written[arg.path] then
			return rejected("unreadable")
		end
		if read_late:peek() then
			return {
				after = function(self: any, ok: (any) -> (), _err: any)
					table.insert(reads_late, function()
						ok({ text = written[arg.path], lines = 1 })
					end)
				end,
			}
		end
		return settled({ text = written[arg.path], lines = 1 })
	end
	return nil
end
local reader = PastedText :: any
reader.read = function(path: string, _session: string?): any
	if read_fails:peek() then
		return rejected("unreadable")
	end
	return settled({ text = "first\nsecond\nthird\n", lines = 3 })
end

ui.command {
	id = "paste_now",
	title = "Paste test_paste_length of test_paste_unit, or test_paste_words",
	run = function()
		local text = if length:peek() > 0 then string.rep(unit:peek(), length:peek()) else words:peek()
		answer:set(tostring(shared.paste_text(text)))
	end,
}
ui.command {
	id = "paste_at_caret",
	title = "Paste test_paste_length of test_paste_unit through the composer's field",
	run = function()
		local text = string.rep(unit:peek(), length:peek())
		answer:set(tostring(shared.paste_text(text, nil, function(shown: string): boolean
			local el = References.field(shared.refs_key(shared.showing()))
			return el ~= nil and el:replace_selection(shown)
		end)))
	end,
}
ui.command {
	id = "label_now",
	title = "Label test_paste_words",
	run = function()
		label:set(shared.paste_label(words:peek()) or "<none>")
	end,
}
ui.command {
	id = "size_now",
	title = "Say 40 MiB and 1.5 GiB",
	run = function()
		size:set(`{shared.size_label(40 * 1024 * 1024)}|{shared.size_label(1.5 * 1024 * 1024 * 1024)}`)
	end,
}
ui.command {
	id = "open_first_pasted",
	title = "Press the first pasted chip",
	run = function()
		for _, item in shared.cargo:peek() do
			if item.pasted then
				shared.open_pasted(item.id)
				shared.open_pasted(item.id)
				return
			end
		end
	end,
}
ui.command {
	id = "answer_reads",
	title = "The host answers the reads it was asked",
	run = function()
		local waiting = reads_late
		reads_late = {}
		for _, answered in waiting do
			answered()
		end
	end,
}
ui.command {
	id = "open_pasted_at_caret",
	title = "Press the first pasted chip, the composer taking it",
	run = function()
		for _, item in shared.cargo:peek() do
			if item.pasted then
				shared.open_pasted(item.id, function(text: string): boolean
					chip:set(`caret:{#text}`)
					return true
				end)
				return
			end
		end
	end,
}
ui.command {
	id = "remove_first_pasted",
	title = "Remove the first pasted chip",
	run = function()
		for _, item in shared.cargo:peek() do
			if item.pasted then
				shared.remove_cargo(item.id)
				return
			end
		end
	end,
}

local failed_text: string? = nil
local failed_cargo: { any }? = nil
ui.command {
	id = "take_now",
	title = "Take the draft, as a send does",
	run = function()
		failed_text, failed_cargo = shared.take()
		local names = {}
		for _, item in failed_cargo do
			table.insert(names, item.name)
		end
		ui.state("test_taken", nil :: any):set({ text = failed_text, names = names })
	end,
}
ui.command {
	id = "restore_now",
	title = "Hand back what was taken, as a failed send does",
	run = function()
		shared.restore(failed_text or "", failed_cargo or {})
	end,
}
ui.command {
	id = "stage_more",
	title = "Stage a file",
	run = function()
		shared.add_to_chat({ "later.rs" }, "/tmp/atlas")
	end,
}
ui.command {
	id = "attach_test_files",
	title = "Attach test_files",
	run = function()
		shared.attach_files(files:peek())
	end,
}
ui.command {
	id = "paste_image_now",
	title = "Paste the clipboard's picture into the composer",
	run = function()
		shared.cargo_props().on_paste_image()
	end,
}
ui.command {
	id = "answer_late",
	title = "The host answers the stage it was asked",
	run = function()
		if late then
			late({ { path = "/x/a.txt", size = 1, is_file = true, readable = true } })
			late = nil
		end
	end,
}
ui.command {
	id = "draft_of_s2",
	title = "Copy s2's draft to test_chip",
	run = function()
		local d = shared.draft_peek("s2")
		local names: { string } = {}
		for _, item in d.cargo do
			table.insert(names, item.name)
		end
		chip:set(`{d.text}|{table.concat(names, ",")}`)
	end,
}
ui.command {
	id = "draft_of_s1",
	title = "Copy s1's draft to test_chip",
	run = function()
		local d = shared.draft_peek("s1")
		local names: { string } = {}
		for _, item in d.cargo do
			table.insert(names, item.name)
		end
		chip:set(`{d.text}|{table.concat(names, ",")}`)
	end,
}
ui.command {
	id = "mention_shown",
	title = "Mention the chat shown",
	run = function()
		shared.mention_chat(shared.showing())
		mention:set(shared.refused:peek() or "")
	end,
}
ui.command {
	id = "open_viewer",
	title = "Open the pasted text viewer",
	run = function()
		PastedText.open { title = "Pasted text 2", path = "/tmp/pasted/x.txt" }
	end,
}
local decoded = ui.state("test_decoded", nil :: any)
local decode_text = ui.state("test_decode_text", "")
ui.command {
	id = "decode_now",
	title = "Cut test_decode_text into blocks",
	run = function()
		local blocks, lines = PastedText.decode(decode_text:peek())
		local most, valid = 0, true
		for _, block in blocks do
			most = math.max(most, #block)
			valid = valid and utf8.len(block) ~= nil
		end
		decoded:set { count = #blocks, lines = lines, most = most, valid = valid, first = blocks[1], last = blocks[#blocks] }
	end,
}
local number_in = ui.state("test_number_of", "")
local number_out = ui.state("test_number", 0)
ui.command {
	id = "number_now",
	title = "The pasted number of test_number_of",
	run = function()
		number_out:set(PastedText.number_of(number_in:peek()) or 0)
	end,
}
local function images(count: number, kind: string)
	local files: { any } = {}
	for i = 1, count do
		table.insert(files, { name = if kind == "image" then `p{i}.png` else `f{i}.txt`, path = if kind == "image" then "assets/favicons/zgx.png" else `/tmp/f{i}.txt` })
	end
	shared.attach_files(files)
end
ui.command {
	id = "attach_20_images",
	title = "Attach twenty pictures",
	run = function()
		images(20, "image")
	end,
}
ui.command {
	id = "attach_9_files",
	title = "Attach nine files",
	run = function()
		images(9, "file")
	end,
}

return ui.div {
	w_full = true,
	h_full = true,
	page,
	PastedText.Host(),
}
