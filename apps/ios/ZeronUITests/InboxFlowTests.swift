import XCTest

final class InboxFlowTests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private func launch() -> (XCUIApplication, XCUIElement) {
        let app = XCUIApplication()
        app.launchArguments = ["-demo", "-fast"]
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
        for _ in 0..<12 where !(element.exists && element.isHittable) {
            list.swipeUp()
        }
        XCTAssertTrue(element.isHittable)
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

    func testActiveFirstAndGroupByProject() {
        let (app, list) = launch()
        option(app, "Active First")
        let veil = app.cells["session-chat-veil"]
        scrollTo(veil, in: list)
        snapshot(app, "active-first")
        option(app, "Active First")
        option(app, "Group by Project")
        let chats = app.cells["section-group:chats"]
        scrollTo(chats, in: list)
        snapshot(app, "by-project")
        option(app, "Group by Project")
    }
}
