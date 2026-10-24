import XCTest

final class ClaudeConfigFlowTests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private func open(_ chat: String) -> (XCUIApplication, XCUIElement) {
        let app = XCUIApplication()
        app.launchArguments = ["-demo", "-fast", "-route", "chat:\(chat)"]
        app.launch()
        let input = app.textViews["composer-input"]
        XCTAssertTrue(input.waitForExistence(timeout: 10))
        input.tap()
        return (app, input)
    }

    private func snapshot(_ app: XCUIApplication, _ name: String) {
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = name
        shot.lifetime = .keepAlways
        add(shot)
    }

    private func card(_ app: XCUIApplication, containing text: String) -> XCUIElement {
        app.descendants(matching: .any).matching(NSPredicate(format: "identifier == 'row-card' AND label CONTAINS %@", text)).firstMatch
    }

    private func send(_ app: XCUIApplication, _ input: XCUIElement, _ text: String) {
        input.tap()
        input.typeText(text)
        app.buttons["composer-send"].tap()
    }

    func testConfigOpensThePanelAndClosingLeavesASummary() {
        let (app, input) = open("chat-deploy")
        send(app, input, "/config")
        let table = app.tables["claude-config"]
        XCTAssertTrue(table.waitForExistence(timeout: 5), "a bare /config opens the panel instead of a turn")
        XCTAssertTrue(app.cells["claude-config-row-autoCompact"].waitForExistence(timeout: 10))
        snapshot(app, "config-medium")
        table.swipeUp()
        snapshot(app, "config-large")

        let search = app.descendants(matching: .any)["claude-config-search"]
        search.tap()
        search.typeText("theme")
        XCTAssertTrue(app.cells["claude-config-row-theme"].waitForExistence(timeout: 3))
        snapshot(app, "config-search")
        search.typeText("zzz")
        XCTAssertTrue(app.cells["claude-config-message"].waitForExistence(timeout: 3))
        snapshot(app, "config-search-none")
        search.buttons["Clear text"].tap()
        search.typeText("theme")

        app.cells["claude-config-row-theme"].tap()
        let light = app.cells["claude-config-option-light"]
        XCTAssertTrue(light.waitForExistence(timeout: 3))
        snapshot(app, "config-options")
        light.tap()
        let themeValue = app.cells["claude-config-row-theme"].staticTexts["light"]
        XCTAssertTrue(themeValue.waitForExistence(timeout: 10), "the write lands and the row shows it")

        search.buttons["Clear text"].tap()
        search.typeText("auto-compact")
        let toggle = app.switches["claude-config-toggle-autoCompact"]
        XCTAssertTrue(toggle.waitForExistence(timeout: 3))
        toggle.tap()
        expectation(for: NSPredicate(format: "value == '0'"), evaluatedWith: toggle)
        waitForExpectations(timeout: 10)
        snapshot(app, "config-toggled")

        search.buttons["Clear text"].tap()
        app.buttons["claude-config-filter-Terminal only"].tap()
        snapshot(app, "config-terminal-filter")

        let tabs = app.segmentedControls["claude-config-tabs"]
        tabs.buttons["Status"].tap()
        XCTAssertTrue(app.cells["claude-config-status-MCP servers"].waitForExistence(timeout: 5))
        snapshot(app, "config-status")
        tabs.buttons["Usage"].tap()
        XCTAssertTrue(app.cells["claude-config-meter-Current session"].waitForExistence(timeout: 5))
        snapshot(app, "config-usage")
        tabs.buttons["Stats"].tap()
        XCTAssertTrue(app.cells["claude-config-tiles"].waitForExistence(timeout: 10))
        snapshot(app, "config-stats")

        app.buttons["claude-config-close"].tap()
        let summary = card(app, containing: "Claude Code settings")
        XCTAssertTrue(summary.waitForExistence(timeout: 10), "closing leaves the summary row")
        XCTAssertTrue(summary.label.contains("Theme → light"), summary.label)
        XCTAssertTrue(summary.label.contains("Auto-compact → false"), summary.label)
        snapshot(app, "config-summary")
        app.buttons["card-open-claude-config"].firstMatch.tap()
        XCTAssertTrue(table.waitForExistence(timeout: 5), "Reopen opens the panel again")
    }

    func testClaudeCommandsAnswerWithCardsInsteadOfTurns() {
        let (app, input) = open("chat-deploy")
        send(app, input, "/usage")
        XCTAssertTrue(app.buttons["card-open-usage"].waitForExistence(timeout: 15))
        snapshot(app, "card-usage")
        send(app, input, "/context")
        XCTAssertTrue(app.buttons["card-compact"].waitForExistence(timeout: 15))
        XCTAssertTrue(card(app, containing: "Context").exists)
        snapshot(app, "card-context")
        send(app, input, "/model")
        let chip = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH 'card-model-'")).firstMatch
        XCTAssertTrue(chip.waitForExistence(timeout: 15))
        snapshot(app, "card-model")
        send(app, input, "/mcp")
        XCTAssertTrue(card(app, containing: "MCP servers").waitForExistence(timeout: 15))
        snapshot(app, "card-mcp")
        send(app, input, "/permissions")
        XCTAssertTrue(app.buttons["card-open-claude-permissions"].waitForExistence(timeout: 15))
        XCTAssertTrue(card(app, containing: "only runs in Claude Code's terminal").exists)
        snapshot(app, "card-terminal-only")
        for command in ["/usage", "/context", "/model", "/mcp", "/permissions"] {
            XCTAssertFalse(app.descendants(matching: .any).matching(NSPredicate(format: "label == %@", "You: \(command)")).firstMatch.exists, "\(command) is never a turn")
        }
        send(app, input, "/status")
        let status = app.buttons["card-open-claude-status"].firstMatch
        XCTAssertTrue(status.waitForExistence(timeout: 15))
        status.tap()
        XCTAssertTrue(app.cells["claude-config-status-MCP servers"].waitForExistence(timeout: 10), "Open Status opens the panel's Status tab")
        snapshot(app, "card-open-status")
    }

    func testConfigWithArgumentsStaysATurn() {
        let (app, input) = open("chat-deploy")
        send(app, input, "/config theme=dark")
        XCTAssertTrue(app.descendants(matching: .any).matching(NSPredicate(format: "label BEGINSWITH 'You: /config theme=dark'")).firstMatch.waitForExistence(timeout: 5))
        XCTAssertFalse(app.tables["claude-config"].exists)
    }
}
