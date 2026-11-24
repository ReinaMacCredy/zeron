-- The model picker's reasoning follows the model in place: moving between
-- models that both have levels keeps the slider and its title the same nodes,
-- its levels the new model's. (It was built again for each model, its field and
-- thumb starting over.) And a model's options (Devin Fusion's lead and
-- sidekick) sit under its reasoning, with no card beside its row. On the picker
-- in ui/picker_fixture.luau.

-- A node's identity, by its kind and label (or text): `@id`.
local function node(app: any, kind: string?, label: string): string?
	local pattern = if kind then ` {kind} "{label}"` else ` "{label}"`
	for line in app:tree():gmatch("[^\n]+") do
		if line:find(pattern, 1, true) then
			return line:match("@(%d+)")
		end
	end
	return nil
end

-- Where a node sits in the tree (its line), by what its line holds.
local function line_of(app: any, text: string): number?
	local n = 0
	for line in app:tree():gmatch("[^\n]+") do
		n += 1
		if line:find(text, 1, true) and not line:find(" hidden", 1, true) then
			return n
		end
	end
	return nil
end

local function settle(app: any)
	app:frames(30)
end

local function options(app: any): { [string]: string }
	return (app:state("test_picker_options") or {}) :: { [string]: string }
end

test("moving between models keeps the reasoning slider", function()
	local app = mount("picker_fixture.luau")
	app:resize(600, 700)
	settle(app)
	app:click('role:button "Model"')
	settle(app)
	local groove = node(app, nil, "Reasoning")
	local title = app:tree("#model-picker-title"):match("@(%d+)")
	expect(groove).never.to_be_nil()

	-- Another model with the same levels, then one with five.
	app:set_state("test_picker_model", "opus")
	settle(app)
	expect(`groove @{node(app, nil, "Reasoning")}`).to_be(`groove @{groove}`)
	expect(`title @{app:tree("#model-picker-title"):match("@(%d+)")}`).to_be(`title @{title}`)
	app:set_state("test_picker_model", "gpt")
	settle(app)
	expect(`groove @{node(app, nil, "Reasoning")}`).to_be(`groove @{groove}`)
	expect(app:state("test_picker_effort")).to_be("medium")
	-- Its levels are the new model's: End reaches its fifth.
	app:key("End")
	settle(app)
	expect(app:state("test_picker_effort")).to_be("xhigh")
	expect(app:wait('"Extra high"', { timeout_ms = 2000 })).to_be(true)

	-- A model without levels has none; back to one with them, a slider.
	app:set_state("test_picker_model", "mini")
	settle(app)
	expect(line_of(app, '"Reasoning"')).to_be_nil()
	app:set_state("test_picker_model", "sonnet")
	settle(app)
	expect(line_of(app, '"Reasoning"')).never.to_be_nil()
end)

-- The chosen model's options sit under its reasoning (the lead first): each a
-- row opening its choices beside the panel,
-- the choice made shown on it. A pick is kept by option id; the option's
-- own start is no pick.
test("a model's options sit under its reasoning, and are picked beside it", function()
	local app = mount("picker_fixture.luau")
	app:resize(700, 700)
	app:set_state("test_picker_model", "fusion")
	settle(app)
	app:click('role:button "Model"')
	settle(app)
	local lead = line_of(app, 'button "Lead"')
	local sidekick = line_of(app, 'button "Sidekick"')
	local reasoning = line_of(app, '"Reasoning"')
	expect(lead).never.to_be_nil()
	expect(sidekick).never.to_be_nil()
	expect(reasoning).never.to_be_nil()
	expect((lead :: number) < (sidekick :: number)).to_be(true)
	expect((reasoning :: number) < (lead :: number)).to_be(true)
	-- The lead's choices open beside the row, its start tagged; a pick.
	app:click('role:button "Lead"')
	settle(app)
	expect(app:has('role:option ~"Fable 5.1" >> ~"Default"')).to_be(true)
	app:click('role:option ~"GPT-6 Sol"')
	settle(app)
	expect(options(app).lead).to_be("sol")
	expect(app:has('role:button "Lead" >> ~"GPT-6 Sol"')).to_be(true)
	-- Back to its start: no pick.
	app:click('role:button "Lead"')
	settle(app)
	app:click('role:option ~"Fable 5.1"')
	settle(app)
	expect(options(app).lead).to_be(nil)
	-- A model without options has no rows.
	app:set_state("test_picker_model", "sonnet")
	settle(app)
	expect(line_of(app, 'button "Lead"')).to_be_nil()
	expect(line_of(app, '"Reasoning"')).never.to_be_nil()
	expect(app:error()).to_be(nil)
end)

