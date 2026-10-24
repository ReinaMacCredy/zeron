import XCTest

/// Against a real stack: `wrangler dev` edge (AUTH_MODE=dev) + a headless
/// engine running the mock harness, both as alice@org1. Skipped unless
/// `TEST_RUNNER_ZERON_LIVE_EDGE=http://localhost:<port>` is set:
///
///   (cd edge && npx wrangler dev --port 27650 --var AUTH_MODE:dev) &
///   ZERON_DATA_DIR=/tmp/e ZERON_IPC_PORT=27811 ZERON_EDGE_URL=http://localhost:27650 \
///     ZERON_EDGE_TOKEN=alice@org1 ZERON_ORG_ID=org1 ZERON_HARNESS=mock zeron headless &
///   TEST_RUNNER_ZERON_LIVE_EDGE=http://localhost:27650 xcodebuild test \
///     -only-testing:ZeronUITests/LiveStackTests …
final class LiveStackTests: XCTestCase {
    func testSendRoundTripsThroughRealEngine() throws {
        let edge = try XCTUnwrap(ProcessInfo.processInfo.environment["ZERON_LIVE_EDGE"], "set TEST_RUNNER_ZERON_LIVE_EDGE")
        let app = XCUIApplication()
        // The draft's harness wins over the engine's ZERON_HARNESS default:
        // pin the mock so the test never starts a real agent.
        app.launchArguments = ["-signedout", "-dev", "alice", "org1", "-edge", edge, "-harness", "mock"]
        app.launch()

        let accessory = app.buttons["new-session"]
        XCTAssertTrue(accessory.waitForExistence(timeout: 15))
        accessory.tap()
        let input = app.textViews["composer-input"]
        XCTAssertTrue(input.waitForExistence(timeout: 5))
        input.typeText("hello from the phone")
        app.buttons["composer-send"].tap()

        // The engine adopts the command (echo keeps its id) and the mock
        // harness replies through the chat2 room.
        let echo = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH 'You: hello from the phone'")).firstMatch
        XCTAssertTrue(echo.waitForExistence(timeout: 10))
        let reply = app.staticTexts.matching(identifier: "row-markdown").firstMatch
        XCTAssertTrue(reply.waitForExistence(timeout: 60), "mock harness reply streamed back")
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = "live-roundtrip"
        shot.lifetime = .keepAlways
        add(shot)
        XCTAssertTrue(app.buttons["Send message"].waitForExistence(timeout: 60), "turn settles")
    }

    func testWallpaperFollowsTheMac() throws {
        let env = ProcessInfo.processInfo.environment
        let edge = try XCTUnwrap(env["ZERON_LIVE_EDGE"], "set TEST_RUNNER_ZERON_LIVE_EDGE")
        let dataDir = try XCTUnwrap(env["ZERON_LIVE_DATA_DIR"])
        let artA = try XCTUnwrap(env["ZERON_LIVE_ART_A"]), artB = try XCTUnwrap(env["ZERON_LIVE_ART_B"])
        let phoneArt = try XCTUnwrap(env["ZERON_LIVE_ART_PHONE"])
        func macSettings(_ art: String, _ chat: [String: Any]) throws {
            let settings: [String: Any] = [
                "newThreadComposerBackground": ["path": art, "name": (art as NSString).lastPathComponent],
                "newThreadBackgroundEffect": "none",
                "chatBackground": chat,
            ]
            try JSONSerialization.data(withJSONObject: settings).write(to: URL(fileURLWithPath: dataDir).appendingPathComponent("ui-settings.json"))
        }
        try macSettings(artA, ["enabled": true, "placement": "full", "opacity": 0.7, "blur": 0, "scrim": true])

        let app = XCUIApplication()
        app.launchArguments = ["-signedout", "-dev", "alice", "org1", "-edge", edge, "-harness", "mock", "-wallpaper", phoneArt, "-route", "more"]
        app.launch()
        for _ in 0..<6 {
            guard let step = ["Skip for now", "Continue", "Not now", "I'll do this later"].map({ app.buttons[$0] }).first(where: { $0.waitForExistence(timeout: 2) }) else { break }
            step.tap()
        }
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 15))
        let sync = app.switches["toggle-wallpaper:sync"]
        for _ in 0..<6 where !sync.isHittable { app.swipeUp() }
        if sync.value as? String == "1" { sync.tap() }
        XCTAssertTrue(app.staticTexts[(phoneArt as NSString).lastPathComponent].waitForExistence(timeout: 5))
        shot(app, "sync-off")

        let enabled = NSPredicate(format: "isEnabled == true")
        expectation(for: enabled, evaluatedWith: sync)
        waitForExpectations(timeout: 20)
        sync.tap()
        XCTAssertTrue(app.staticTexts[(artA as NSString).lastPathComponent].waitForExistence(timeout: 20), "the Mac's artwork arrives")
        XCTAssertTrue(app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH 'Follows '")).firstMatch.exists)
        shot(app, "sync-on-top")
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.75)).press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.4)))
        XCTAssertTrue(app.staticTexts["Scrim"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.sliders["slider-wallpaper:opacity"].isEnabled, "the Mac's settings are read-only here")
        shot(app, "sync-on")

        app.buttons["new-session"].tap()
        let input = app.textViews["composer-input"]
        XCTAssertTrue(input.waitForExistence(timeout: 5))
        input.typeText("wallpaper check")
        app.buttons["composer-send"].tap()
        let notNow = XCUIApplication(bundleIdentifier: "com.apple.springboard").buttons.matching(NSPredicate(format: "label BEGINSWITH 'Don'")).firstMatch
        if notNow.waitForExistence(timeout: 5) { notNow.tap() }
        XCTAssertTrue(app.staticTexts.matching(identifier: "row-markdown").firstMatch.waitForExistence(timeout: 60))
        shot(app, "sync-chat-a")

        try macSettings(artB, ["enabled": true, "placement": "hero", "opacity": 0.9, "blur": 0, "scrim": false])
        XCUIDevice.shared.press(.home)
        sleep(2)
        app.activate()
        sleep(15)
        shot(app, "sync-chat-b-after-foreground")
    }

    private func shot(_ app: XCUIApplication, _ name: String) {
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }
}
