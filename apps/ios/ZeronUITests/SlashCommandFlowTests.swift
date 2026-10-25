import XCTest

final class SlashCommandFlowTests: XCTestCase {
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

    private func row(_ app: XCUIApplication, _ label: String) -> XCUIElement {
        app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH 'invocation-' AND label == %@", label)).firstMatch
    }

    func testEveryHarnessListsItsCommandsAndZeronsOwn() {
        let harnesses = [
            ("chat-deploy", "claude-code", "/context"),
            ("chat-tabs", "codex", "/approvals"),
            ("chat-agents", "opencode", "/undo"),
            ("chat-pi", "pi", "/tree"),
            ("chat-acp", "acp", "/memory"),
            ("chat-cursor", "cursor", "/compact"),
        ]
        for (chat, harness, native) in harnesses {
            let (app, input) = open(chat)
            input.typeText("/")
            XCTAssertTrue(row(app, native).waitForExistence(timeout: 5), "\(harness): \(native)")
            XCTAssertTrue(row(app, "/btw").exists, "\(harness): /btw")
            snapshot(app, "slash-\(harness)")
            let stop = row(app, "/stop")
            XCTAssertTrue(stop.exists, "\(harness): the whole list, not the first six")
            for _ in 0..<4 where !stop.isHittable { app.scrollViews["suggestions-list"].swipeUp() }
            XCTAssertTrue(stop.isHittable, "\(harness): the list scrolls to its last row")
            snapshot(app, "slash-\(harness)-end")
            input.typeText(XCUIKeyboardKey.delete.rawValue + "$")
            snapshot(app, "skills-\(harness)")
            app.terminate()
        }
    }

    func testSkillsAndFailuresShowTheDesktopStates() {
        var (app, input) = open("chat-pi")
        input.typeText("$")
        XCTAssertTrue(app.staticTexts["invocation-empty"].waitForExistence(timeout: 5))
        XCTAssertEqual(app.staticTexts["invocation-empty"].label, "This agent does not advertise skills")
        snapshot(app, "skills-unsupported")
        app.terminate()
        (app, input) = open("chat-antigravity")
        input.typeText("/")
        XCTAssertTrue(app.staticTexts["invocation-error"].waitForExistence(timeout: 5))
        XCTAssertTrue(row(app, "/rename").exists, "Zeron's commands survive a failed discovery")
        snapshot(app, "slash-error")
        input.typeText("zzz")
        XCTAssertTrue(app.staticTexts["invocation-error"].exists)
        XCTAssertFalse(app.staticTexts["invocation-empty"].exists, "an error replaces the empty message")
        app.terminate()
        (app, input) = open("chat-deploy")
        input.typeText("/qqq")
        XCTAssertTrue(app.staticTexts["invocation-empty"].waitForExistence(timeout: 5))
        XCTAssertEqual(app.staticTexts["invocation-empty"].label, "No matching commands")
        snapshot(app, "slash-no-match")
    }

    func testCompactIsShownButRefusedWhereTheHarnessCannotCompact() {
        let (app, input) = open("chat-cursor")
        input.typeText("/comp")
        let compact = row(app, "/compact")
        XCTAssertTrue(compact.waitForExistence(timeout: 5))
        XCTAssertEqual(compact.value as? String, "Cursor does not support compaction")
        snapshot(app, "slash-compact-blocked")
        compact.tap()
        XCTAssertEqual(input.value as? String, "/comp", "a blocked row is not picked")
    }