-- An option row opens its choices on hover, as Zeron's do with none open, and
-- a press on the row then keeps them open (it doesn't toggle them shut).
test("an option's choices open on hover, and a press keeps them", function()
	local app = mount("picker_fixture.luau")
	app:resize(700, 700)
	app:set_state("test_picker_model", "fusion")
	settle(app)
	app:click('role:button "Model"')
	settle(app)
	expect(line_of(app, 'option "F Fable 5.1')).to_be_nil()
	app:hover('role:button "Lead"')
	settle(app)
	expect(line_of(app, 'option "F Fable 5.1')).never.to_be_nil()
	app:click('role:button "Lead"')
	settle(app)
	expect(line_of(app, 'option "F Fable 5.1')).never.to_be_nil()
	-- A second press puts it away, as before.
	app:click('role:button "Lead"')
	settle(app)
	expect(line_of(app, 'option "F Fable 5.1')).to_be_nil()
	app:click('role:button "Sidekick"')
	settle(app)
	expect(line_of(app, 'option "S SWE-2 Medium')).never.to_be_nil()
	expect(app:error()).to_be(nil)
end)

-- Devin Fusion has no card beside its row: picked, its options are rows on
-- the panel, as any model's are.
test("Fusion is picked like any model, its options then under it", function()
	local app = mount("picker_fixture.luau")
	app:resize(700, 700)
	app:click('role:button "Model"')
	settle(app)
	-- The list (a press on the panel's title), at Devin's group.
	app:click("#model-picker-title")
	settle(app)
	app:click('role:button "Jump to Devin"')
	settle(app)
	-- Resting on Fusion opens nothing beside it.
	app:hover('role:option ~"Fusion"')
	app:wait("#a-moment-passes", { timeout_ms = 400 })
	settle(app)
	expect(line_of(app, '"Fusion options"')).to_be_nil()
	-- Picked, back on the panel, its Lead and Sidekick under its reasoning.
	app:click('role:option ~"Fusion"')
	settle(app)
	expect(app:state("test_picker_model")).to_be("fusion")
	if not app:has('role:button "Lead"') then
		app:click('role:button "Model"')
		settle(app)
	end
	local lead = line_of(app, 'button "Lead"')
	local sidekick = line_of(app, 'button "Sidekick"')
	expect(lead).never.to_be_nil()
	expect(sidekick).never.to_be_nil()
	expect(app:error()).to_be(nil)
end)

-- Moving between the full layout's tabs keeps the panel as it is: the same
-- panel, the same size every frame, only its list changing. (The list's
-- window took each tab's height, easing to it: on Wayland a popup that
-- changes size is a new `xdg_popup`, and the compositor faded each one in
-- and the last out, so the menu flickered see-through, two of it
-- overlapping, on every switch.)
test("moving between tabs keeps the panel's size, its list changing in place", function()
	local app = mount("picker_fixture.luau")
	app:resize(600, 700)
	settle(app)
	app:set_state("ds/model_picker.luau::model_picker_compact", false)
	-- (On a chat: the tabs only browse.)
	app:set_state("test_picker_committed", true)
	app:click('role:button "Model"')
	settle(app)
	-- The panel: its node and its box, as the tree has it.
	local function panel(): (string?, string?)
		for line in app:tree():gmatch("[^\n]+") do
			local id, size = line:match("@(%d+) dialog %S+ (%d+x%d+)")
			if id then
				return id, size
			end
		end
		return nil, nil
	end
	local id, size = panel()
	expect(id).never.to_be_nil()
	-- The reasoning's slider, once.
	local reasoning = app:count('"Reasoning"')
	-- Claude's two models, Codex's two, the starred (none), Devin's two:
	-- frame by frame through each switch, the same panel at the same size.
	for _, tab in { "Codex", "Starred", "Devin", "Claude" } do
		app:click(`role:tab "{tab}"`)
		for _ = 1, 20 do
			app:frames(1)
			local now_id, now_size = panel()
			expect(`{tab}: @{now_id} {now_size}`).to_be(`{tab}: @{id} {size}`)
			expect(app:count('"Reasoning"')).to_be(reasoning)
		end
	end
	-- Its list is the tab's: Claude's models again.
	expect(app:has('role:option ~"Sonnet"')).to_be(true)
	expect(app:has('role:option ~"GPT"')).to_be(false)
	-- A search that finds nothing keeps it too.
	app:type('role:textbox "Search"', "zz")
	for _ = 1, 20 do
		app:frames(1)
		local now_id, now_size = panel()
		expect(`search: @{now_id} {now_size}`).to_be(`search: @{id} {size}`)
	end
	expect(app:has('role:option ~"Sonnet"')).to_be(false)
	expect(app:error()).to_be(nil)
	app:set_state("ds/model_picker.luau::model_picker_compact", true)
end)

test("a model the catalog no longer lists says so, and picking it does nothing", function()
	local app = mount("picker_fixture.luau")
	app:resize(900, 700)
	settle(app)
	app:set_state("test_picker_model", "gone-from-the-catalog")
	settle(app)
	app:click('role:button "Model"')
	settle(app)
	expect(app:has('#model-picker-title >> ~"gone-from-the-catalog"')).to_be(true)
	-- Rows showing, its own among them in the full layout's tab.
	app:set_state("test_picker_model", "claude/gone")
	app:set_state("ds/model_picker.luau::model_picker_compact", false)
	settle(app)
	expect(app:has('role:option "gone"')).to_be(true)
	app:click('role:option "gone"')
	settle(app)
	expect(app:state("test_picker_model")).to_be("claude/gone")
	expect(app:has('role:option "Sonnet"')).to_be(true)
	expect(app:error()).to_be_nil()
	app:set_state("ds/model_picker.luau::model_picker_compact", true)
end)

-- The full layout's rows and tabs say how the catalog stands: a model new to
-- it wears NEW, an agent still listing holds its rows' shapes (no "No models"),
-- one that failed says why with Retry, which asks again.
test("a new model wears NEW; a listing agent holds its rows, a failed one says why and retries", function()
	local app = mount("picker_fixture.luau")
	app:set_state("test_picker_more", true)
	app:set_state("test_picker_committed", true)
	app:set_state("ds/model_picker.luau::model_picker_compact", false)
	app:resize(700, 760)
	settle(app)
	app:click('role:button "Model"')
	settle(app)
	app:click('role:tab "Codex"')
	settle(app)
	expect(line_of(app, 'option "Sol NEW"')).never.to_be_nil()
	expect(line_of(app, 'option "GPT')).never.to_be_nil()
	app:click('role:tab "Pi"')
	settle(app)
	expect(line_of(app, '"No models"')).to_be_nil()
	expect(line_of(app, " option ")).to_be_nil()
	app:click('role:tab "Fx"')
	settle(app)
	expect(line_of(app, '"Not signed in to Fx"')).never.to_be_nil()
	app:click('role:button "Retry"')
	settle(app)
	expect(app:state("test_picker_retried")).to_be(1)
	expect(app:error()).to_be(nil)
end)

-- A new chat with stars opens the full layout on them, its rows hinted ⌘1…,
-- until a tab is picked, which it opens on again; the compact list has no hints, and two rows of one name tell themselves
-- apart by the description.
test("a new chat's full list opens on its stars with ⌘ hints; same-named rows say which", function()
	local app = mount("picker_fixture.luau")
	app:set_state("test_picker_more", true)
	app:set_state("test_picker_favorites", { gpt = true })
	app:set_state("ds/model_picker.luau::model_picker_compact", false)
	app:resize(700, 760)
	settle(app)
	app:click('role:button "Model"')
	settle(app)
	expect(line_of(app, 'option "GPT ⌘1"')).never.to_be_nil()
	expect(line_of(app, 'option "Sonnet')).to_be_nil()
	app:click('role:tab "Codex"')
	settle(app)
	app:key("Escape")
	settle(app)
	app:click('role:button "Model"')
	settle(app)
	expect(line_of(app, 'option "Sol NEW"')).never.to_be_nil()
	app:key("Escape")
	settle(app)
	app:set_state("ds/model_picker.luau::model_picker_compact", true)
	app:click('role:button "Model"')
	settle(app)
	app:click("#model-picker-title")
	settle(app)
	expect(line_of(app, "⌘")).to_be_nil()
	expect(line_of(app, 'option "Auto Codex picks"')).never.to_be_nil()
	expect(line_of(app, 'option "Auto"')).never.to_be_nil()
	expect(line_of(app, 'option "Mini the anthropic route"')).to_be_nil()
end)

test("while its catalog is on its way, the model holds its shape rather than saying Choose", function()
	local app = mount("chat_fixture.luau")
	app:resize(1100, 760)
	app:command("go_s1")
	app:frames(20)
	local catalog = app:state("chat.catalog")
	app:set_state("chat.catalog", { state = "looking", complete = false, harnesses = {}, models = {} })
	app:frames(5)
	expect(app:has('#composer-toolbar >> "Choose…"')).to_be(false)
	-- All in, and truly none: it may say so.
	app:set_state("chat.catalog", { state = "ready", complete = true, harnesses = {}, models = {} })
	app:frames(5)
	expect(app:has('#composer-toolbar >> "Choose…"')).to_be(true)
	app:set_state("chat.catalog", catalog)
end)

-- A chat's model its agent's catalog doesn't list (renamed, retired, or a
-- Cloud agent's list not in yet) keeps its agent: the chip names it beside
-- that agent's mark, and the panel opens on that agent's tab, not the first
-- agent's (it opened on Claude's, the chip a bare name).
test("a model its catalog doesn't list keeps its agent's mark and tab", function()
	local app = mount("picker_fixture.luau")
	app:set_state("test_picker_model", "devin/swe-9")
	app:set_state("ds/model_picker.luau::model_picker_compact", false)
	app:resize(700, 760)
	settle(app)
	expect(app:tree():find("Unlisted", 1, true) ~= nil).to_be(true)
	app:click('role:button "Model"')
	settle(app)
	expect(app:tree():find("Search Devin models", 1, true) ~= nil).to_be(true)
	expect(app:tree():find("Search Claude models", 1, true) ~= nil).to_be(false)
	expect(app:error()).to_be_nil()
end)
