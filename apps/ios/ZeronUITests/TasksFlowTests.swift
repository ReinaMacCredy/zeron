import XCTest

final class TasksFlowTests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private func launch(_ route: String) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-demo", "-fast", "-route", route]
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

    private func scrollTo(_ element: XCUIElement, in list: XCUIElement) {
        for _ in 0..<8 where !(element.exists && element.isHittable) {
            list.swipeUp()
        }
    }

    func testApproveAndRun() {
        let app = launch("tasks:mission:task-ios-install")
        let approve = cell(app, "mc-approve")
        XCTAssertTrue(approve.waitForExistence(timeout: 15))
        XCTAssertTrue(app.descendants(matching: .any)["plan-markdown"].waitForExistence(timeout: 5))
        snapshot(app, "mission-plan-ready")
        let cost = cell(app, "mc-est-cost")
        scrollTo(cost, in: app.collectionViews["mission-control"])
        XCTAssertTrue(cost.waitForExistence(timeout: 10))
        XCTAssertTrue(cost.label.contains("Relative cost 3.2x"))
        snapshot(app, "mission-plan-estimate")
        scrollTo(approve, in: app.collectionViews["mission-control"])
        approve.tap()
        XCTAssertTrue(approve.waitForNonExistence(timeout: 10))
        snapshot(app, "mission-approved")
    }

    func testStepOpensItsThread() {
        let app = launch("tasks:mission:task-settle-shelf")
        let step = cell(app, "mc-step:task-settle-shelf-s1")
        XCTAssertTrue(step.waitForExistence(timeout: 15))
        snapshot(app, "mission-running")
        step.tap()
        let open = app.buttons["Open thread"]
        XCTAssertTrue(open.waitForExistence(timeout: 5))
        snapshot(app, "step-actions")
        open.tap()
        XCTAssertTrue(app.scrollViews["transcript"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.buttons["mission-strip"].waitForExistence(timeout: 5))
        snapshot(app, "step-thread-with-strip")
        app.buttons["mission-strip"].tap()
        XCTAssertTrue(app.collectionViews["mission-control"].waitForExistence(timeout: 5))
    }

    func testAnsweringADecision() {
        let app = launch("tasks:supervisor")
        let entry = app.cells.matching(NSPredicate(format: "identifier BEGINSWITH 'sv-needs:' AND label CONTAINS 'TestFlight'")).firstMatch
        XCTAssertTrue(entry.waitForExistence(timeout: 15))
        snapshot(app, "supervisor-needs-you")
        entry.tap()
        let answers = app.collectionViews["answers"]
        XCTAssertTrue(answers.waitForExistence(timeout: 5))
        let pick = cell(app, "answers-pick:item-testflight:1")
        scrollTo(pick, in: answers)
        XCTAssertTrue(pick.waitForExistence(timeout: 5))
        pick.tap()
        snapshot(app, "answers-picked")
        app.buttons["answers-send"].tap()
        XCTAssertTrue(pick.waitForNonExistence(timeout: 10))
        snapshot(app, "answers-sent")
    }

    func testAuthoritySheet() {
        let app = launch("tasks:supervisor")
        let host = cell(app, "sv-host:dev-mac")
        XCTAssertTrue(host.waitForExistence(timeout: 15))
        host.tap()
        let authority = app.buttons["Authority…"]
        XCTAssertTrue(authority.waitForExistence(timeout: 5))
        snapshot(app, "supervisor-host-actions")
        authority.tap()
        let decide = app.switches["authority-decide"]
        XCTAssertTrue(decide.waitForExistence(timeout: 10))
        snapshot(app, "authority-sheet")
        decide.tap()
        snapshot(app, "authority-decide-toggled")
    }

    func testHeldApprovalCountsDown() {
        let app = launch("tasks:mission:task-widget")
        let approval = cell(app, "mc-approval")
        XCTAssertTrue(approval.waitForExistence(timeout: 15))
        let first = approval.label
        snapshot(app, "countdown-1")
        let changed = NSPredicate { _, _ in approval.label != first }
        expectation(for: changed, evaluatedWith: approval)
        waitForExpectations(timeout: 5)
        snapshot(app, "countdown-2")
    }

    func testSupervisorStopsAndResumes() {
        let app = launch("tasks:supervisor")
        let host = cell(app, "sv-host:dev-mac")
        XCTAssertTrue(host.waitForExistence(timeout: 15))
        host.tap()
        let stop = app.buttons["Stop"].firstMatch
        XCTAssertTrue(stop.waitForExistence(timeout: 5))
        stop.tap()
        XCTAssertTrue(app.staticTexts["Stop the Supervisor on MacBook Pro?"].waitForExistence(timeout: 5))
        let confirm = app.buttons["Stop"].firstMatch
        XCTAssertTrue(confirm.waitForExistence(timeout: 5))
        snapshot(app, "supervisor-stop-confirm")
        confirm.tap()
        XCTAssertTrue(app.descendants(matching: .any).matching(NSPredicate(format: "label CONTAINS 'Stopped by you'")).firstMatch.waitForExistence(timeout: 10))
        snapshot(app, "supervisor-stopped")
        host.tap()
        app.buttons["Resume"].tap()
        XCTAssertTrue(app.descendants(matching: .any).matching(NSPredicate(format: "label CONTAINS 'Stopped by you'")).firstMatch.waitForNonExistence(timeout: 10))
        host.tap()
        let conversation = app.buttons["Open conversation"]
        XCTAssertTrue(conversation.waitForExistence(timeout: 5))
        conversation.tap()
        XCTAssertTrue(app.scrollViews["transcript"].waitForExistence(timeout: 10))
        snapshot(app, "supervisor-conversation")
    }

    func testInlinePlanEdit() {
        let app = launch("tasks:mission:task-ios-install")
        let edit = app.buttons["mc-edit"]
        XCTAssertTrue(edit.waitForExistence(timeout: 15))
        edit.tap()
        let list = app.collectionViews["mission-control"]
        let step = cell(app, "mc-step:task-ios-install-s2")
        scrollTo(step, in: list)
        step.tap()
        let title = app.textFields["step-title"]
        XCTAssertTrue(title.waitForExistence(timeout: 5))
        title.tap()
        let current = (title.value as? String) ?? ""
        title.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: current.count) + "Renamed on the phone")
        app.alerts.buttons["Save"].tap()
        let add = cell(app, "mc-add-step")
        scrollTo(add, in: list)
        add.tap()
        let newTitle = app.textFields["step-title"]
        XCTAssertTrue(newTitle.waitForExistence(timeout: 5))
        newTitle.typeText("A step from the phone")
        app.alerts.buttons["Save"].tap()
        let send = cell(app, "mc-send-draft")
        scrollTo(send, in: list)
        XCTAssertTrue(send.waitForExistence(timeout: 5))
        snapshot(app, "plan-draft")
        send.tap()
        XCTAssertTrue(send.waitForNonExistence(timeout: 10))
        XCTAssertTrue(app.descendants(matching: .any).matching(NSPredicate(format: "label CONTAINS 'A step from the phone'")).firstMatch.waitForExistence(timeout: 10))
        snapshot(app, "plan-draft-sent")
    }

    func testLeadBrief() {
        let app = launch("tasks:mission:task-ios-install")
        let menu = app.buttons["mc-menu"]
        XCTAssertTrue(menu.waitForExistence(timeout: 15))
        menu.tap()
        snapshot(app, "mission-menu")
        app.buttons["Lead brief"].tap()
        XCTAssertTrue(app.staticTexts["brief-title"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.descendants(matching: .any).matching(NSPredicate(format: "label CONTAINS 'From you'")).firstMatch.waitForExistence(timeout: 15))
        snapshot(app, "lead-brief")
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

    func testNewMissionFromTheTasksTab() {
        let app = launch("tasks")
        let add = app.buttons["tasks-add"]
        XCTAssertTrue(add.waitForExistence(timeout: 15))
        add.tap()
        let title = app.textFields["new-task-title"]
        XCTAssertTrue(title.waitForExistence(timeout: 5))
        title.typeText("Ship the phone widget")
        app.switches["new-task-mission"].tap()
        snapshot(app, "new-mission")
        app.buttons["new-task-create"].tap()
        XCTAssertTrue(app.collectionViews["mission-control"].waitForExistence(timeout: 10))
        snapshot(app, "new-mission-control")
    }
}
