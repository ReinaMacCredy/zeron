import XCTest

final class AgentsPillFlowTests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private func launch() -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-demo", "-fast", "-route", "chat:chat-home"]
        app.launch()
        return app
    }

    private func snapshot(_ app: XCUIApplication, _ name: String) {
        let screen = app.screenshot()
        let shot = XCTAttachment(screenshot: screen)
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
        if let dir = ProcessInfo.processInfo.environment["PROOF_DIR"] {
            try? screen.pngRepresentation.write(to: URL(fileURLWithPath: dir).appendingPathComponent("\(name).png"))
        }
    }

    private func tapTranscript(_ app: XCUIApplication) {
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.3)).tap()
    }

    func testAgentsPillOpensTheRosterCardAboveTheComposer() {
        let app = launch()
        let pill = app.buttons["agents-pill"]
        XCTAssertTrue(pill.waitForExistence(timeout: 15))
        XCTAssertEqual(pill.label, "2 of 4 working · 1 shell")
        snapshot(app, "1-pill-keyboard-down")

        let input = app.textViews["composer-input"]
        input.tap()
        let tip = app.buttons["Continue"]
        if tip.waitForExistence(timeout: 2) { tip.tap() }
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
        XCTAssertTrue(pill.isHittable)
        XCTAssertLessThan(pill.frame.maxY, input.frame.minY)
        snapshot(app, "2-pill-keyboard-up")
        tapTranscript(app)
        XCTAssertTrue(app.keyboards.firstMatch.waitForNonExistence(timeout: 5))

        let composer = input.frame
        pill.tap()
        let heading = app.staticTexts["agents-card-heading"]
        XCTAssertTrue(heading.waitForExistence(timeout: 5))
        XCTAssertEqual(heading.label, "2 of 4 working")
        XCTAssertEqual(pill.value as? String, "Open")
        XCTAssertEqual(input.frame, composer)
        XCTAssertEqual(app.staticTexts["agents-card-row-3-title"].label, "Reproduce the composer jump when the key…")
        XCTAssertTrue(app.staticTexts["agents-card-row-3-line"].label.hasPrefix("Failed"))
        XCTAssertTrue(app.buttons["agents-card-stop-1"].exists)
        XCTAssertFalse(app.buttons["agents-card-stop-0"].exists)
        XCTAssertTrue(app.staticTexts["agents-card-shell-0-title"].exists)
        XCTAssertLessThan(heading.frame.maxY, pill.frame.minY)
        snapshot(app, "3-card-open-long-title")

        tapTranscript(app)
        XCTAssertTrue(heading.waitForNonExistence(timeout: 5))
        XCTAssertEqual(pill.value as? String ?? "", "")
        snapshot(app, "4-after-outside-tap")

        pill.tap()
        XCTAssertTrue(heading.waitForExistence(timeout: 5))
        pill.tap()
        XCTAssertTrue(heading.waitForNonExistence(timeout: 5))
        pill.tap()
        XCTAssertTrue(heading.waitForExistence(timeout: 5))
        app.buttons["agents-card-close"].tap()
        XCTAssertTrue(heading.waitForNonExistence(timeout: 5))

        pill.tap()
        XCTAssertTrue(heading.waitForExistence(timeout: 5))
        app.staticTexts["agents-card-shell-0-title"].tap()
        XCTAssertTrue(app.textViews["shell-output"].waitForExistence(timeout: 5))
        snapshot(app, "5-shell-output-sheet")
        app.buttons["shell-stop"].tap()
        XCTAssertTrue(app.textViews["shell-output"].waitForNonExistence(timeout: 5))
        XCTAssertTrue(heading.exists)
        XCTAssertTrue(app.staticTexts["agents-card-shell-0-title"].waitForNonExistence(timeout: 5))
        XCTAssertEqual(pill.label, "2 of 4 working")

        app.buttons["agents-card-stop-1"].tap()
        let settled = NSPredicate(format: "label == %@", "1 of 4 working")
        expectation(for: settled, evaluatedWith: heading)
        waitForExpectations(timeout: 5)
        XCTAssertEqual(pill.label, "1 of 4 working")
        snapshot(app, "6-after-stop")
    }
}
