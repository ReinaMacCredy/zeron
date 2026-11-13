
local SIDE = "#shell-sidebar"

local function eventually(app: any, ok: () -> boolean): boolean
	for _ = 1, 300 do
		if ok() then
			return true
		end
		app:wait('"never shown, only waited past"', { timeout_ms = 50 })
	end
	return ok()
end

local function open(): any
	local app = mount("shell_fixture.luau")
	app:resize(1200, 800)
	app:frames(30)
	expect(app:error()).to_be_nil()
	return app
end

local function show_chat(app: any, title: string)
	app:click(`{SIDE} >> role:button "{title}"`)
	expect(app:wait("#sidecar", { timeout_ms = 15000 })).to_be(true)
	app:frames(10)
end

local function is_panel(app: any): boolean
	return eventually(app, function(): boolean
		return app:has("#sidecar-collapse") and not app:has("#sidecar-expand")
	end)
end
local function is_rail(app: any): boolean
	return eventually(app, function(): boolean
		return app:has("#sidecar-expand") and not app:has("#sidecar-collapse")
	end)
end

local function split(app: any)
	app:set_state("inset_split_trees", { { kind = "row", children = { { kind = "leaf", id = "a1" }, { kind = "leaf", id = "a2" } } } })
end

local function focused(app: any): string
	for line in app:tree():gmatch("[^\n]+") do
		if line:find(" focused", 1, true) then
			return line
		end
	end
	return ""
end

test("it stands beside a chat, and not with no chat or in a split", function()
	local app = open()
	expect(app:has("#sidecar")).to_be(false)
	show_chat(app, "Fix the build")
	expect(is_panel(app)).to_be(true)
	split(app)
	expect(eventually(app, function(): boolean
		return app:has("#split-pane-a2") and not app:has("#sidecar")
	end)).to_be(true)
end)

test("folded by hand it stays a rail, and the keyboard stays on what brings it back, and back again", function()
	local app = open()
	show_chat(app, "Fix the build")
	app:click("#sidecar-collapse")
	expect(is_rail(app)).to_be(true)
	expect(focused(app):find("sidecar-expand", 1, true) ~= nil).to_be(true)
	expect(app:state("screens/sidecar.luau::sidecar_collapsed")).to_be(true)
	show_chat(app, "Write the docs")
	expect(is_rail(app)).to_be(true)
	app:click("#sidecar-expand")
	expect(is_panel(app)).to_be(true)
	expect(focused(app):find("sidecar-collapse", 1, true) ~= nil).to_be(true)
	expect(app:state("screens/sidecar.luau::sidecar_collapsed")).to_be(false)
end)

test("the panel open or a narrow column makes it a rail; Expand shuts the panel", function()
	local app = open()
	show_chat(app, "Fix the build")
	app:click('#titlebar >> role:button "Panel"')
	expect(is_rail(app)).to_be(true)
	app:click("#sidecar-expand")
	expect(is_panel(app)).to_be(true)
	expect(((app:state("shell_panel_open") :: any) or {}).a1 == true).to_be(false)
	app:resize(900, 800)
	expect(is_rail(app)).to_be(true)
	app:resize(1400, 800)
	expect(is_panel(app)).to_be(true)
end)

test("a rail button opens its section's card; again closes it; another swaps it", function()
	local app = open()
	show_chat(app, "Fix the build")
	app:click("#sidecar-collapse")
	expect(is_rail(app)).to_be(true)
	local function shown(id: string): boolean
		return app:has(`#sidecar-popover-{id}`) and not app:tree(`#sidecar-popover-{id}`):find(" hidden", 1, true)
	end
	app:click("#sidecar-rail-workspace")
	expect(eventually(app, function(): boolean
		return shown("workspace")
	end)).to_be(true)
	app:click("#sidecar-rail-workspace")
	expect(eventually(app, function(): boolean
		return not shown("workspace")
	end)).to_be(true)
	app:click("#sidecar-rail-workspace")
	expect(eventually(app, function(): boolean
		return shown("workspace")
	end)).to_be(true)
	app:click("#sidecar-rail-beside")
	expect(eventually(app, function(): boolean
		return shown("beside") and not shown("workspace")
	end)).to_be(true)
end)

