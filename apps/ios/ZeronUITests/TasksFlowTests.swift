import XCTest

final class TasksFlowTests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private func launch(_ route: String, _ extra: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-demo", "-fast", "-route", route] + extra
        app.launch()
        return app
    }

    private func snapshot(_ app: XCUIApplication, _ name: String) {
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }

    private func cell(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.cells[id].firstMatch
    }

    func testHandATaskToAThread() {
        let app = launch("tasks:task:task-relay-docs")
        let sheet = app.collectionViews["task-sheet"]
        XCTAssertTrue(sheet.waitForExistence(timeout: 15))
        let assign = cell(app, "task-sheet-assign")
        scrollTo(assign, in: sheet)
        XCTAssertTrue(assign.waitForExistence(timeout: 5))
        snapshot(app, "task-sheet")
        assign.tap()
        let picker = app.collectionViews["thread-picker"]
        XCTAssertTrue(picker.waitForExistence(timeout: 5))
        snapshot(app, "thread-picker")
        picker.cells.element(boundBy: 0).tap()
        let link = app.buttons["Link only"]
        XCTAssertTrue(link.waitForExistence(timeout: 5))
        link.tap()
        let unassign = cell(app, "task-sheet-unassign")
        scrollTo(unassign, in: sheet)
        XCTAssertTrue(unassign.waitForExistence(timeout: 10))
        snapshot(app, "task-assigned")
    }

    func testNewTaskFromTheTasksTab() {
        let app = launch("tasks")
        let add = app.buttons["tasks-add"]
        XCTAssertTrue(add.waitForExistence(timeout: 15))
        add.tap()
        let title = app.textFields["new-task-title"]
        XCTAssertTrue(title.waitForExistence(timeout: 5))
        title.typeText("Ship the phone widget")
        snapshot(app, "new-task")
        app.buttons["new-task-create"].tap()
        XCTAssertTrue(app.collectionViews["task-sheet"].waitForExistence(timeout: 10))
        snapshot(app, "new-task-sheet")
    }

    func testAgentTreesOpenTheirOutline() {
        let app = launch("tasks", ["-tasksView", "list"])
        let tree = cell(app, "tree-chat-agents")
        XCTAssertTrue(app.collectionViews["tasks-list"].waitForExistence(timeout: 15))
        scrollTo(tree, in: app.collectionViews["tasks-list"])
        snapshot(app, "trees-list")
        tree.tap()
        XCTAssertTrue(app.collectionViews["tree-outline"].waitForExistence(timeout: 10))
        XCTAssertTrue(cell(app, "tree-node-chat-devin").waitForExistence(timeout: 5))
        snapshot(app, "tree-outline")
    }

    func testKanbanChoiceSurvivesARelaunch() {
        var app = launch("tasks")
        let views = app.segmentedControls["tasks-view"]
        XCTAssertTrue(views.waitForExistence(timeout: 15))
        views.buttons["Kanban"].tap()
        let board = app.descendants(matching: .any)["tasks-kanban"]
        XCTAssertTrue(board.waitForExistence(timeout: 5))
        snapshot(app, "tasks-kanban")
        app.terminate()
        app = launch("tasks")
        XCTAssertTrue(app.descendants(matching: .any)["tasks-kanban"].waitForExistence(timeout: 15))
        XCTAssertTrue(app.segmentedControls["tasks-view"].buttons["Kanban"].isSelected)
        app.segmentedControls["tasks-view"].buttons["List"].tap()
        XCTAssertTrue(app.collectionViews["tasks-list"].isHittable)
    }

}
