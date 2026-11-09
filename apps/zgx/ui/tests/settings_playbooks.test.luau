-- Settings → Playbooks on ui/playbooks_fixture.luau: each playbook with where it comes from, Start for one with a
-- file (a new chat, its link in the draft), Edit, Delete asked once more, New playbook, Reveal folder.

local PAGE = "#settings-page"

local function open(): any
	local app = mount("playbooks_fixture.luau")
	app:resize(1200, 1400)
	app:frames(30)
	app:command("shell.settings")
	app:frames(10)
	app:click('#settings-nav >> role:tab "Playbooks"')
	app:frames(30)
	return app
end

test("Playbooks: triage, this Mac's and a project's, each saying where it comes from", function()
	local app = open()
	local tree = app:tree(PAGE)
	expect(tree).to_contain("Triage an issue")
	expect(tree).to_contain("Built in")
	expect(tree).to_contain("All projects · release.md")
	expect(tree).to_contain("deploy.md")
	expect(app:text("#playbooks-note")).to_contain("/home/ada/.zeron/playbooks")
	expect(app:state("playbooks_calls")).to_be("list")
end)

test("Playbooks: Start opens a new chat with the playbook's link first in the draft", function()
	local app = open()
	app:click(`#playbook-start-{string.gsub("/home/ada/.zeron/playbooks/release.md", "[^%w]", "-")}`)
	app:frames(30)
	expect(app:state("shell_route_path")).to_be("/")
	expect(app:tree("role:textbox")).to_contain("Read [release.md]")
end)

test("Playbooks: triage's Start prepares its folder and opens a new chat with its link", function()
	local app = open()
	expect(app:has("#playbook-edit-triage")).to_be(false)
	app:click("#playbook-start-triage")
	app:frames(30)
	expect(app:state("playbooks_calls")).to_be("list,triage")
	expect(app:state("shell_route_path")).to_be("/")
	expect(app:tree("role:textbox")).to_contain("Read [playbook.md]")
end)

test("Playbooks: Edit opens the file, Delete asks once more then moves it to the Trash, New and Reveal ask the host", function()
	local app = open()
	local key = string.gsub("/home/ada/.zeron/playbooks/release.md", "[^%w]", "-")
	app:click(`#playbook-edit-{key}`)
	app:frames(5)
	app:click(`#playbook-delete-{key}`)
	app:frames(5)
	expect(app:tree(`#playbook-delete-{key}`)).to_contain("Confirm delete")
	expect(app:state("playbooks_calls")).never.to_contain("trash")
	app:click(`#playbook-delete-{key}`)
	app:frames(10)
	expect(app:state("playbooks_calls")).to_contain("trash:/home/ada/.zeron/playbooks/release.md")
	expect(app:tree(PAGE)).never.to_contain("Cut a release")
	app:click("#new-playbook")
	app:frames(10)
	expect(app:tree(PAGE)).to_contain("New playbook")
	app:click("#playbooks-reveal-folder")
	app:frames(5)
	expect(app:state("playbooks_calls")).to_be(
		"list,open:/home/ada/.zeron/playbooks/release.md,trash:/home/ada/.zeron/playbooks/release.md,list,create,list,reveal:/home/ada/.zeron/playbooks"
	)
end)