test("running agents lead and the rest fold under Previous", function()
	local app = open()
	show_chat(app, "Fix the build")
	local function spawned(ref: string, status: string): any
		return { id = ref, name = "subagent", subagent = { ref = ref, title = ref, status = status } }
	end
	app:set_state("fixture.chats", {
		a1 = { session = { id = "a1" }, messages = { { id = "m", role = "assistant", started = 100, tools = { spawned("one", "completed"), spawned("two", "failed"), spawned("three", "running") } } } },
	})
	expect(eventually(app, function(): boolean
		return app:has("#sidecar-panel-agent-three")
	end)).to_be(true)
	expect(app:has("#sidecar-panel-empty")).to_be(false)
	expect(app:has("#sidecar-panel-tabs")).to_be(false)
	expect(app:has("#sidecar-panel-agent-one")).to_be(false)
	app:click("#sidecar-panel-fold")
	expect(eventually(app, function(): boolean
		return app:has("#sidecar-panel-agent-one") and app:has("#sidecar-panel-agent-two")
	end)).to_be(true)
end)

test("beside the thread by kind: the asking thread's tab opens, old threads fold under Previous, a shell reads its description", function()
	local app = open()
	show_chat(app, "Fix the build")
	local now = os.time()
	local all = table.clone(app:state("sessions") :: any)
	all.children = {
		a1 = {
			{ id = "old", kind = "side_chat", title = "Old side chat", updated = now - 3 * 86400, created = now - 3 * 86400 },
			{ id = "busy", kind = "agent", title = "Busy review", status = "working", updated = now - 60, created = now - 600 },
			{ id = "asker", kind = "agent", title = "Asking review", status = "input", updated = now - 120, created = now - 900 },
		},
	}
	app:set_state("sessions", all)
	local function spawned(ref: string, status: string): any
		return { id = ref, name = "subagent", subagent = { ref = ref, title = ref, status = status } }
	end
	app:set_state("fixture.chats", {
		a1 = {
			session = { id = "a1", tasks = { { id = "t", task_kind = "shell", name = "Launch Dev 86", detail = "EXTRA_ENV=1 /private/tmp/dev86/launch.sh", since = now - 60 } } },
			messages = { { id = "m", role = "assistant", started = 100, tools = { spawned("lane", "running"), spawned("done", "completed") } } },
		},
	})
	expect(eventually(app, function(): boolean
		return app:has("#sidecar-panel-child-asker") and app:has("#sidecar-panel-tab-agent")
	end)).to_be(true)
	expect(app:has("#sidecar-panel-agent-lane")).to_be(false)
	expect(app:has("#sidecar-panel-tab-thread >> .sidecar-tab-asking")).to_be(true)
	expect(app:has("#sidecar-panel-child-busy")).to_be(true)
	expect(app:has("#sidecar-panel-child-old")).to_be(false)
	local order = app:tree("#sidecar-panel-section-beside")
	expect(order:find("Asking review", 1, true) < order:find("Busy review", 1, true)).to_be(true)
	app:click("#sidecar-panel-fold")
	expect(eventually(app, function(): boolean
		return app:has("#sidecar-panel-child-old")
	end)).to_be(true)
	app:click("#sidecar-panel-tab-shell")
	expect(eventually(app, function(): boolean
		return app:has("#sidecar-panel-shell-t")
	end)).to_be(true)
	expect(app:tree("#sidecar-panel-shell-t"):find("Launch Dev 86", 1, true) ~= nil).to_be(true)
	expect(app:has("#sidecar-panel-child-old")).to_be(false)
	app:frames(60)
	local function left(id: string): number
		local line = app:tree(`#{id}`):match("[^\n]+")
		return tonumber(line:match(" (%-?[%d.]+),%-?[%d.]+ [%d.]+x")) :: number
	end
	expect(math.abs(left("sidecar-panel-tabs-thumb") - left("sidecar-panel-tab-shell")) < 1).to_be(true)
	app:click("#sidecar-panel-tab-agent")
	-- The thumb springs across from Shell, not jumping to Agent.
	app:frames(3)
	local mid = left("sidecar-panel-tabs-thumb")
	expect(mid > left("sidecar-panel-tab-agent") + 1 and mid < left("sidecar-panel-tab-shell") - 1).to_be(true)
	expect(eventually(app, function(): boolean
		return app:has("#sidecar-panel-agent-lane")
	end)).to_be(true)
	expect(app:has("#sidecar-panel-agent-done")).to_be(false)
	expect(app:error()).to_be_nil()
end)