    func testZeronCommandsRunOnThePhoneInsteadOfSending() {
        let (app, input) = open("chat-deploy")
        input.typeText("keep this draft /term")
        let terminal = row(app, "/terminal")
        XCTAssertTrue(terminal.waitForExistence(timeout: 5))
        terminal.tap()
        XCTAssertTrue(app.navigationBars.firstMatch.waitForExistence(timeout: 5))
        snapshot(app, "slash-terminal-opened")
        XCTAssertEqual(input.value as? String, "keep this draft ", "only the trigger is consumed")
        app.terminate()
        let (again, typed) = open("chat-deploy")
        typed.typeText("/rename")
        XCTAssertTrue(row(again, "/rename").waitForExistence(timeout: 5))
        again.buttons["composer-send"].tap()
        XCTAssertTrue(again.alerts["Rename Session"].waitForExistence(timeout: 5), "a typed /rename runs Zeron's action")
        snapshot(again, "slash-rename-typed")
        again.alerts.buttons["Cancel"].tap()
        XCTAssertFalse(again.descendants(matching: .any).matching(NSPredicate(format: "label BEGINSWITH 'You: /rename'")).firstMatch.exists, "never sent as a turn")
    }

    func testZeronModelOpensTheModelChip() {
        let (app, input) = open("chat-agents")
        input.typeText("/mod")
        let model = row(app, "/model")
        XCTAssertTrue(model.waitForExistence(timeout: 5))
        model.tap()
        XCTAssertTrue(app.collectionViews.firstMatch.waitForExistence(timeout: 5), "the model chip's menu opens")
        snapshot(app, "slash-model-menu")
        XCTAssertEqual(input.value as? String ?? "", "", "the trigger is consumed")
    }

    func testOwnCommandsOnTheNewSessionPageAreNotSentAsTurns() {
        let app = XCUIApplication()
        app.launchArguments = ["-demo", "-fast", "-route", "new"]
        app.launch()
        let input = app.textViews["composer-input"]
        XCTAssertTrue(input.waitForExistence(timeout: 10))
        input.tap()
        input.typeText("/compact")
        app.buttons["composer-send"].tap()
        XCTAssertTrue(app.staticTexts["Start a chat before compacting its context."].waitForExistence(timeout: 3))
        XCTAssertEqual(input.value as? String, "/compact", "kept, not sent")
        snapshot(app, "new-session-compact-refused")
        input.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: 8) + "/context")
        app.buttons["composer-send"].tap()
        XCTAssertTrue(app.staticTexts["Mac Studio is offline, so Claude Code can't answer /context."].waitForExistence(timeout: 3))
        XCTAssertEqual(input.value as? String, "/context", "kept for an online host")
        snapshot(app, "new-session-context-offline")
        app.buttons["composer-chip-project"].tap()
        let edge = app.buttons.matching(NSPredicate(format: "label == 'edge' OR label BEGINSWITH 'edge,'")).firstMatch
        XCTAssertTrue(edge.waitForExistence(timeout: 5))
        edge.tap()
        app.buttons["composer-send"].tap()
        XCTAssertTrue(app.buttons["card-compact"].waitForExistence(timeout: 20), "a Claude command creates the chat and answers with its card")
        XCTAssertFalse(app.descendants(matching: .any).matching(NSPredicate(format: "label == 'You: /context'")).firstMatch.exists, "never a turn")
        snapshot(app, "new-session-context-card")
    }

    func testClaudeTerminalCommandsListAtTheBottomTagged() {
        let (app, input) = open("chat-deploy")
        input.typeText("/perm")
        let permissions = row(app, "/permissions")
        XCTAssertTrue(permissions.waitForExistence(timeout: 5), "listed though the catalog leaves it out")
        XCTAssertEqual(permissions.value as? String, "Runs only in Claude Code's terminal")
        snapshot(app, "slash-terminal-permissions")
        input.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: 4) + "stat")
        XCTAssertTrue(row(app, "/status").waitForExistence(timeout: 5))
        snapshot(app, "slash-terminal-status")
        input.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: 4))
        let first = app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH 'invocation-'")).firstMatch
        XCTAssertTrue(first.waitForExistence(timeout: 5))
        XCTAssertNotEqual(first.value as? String, "Runs only in Claude Code's terminal", "terminal rows sink below the rest")
    }
}
