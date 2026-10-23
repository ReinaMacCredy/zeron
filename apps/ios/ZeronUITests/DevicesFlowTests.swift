import XCTest

final class DevicesFlowTests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private func launch(_ route: String, _ args: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-demo", "-fast", "-route", route] + args
        app.launch()
        return app
    }

    private func snapshot(_ app: XCUIApplication, _ name: String) {
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }

    private func label(_ app: XCUIApplication, containing text: String) -> XCUIElement {
        app.descendants(matching: .any).matching(NSPredicate(format: "label CONTAINS %@", text)).firstMatch
    }

    private func openMenu(_ app: XCUIApplication) {
        let menu = app.buttons["session-menu"]
        XCTAssertTrue(menu.waitForExistence(timeout: 10))
        menu.tap()
    }

    func testStopComputerUseConfirmsThenEndsThePreview() {
        let app = launch("chat:chat-computer", ["-still"])
        let still = app.images["computer-still"]
        XCTAssertTrue(still.waitForExistence(timeout: 10))
        openMenu(app)
        let stop = app.buttons["Stop Computer Use"]
        XCTAssertTrue(stop.waitForExistence(timeout: 5))
        snapshot(app, "stop-before")
        stop.tap()
        let alert = app.alerts["Stop computer use?"]
        XCTAssertTrue(alert.waitForExistence(timeout: 5))
        XCTAssertTrue(alert.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "for this chat")).firstMatch.exists)
        snapshot(app, "stop-confirm")
        alert.buttons["Stop"].tap()
        XCTAssertTrue(still.waitForNonExistence(timeout: 5))
        XCTAssertTrue(label(app, containing: "Desktop access revoked by you").waitForExistence(timeout: 10))
        snapshot(app, "stop-after")
        openMenu(app)
        XCTAssertTrue(app.buttons["Terminal"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.buttons["Stop Computer Use"].exists, "nothing left to stop")
    }

    func testTerminalSheetPaintsTheMacsScreenReadOnly() {
        let app = launch("chat:chat-computer")
        openMenu(app)
        let item = app.buttons["Terminal"]
        XCTAssertTrue(item.waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["Project Actions"].exists)
        snapshot(app, "terminal-before")
        item.tap()
        let output = app.textViews["terminal-output"]
        XCTAssertTrue(output.waitForExistence(timeout: 10))
        let failed = NSPredicate(format: "value CONTAINS %@", "test result: FAILED")
        expectation(for: failed, evaluatedWith: output)
        waitForExpectations(timeout: 10)
        XCTAssertTrue(app.staticTexts["terminal-footer"].exists)
        XCTAssertTrue(app.buttons["terminal-tab-term-demo-1"].exists)
        snapshot(app, "terminal-open")
    }

    func testRunningAProjectActionFollowsItsTerminal() {
        let app = launch("chat:chat-computer/actions")
        let run = app.buttons["project-action-run-act-lint"]
        XCTAssertTrue(run.waitForExistence(timeout: 10))
        XCTAssertTrue(app.buttons["project-action-run-act-dev"].exists)
        snapshot(app, "run-before")
        run.tap()
        let status = app.staticTexts["project-action-status-act-lint"]
        XCTAssertTrue(status.waitForExistence(timeout: 5))
        let failed = NSPredicate(format: "label == %@", "Failed (exit 1)")
        expectation(for: failed, evaluatedWith: status)
        waitForExpectations(timeout: 10)
        snapshot(app, "run-after")
        app.cells["project-action-act-lint"].tap()
        let output = app.textViews["terminal-output"]
        XCTAssertTrue(output.waitForExistence(timeout: 10))
        expectation(for: NSPredicate(format: "value CONTAINS %@", "cargo clippy --workspace"), evaluatedWith: output)
        waitForExpectations(timeout: 10)
        snapshot(app, "run-terminal")
    }

    func testTheMacPageStopsEveryChatUsingTheComputer() {
        let app = launch("page:mac", ["-still"])
        let row = app.staticTexts["Stop computer use"]
        for _ in 0..<4 where !(row.exists && row.isHittable) {
            app.swipeUp()
        }
        XCTAssertTrue(row.waitForExistence(timeout: 10))
        snapshot(app, "mac-stop-before")
        row.tap()
        let confirm = app.buttons["Stop"]
        XCTAssertTrue(confirm.waitForExistence(timeout: 5))
        XCTAssertTrue(label(app, containing: "Stop computer use on").exists)
        snapshot(app, "mac-stop-confirm")
        confirm.tap()
        XCTAssertTrue(row.waitForExistence(timeout: 10), "the page reloads after the stop")
    }
}
