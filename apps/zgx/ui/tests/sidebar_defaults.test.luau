-- What the sidebar opens with before anyone changes it: Activity off (it is
-- offered nowhere), by project then device, last updated first, no empty
-- projects, full rows, no archived chats, each row's agent and project icon.

test("the sidebar opens without Activity, a list by project then device", function()
	local app = mount("sidebar_defaults_fixture.luau")
	app:frames(2)
	local seen = app:state("sidebar_defaults")
	expect(seen.activity).to_be(false)
	expect(seen.organize).to_be("project,device")
	expect(seen.sort).to_be("updated")
	expect(seen.empty_projects).to_be(false)
	expect(seen.compact).to_be(false)
	expect(seen.archived).to_be(false)
	expect(seen.harness).to_be(true)
	expect(seen.project_icon).to_be(true)
end)
