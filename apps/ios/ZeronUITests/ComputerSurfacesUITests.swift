import XCTest

final class ComputerSurfacesUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private func launch(_ args: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-demo", "-fast", "-route", "chat:chat-computer"] + args
        app.launch()
        return app
    }

    private func snapshot(_ app: XCUIApplication, _ name: String) {
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }

    private func label(_ app: XCUIApplication, containing texts: String...) -> XCUIElement {
        let predicates = texts.map { NSPredicate(format: "label CONTAINS %@", $0) }
        return app.descendants(matching: .any).matching(NSCompoundPredicate(andPredicateWithSubpredicates: predicates)).firstMatch
    }

    func testConsentCardIsAnsweredFromThePhone() {
        let app = launch()
        let allow = app.buttons["computer-consent-allow"]
        XCTAssertTrue(allow.waitForExistence(timeout: 10))
        XCTAssertTrue(app.buttons["computer-consent-decline"].exists)
        XCTAssertTrue(app.textViews["composer-input"].exists, "a consent card never replaces the composer")
        snapshot(app, "consent-open")
        allow.tap()
        XCTAssertTrue(allow.waitForNonExistence(timeout: 5))
        XCTAssertTrue(label(app, containing: "until the turn ends", "Allowed").waitForExistence(timeout: 5))
        XCTAssertTrue(label(app, containing: "Calculator shows 4 again").waitForExistence(timeout: 15))
        snapshot(app, "consent-allowed")
    }

    func testEnableOnTheDeniedCardPrefixesTheDraft() {
        let app = launch()
        let enable = app.buttons["computer-denied-enable"]
        XCTAssertTrue(app.scrollViews["transcript"].waitForExistence(timeout: 10))
        for _ in 0..<6 where !(enable.exists && enable.isHittable) {
            app.scrollViews["transcript"].swipeDown()
        }
        XCTAssertTrue(enable.waitForExistence(timeout: 10))
        enable.tap()
        let input = app.textViews["composer-input"]
        XCTAssertTrue(input.waitForExistence(timeout: 5))
        XCTAssertEqual(input.value as? String, "/computer-use ")
        snapshot(app, "denied-enable")
    }

    func testStillPreviewShowsWhileWorkingAndHides() {
        let app = launch(["-still"])
        let still = app.images["computer-still"]
        XCTAssertTrue(still.waitForExistence(timeout: 10))
        snapshot(app, "still-preview")
        app.buttons["computer-still-hide"].tap()
        XCTAssertTrue(still.waitForNonExistence(timeout: 5))
    }
}
