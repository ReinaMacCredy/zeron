-- No mock turn writes these rows; src/chat.rs's tests prove entries come out in this shape.

local function system(id: string, line: string, text: string, title: string?): any
	return { id = id, role = "system", line = line, text = text, title = title }
end

local MESSAGES = {
	system("s1", "classifier", "Classifier: same task"),
	system("s2", "fork", "", "Old chat"),
	system("s3", "handoff", "", "Older chat"),
	system("s4", "handoff", ""),
	{
		id = "a1",
		role = "assistant",
		text = "",
		blocks = {
			{ id = "i1", kind = "line", line = "input", title = "Pick", outcome = "pending" },
			{ id = "i2", kind = "line", line = "input", text = "Pick → A", outcome = "answered", answers = { { label = "Pick", answer = "A" } } },
			{ id = "i3", kind = "line", line = "secret", title = "API key", status = "declined" },
			{ id = "i4", kind = "line", line = "fork", title = "Reply fork" },
			{ id = "i5", kind = "line", line = "switch", from = "Claude", to = "Codex" },
		},
	},
}

local WORDS = {
	"Classifier: same task",
	"Forked from Old chat",
	"Continued from Older chat",
	"Context handoff",
	"Awaiting your answer…",
	"Pick → A",
	"Declined",
	"Forked from Reply fork",
	"Switched from Claude to Codex",
}

test("every host-written row and reply line draws its words", function()
	local app = mount("display_tools_fixture.luau")
	app:resize(900, 900)
	app:set_state("transcript_messages", MESSAGES)
	app:frames(60)
	expect(app:error()).to_be_nil()
	local tree = app:tree("#transcript")
	for _, words in WORDS do
		expect(tree:find(words, 1, true) ~= nil).to_be(true)
	end
end)