test("elapsed and ago read the time as they're worked out, with nothing having ticked", function()
	local app = open()
	show_chat(app, "Fix the build")
	local now = os.time()
	local all = table.clone(app:state("sessions") :: any)
	all.children = {
		a1 = {
			{ id = "old", kind = "side_chat", title = "Old", updated = now - 1800 },
			{ id = "lapsing", kind = "side_chat", title = "Lapsing", updated = now - 60, expires = now + 2 },
			{ id = "lapsed", kind = "side_chat", title = "Lapsed", updated = now - 60, expires = now - 1 },
		},
	}
	app:set_state("sessions", all)
	app:set_state("fixture.chats", {
		a1 = { session = { id = "a1", tasks = { { id = "t", task_kind = "shell", name = "server", since = now - 7200 } } }, messages = {} },
	})
	local function says(id: string, words: string): boolean
		return eventually(app, function(): boolean
			return app:has(id) and app:tree(id):find(words, 1, true) ~= nil
		end)
	end
	expect(says("#sidecar-panel-shell-t", "2h")).to_be(true)
	app:click("#sidecar-panel-tab-thread")
	app:click("#sidecar-panel-fold")
	expect(says("#sidecar-panel-child-old", "30m ago")).to_be(true)
	expect(says("#sidecar-panel-child-lapsed", "Expired")).to_be(true)
	expect(says("#sidecar-panel-child-lapsing", "1m ago")).to_be(true)
	app:wait('"never shown, only waited past"', { timeout_ms = 2500 })
	app:hover(SIDE)
	app:hover("#sidecar-panel-machine")
	expect(says("#sidecar-panel-child-lapsing", "Expired")).to_be(true)
end)

test("the tick comes at the soonest change of what's running", function()
	local tick = require("../screens/sidecar_tick")
	expect(tick.soonest({ 1000 - 230, 1000 - 200 }, 1000)).to_be(10)
	expect(tick.soonest({ 1000 - 200, 1000 - 230 }, 1000)).to_be(10)
	expect(tick.soonest({ 1000 - 200, 1000 - 30 }, 1000)).to_be(1)
	expect(tick.soonest({}, 1000)).to_be_nil()
end)

test("a comparison answered after the head moved on is left out", function()
	local app = open()
	show_chat(app, "Fix the build")
	local function head(sha: string)
		local all = table.clone(app:state("sessions") :: any)
		local chats = table.clone(all.chats)
		for i, c in chats do
			if c.id == "a1" then
				local changed = table.clone(c)
				changed.cwd, changed.head, changed.live_branch = "/work/atlas", sha, "fix/build"
				chats[i] = changed
			end
		end
		all.chats = chats
		app:set_state("sessions", all)
		app:frames(10)
	end
	head("aaaaaaaaaa")
	head("bbbbbbbbbb")
	app:command("fixture.comparison_last")
	expect(eventually(app, function(): boolean
		return app:has("#sidecar-panel-commit") and app:tree("#sidecar-panel-commit"):find("bbbbbbb · 2 ahead of main", 1, true) ~= nil
	end)).to_be(true)
	app:command("fixture.comparison_first")
	app:frames(20)
	expect(app:tree("#sidecar-panel-commit"):find("2 ahead of main", 1, true) ~= nil).to_be(true)
end)

test("Expand that leaves a rail in a narrow window leaves no claim on the keyboard for later", function()
	local app = open()
	show_chat(app, "Fix the build")
	app:click("#sidecar-collapse")
	expect(is_rail(app)).to_be(true)
	app:resize(900, 800)
	app:frames(20)
	app:click("#sidecar-expand")
	app:frames(20)
	expect(is_rail(app)).to_be(true)
	app:click('role:textbox "Message"')
	app:frames(10)
	expect(focused(app):find("Message", 1, true) ~= nil).to_be(true)
	app:resize(1400, 800)
	expect(is_panel(app)).to_be(true)
	app:frames(20)
	expect(focused(app):find("Message", 1, true) ~= nil).to_be(true)
end)

test("Expand as the rail's still fading lands on Fold", function()
	local app = open()
	show_chat(app, "Fix the build")
	app:click("#sidecar-collapse")
	app:click("#sidecar-expand")
	expect(is_panel(app)).to_be(true)
	app:frames(20)
	expect(focused(app):find("sidecar-collapse", 1, true) ~= nil).to_be(true)
end)

test("the panel's home leaves what the column shows, and has it again in a split", function()
	local app = open()
	show_chat(app, "Fix the build")
	app:click('#titlebar >> role:button "Panel"')
	expect(app:wait("#panel-empty", { timeout_ms = 15000 })).to_be(true)
	app:frames(20)
	expect(app:has("#panel-home-device")).to_be(false)
	expect(app:tree("#panel-empty"):find("Side chats", 1, true) == nil).to_be(true)
	split(app)
	expect(eventually(app, function(): boolean
		return app:has("#panel-home-device")
	end)).to_be(true)
end)
