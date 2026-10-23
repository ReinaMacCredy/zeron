import XCTest

final class InboxFlowTests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private func launch(pinnedView: Bool = true) -> (XCUIApplication, XCUIElement) {
        let app = XCUIApplication()
        app.launchArguments = ["-demo", "-fast"]
        // The argument domain pins the persisted list menu so one test's choice can't leak into the next.
        if pinnedView { app.launchArguments += ["-sidebarOrganization", "inbox", "-sidebarSort", "lastUpdated", "-activeFirst", "NO"] }
        app.launch()
        let list = app.collectionViews.firstMatch
        XCTAssertTrue(app.cells["section-needs-you"].waitForExistence(timeout: 20))
        return (app, list)
    }

    private func snapshot(_ app: XCUIApplication, _ name: String) {
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }

    private func scrollTo(_ element: XCUIElement, in list: XCUIElement) {
        for _ in 0..<20 where !(element.exists && element.isHittable) {
            list.swipeUp()
        }
        XCTAssertTrue(element.isHittable)
    }

    private func drag(_ list: XCUIElement, from: CGFloat, to: CGFloat) {
        list.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: from))
            .press(forDuration: 0.05, thenDragTo: list.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: to)))
    }

    private func rows(under header: XCUIElement, in app: XCUIApplication) -> [String] {
        let below = header.frame.maxY - 1
        return app.cells.matching(NSPredicate(format: "identifier BEGINSWITH 'session-'")).allElementsBoundByIndex
            .filter { $0.frame.minY >= below }
            .sorted { $0.frame.minY < $1.frame.minY }
            .prefix(8).map(\.identifier)
    }

    private func setFold(_ header: XCUIElement, collapsed: Bool) {
        if (header.value as? String == "Collapsed") != collapsed { header.tap() }
    }

    private func option(_ app: XCUIApplication, _ title: String) {
        app.buttons["sessions-options"].tap()
        let item = app.buttons[title]
        XCTAssertTrue(item.waitForExistence(timeout: 5))
        item.tap()
    }

    func testASessionSettlesAndUnsettlesBySwipe() {
        let (app, list) = launch()
        let deploy = app.cells["session-chat-deploy"]
        scrollTo(deploy, in: list)
        deploy.swipeRight()
        let settle = app.buttons["Settle"]
        XCTAssertTrue(settle.waitForExistence(timeout: 5))
        snapshot(app, "settle-swipe")
        settle.tap()
        let shelf = app.cells["section-settled"]
        scrollTo(shelf, in: list)
        setFold(shelf, collapsed: false)
        XCTAssertTrue(deploy.waitForExistence(timeout: 10))
        XCTAssertTrue((deploy.label).hasSuffix("Settled"))
        XCTAssertTrue(app.cells["session-chat-old-mac"].exists)
        scrollTo(deploy, in: list)
        snapshot(app, "settled-shelf")
        deploy.swipeRight()
        let unsettle = app.buttons["Unsettle"]
        XCTAssertTrue(unsettle.waitForExistence(timeout: 5))
        unsettle.tap()
        for _ in 0..<12 { list.swipeDown() }
        scrollTo(deploy, in: list)
        let back = NSPredicate(format: "NOT (label ENDSWITH 'Settled')")
        expectation(for: back, evaluatedWith: deploy)
        waitForExpectations(timeout: 10)
        snapshot(app, "unsettled")
        scrollTo(shelf, in: list)
        setFold(shelf, collapsed: true)
    }

    func testStepThreadsNestUnderTheirLeadAndFold() {
        let (app, list) = launch()
        let lead = app.cells["session-chat-lead-shelf"]
        scrollTo(lead, in: list)
        let step = app.cells["session-chat-shelf-3"]
        XCTAssertTrue(step.exists)
        XCTAssertGreaterThan(step.frame.minX + 1, lead.frame.minX)
        snapshot(app, "nested-lead")
        app.buttons["session-children-chat-lead-shelf"].tap()
        XCTAssertTrue(step.waitForNonExistence(timeout: 5))
        snapshot(app, "nested-lead-folded")
        app.buttons["session-children-chat-lead-shelf"].tap()
        XCTAssertTrue(step.waitForExistence(timeout: 5))
        let btw = app.cells["session-chat-btw"]
        scrollTo(btw, in: list)
        XCTAssertTrue(btw.label.contains("Expired"))
        snapshot(app, "side-chat-expired")
    }

    func testLongPressShowsThePreview() {
        let (app, list) = launch()
        let lead = app.cells["session-chat-lead-shelf"]
        scrollTo(lead, in: list)
        lead.press(forDuration: 1.2)
        XCTAssertTrue(app.buttons["Settle"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.otherElements["session-preview"].waitForExistence(timeout: 5))
        snapshot(app, "preview")
    }

    func testSectionsMoveAndFoldInTheSharedOrder() {
        let (app, list) = launch()
        let p0 = app.cells["section-section-p0"]
        let mobile = app.cells["section-section-mobile"]
        scrollTo(mobile, in: list)
        XCTAssertLessThan(p0.frame.minY, mobile.frame.minY)
        mobile.press(forDuration: 1.2)
        let up = app.buttons["Move Up"]
        XCTAssertTrue(up.waitForExistence(timeout: 5))
        snapshot(app, "section-menu")
        up.tap()
        let moved = NSPredicate { _, _ in mobile.frame.minY < p0.frame.minY }
        expectation(for: moved, evaluatedWith: nil)
        waitForExpectations(timeout: 10)
        let wasCollapsed = p0.value as? String == "Collapsed"
        p0.tap()
        let flipped = NSPredicate(format: "value == %@", wasCollapsed ? "Expanded" : "Collapsed")
        expectation(for: flipped, evaluatedWith: p0)
        waitForExpectations(timeout: 10)
        snapshot(app, "section-moved")
    }

    func testTheTodayCardOpensItsSheetAndChecksATask() {
        let (app, _) = launch()
        let card = app.cells["today-card"]
        XCTAssertTrue(card.waitForExistence(timeout: 10))
        expectation(for: NSPredicate(format: "label ENDSWITH 'to do.'"), evaluatedWith: card)
        waitForExpectations(timeout: 10)
        snapshot(app, "today-card")
        card.tap()
        let sheet = app.tables["today-sheet"]
        XCTAssertTrue(sheet.waitForExistence(timeout: 5))
        let open = app.cells.matching(NSPredicate(format: "identifier BEGINSWITH 'today-task-' AND value == 'Open'")).firstMatch
        XCTAssertTrue(open.waitForExistence(timeout: 5))
        let id = open.identifier
        snapshot(app, "today-sheet")
        open.tap()
        let task = app.cells[id]
        expectation(for: NSPredicate(format: "value == 'Done'"), evaluatedWith: task)
        waitForExpectations(timeout: 10)
        snapshot(app, "today-task-done")
        let usage = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH 'Left on'")).firstMatch
        scrollTo(usage, in: sheet)
        snapshot(app, "today-usage")
        task.tap()
        expectation(for: NSPredicate(format: "value == 'Open'"), evaluatedWith: task)
        waitForExpectations(timeout: 10)
    }

    func testOrganizeAndSortFollowTheListMenu() {
        let (app, list) = launch(pinnedView: false)
        option(app, "None")
        option(app, "Last updated")
        option(app, "Inbox")
        app.buttons["sessions-options"].tap()
        XCTAssertTrue(app.buttons["By device"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.buttons["Last updated"].isEnabled)
        snapshot(app, "list-menu")
        app.buttons["By project"].tap()
        let chats = app.cells["section-group:chats"]
        scrollTo(chats, in: list)
        snapshot(app, "by-project")
        for _ in 0..<10 { list.swipeUp() }
        XCTAssertFalse(app.cells["section-settled"].exists)
        for _ in 0..<20 { list.swipeDown() }
        option(app, "Active first")
        let veil = app.cells["session-chat-veil"]
        scrollTo(veil, in: list)
        snapshot(app, "active-first")
        option(app, "Active first")

        option(app, "By device")
        let mac = app.cells["section-group:dev-mac"]
        for _ in 0..<20 { list.swipeDown() }
        scrollTo(mac, in: list)
        XCTAssertTrue(mac.label.hasPrefix("MacBook Pro"))
        snapshot(app, "by-device")

        option(app, "None")
        let recent = app.cells["section-recent"]
        for _ in 0..<20 { list.swipeDown() }
        scrollTo(recent, in: list)
        drag(list, from: recent.frame.midY / list.frame.height, to: 0.25)
        let updated = rows(under: recent, in: app)
        XCTAssertGreaterThan(updated.count, 3)
        option(app, "Created")
        let resorted = NSPredicate { _, _ in self.rows(under: recent, in: app) != updated }
        expectation(for: resorted, evaluatedWith: nil)
        waitForExpectations(timeout: 10)
        snapshot(app, "sort-created")
        option(app, "Last updated")
        option(app, "Inbox")
        scrollTo(app.cells["section-settled"], in: list)
    }

    func testTheSettledShelfShowsMore() {
        let (app, list) = launch()
        let shelf = app.cells["section-settled"]
        scrollTo(shelf, in: list)
        setFold(shelf, collapsed: false)
        let more = app.cells["show-more-settled"]
        scrollTo(more, in: list)
        XCTAssertTrue(app.cells["session-chat-old-7"].exists)
        XCTAssertFalse(app.cells["session-chat-old-9"].exists)
        snapshot(app, "settled-show-more")
        more.tap()
        let last = app.cells["session-chat-old-9"]
        XCTAssertTrue(last.waitForExistence(timeout: 5))
        XCTAssertFalse(more.exists)
        scrollTo(last, in: list)
        snapshot(app, "settled-shown")
        for _ in 0..<20 { list.swipeDown() }
        scrollTo(shelf, in: list)
        setFold(shelf, collapsed: true)
    }

    func testTheTodaySheetAddsATaskAndOpensAllTasks() {
        let (app, _) = launch()
        let card = app.cells["today-card"]
        XCTAssertTrue(card.waitForExistence(timeout: 10))
        card.tap()
        let updated = app.staticTexts["today-updated"].firstMatch
        XCTAssertTrue(updated.waitForExistence(timeout: 10))
        XCTAssertTrue(updated.label.hasPrefix("Updated "))
        snapshot(app, "today-updated")
        let field = app.textFields["today-add-task"]
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        field.tap()
        field.typeText("Ship the Today sheet")
        snapshot(app, "today-add-typing")
        field.typeText("\n")
        let added = app.tables["today-sheet"].staticTexts["Ship the Today sheet"]
        XCTAssertTrue(added.waitForExistence(timeout: 10))
        snapshot(app, "today-add-done")
        let all = app.buttons["today-all-tasks"]
        let sheet = app.tables["today-sheet"]
        for _ in 0..<6 where !(all.exists && all.isHittable) { drag(sheet, from: 0.4, to: 0.8) }
        all.tap()
        XCTAssertTrue(app.collectionViews["tasks-list"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.tables["today-sheet"].waitForNonExistence(timeout: 5))
        snapshot(app, "today-all-tasks")
    }
}
