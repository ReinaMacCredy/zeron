-- The files pane's tree widens and narrows at its edge as the sidebar does: its width is
-- kept (`files_tree_width`, 0 for the theme's), held to 200 to 480, and a double-click on
-- the edge puts the theme's back. (The pointer's drag is the edge's own: a test's drag
-- presses a 6-point edge on its far side, outside it.) On the pane in ui/files_fixture.luau.

local function box(app: any, target: string): { x: number, w: number }
	local line = app:tree(target):match("[^\n]+") :: string
	local x, w = line:match(" (%-?[%d%.]+),%-?[%d%.]+ ([%d%.]+)x[%d%.]+")
	return { x = tonumber(x) :: number, w = tonumber(w) :: number }
end

local function width(app: any, w: number): number
	app:set_state("files_tree_width", w)
	app:frames(30)
	return box(app, "#files-tree-beside").w
end

test("the file tree's width is kept within its limits, its edge on it, and a double-click puts it back", function()
	local app = mount("files_fixture.luau")
	app:resize(900, 600)
	app:frames(30)
	expect(app:wait('role:treeitem "src"', { timeout_ms = 15000 })).to_be(true)
	local theme = box(app, "#files-tree-beside").w
	expect(width(app, 360)).to_be(360)
	local edge = box(app, "#files-tree-edge")
	expect(edge.x + edge.w / 2).to_be(360)
	expect(width(app, 9999)).to_be(480)
	expect(width(app, 40)).to_be(200)
	app:double_click("#files-tree-edge")
	app:frames(30)
	expect(app:state("files_tree_width")).to_be(0)
	expect(box(app, "#files-tree-beside").w).to_be(theme)
	expect(app:error()).to_be_nil()
end)
