import Loro
import XCTest
@testable import Zeron

@MainActor
final class ComputerSurfacesTests: XCTestCase {
    private func store(offline: Bool = false) -> SessionStore {
        SessionStore(chatId: "chat-computer", config: AppConfig(
            edgeURL: URL(string: "http://localhost:1")!, mode: .dev,
            userId: "u", orgId: "o", deviceId: "phone", deviceName: "phone",
            tokens: nil, devBearer: "u@o"), offline: offline)
    }

    private func questions(_ json: String) throws -> [UserInputQuestion] {
        try JSONDecoder().decode([UserInputQuestion].self, from: Data(json.utf8))
    }

    private func inputPart(questions: [[String: Any]], resolved: Bool,
                           answers: [[String: Any]]? = nil) -> MessagePart? {
        var fields: [String: Any] = ["id": "req-1", "kind": "input",
                                     "questions": questions, "resolved": resolved]
        if let answers { fields["answers"] = answers }
        return SessionStore.partFrom(.fromJSON(fields))
    }

    private let consentQuestion: [String: Any] = [
        "id": "consent", "header": "Computer",
        "question": "Allow Computer for this task? The agent wants to run computer_click and will drive this Mac's apps until the turn ends.",
        "options": ["Allow", "Decline"], "multiSelect": false,
        "kind": ["type": "consent", "scope": "task"],
    ]

    func testQuestionKindDecodesConsentScopesAndStaysOptional() throws {
        let decoded = try questions("""
        [{"id":"a","header":"H","question":"Q","options":["x"],"kind":{"type":"consent","scope":"task"}},
         {"id":"b","header":"H","question":"Q","options":["x"],"kind":{"type":"consent","scope":"call"}},
         {"id":"c","header":"H","question":"Q","options":["x"],"multiSelect":true},
         {"id":"d","header":"H","question":"Q","options":["x"],"kind":{"type":"somethingNew","extra":1}},
         {"id":"e","header":"H","question":"Q","options":["x"],"kind":"garbage"}]
        """)
        XCTAssertEqual(decoded.map(\.id), ["a", "b", "c", "d", "e"])
        XCTAssertEqual(decoded[0].kind?.consentScope, .task)
        XCTAssertEqual(decoded[1].kind?.consentScope, .call)
        XCTAssertNil(decoded[2].kind)
        XCTAssertEqual(decoded[2].multiSelect, true)
        XCTAssertFalse(decoded[3].isConsent)
        XCTAssertNil(decoded[4].kind)
        let plain = try JSONEncoder().encode(decoded[2])
        XCTAssertFalse(String(decoding: plain, as: UTF8.self).contains("kind"))
    }

    func testInputPartCarriesPersistedAnswersIntoTheConsentOutcome() throws {
        let pending = try XCTUnwrap(inputPart(questions: [consentQuestion], resolved: false))
        guard case .input(_, let requestId, let qs, let resolved, let answers) = pending else {
            return XCTFail("input part")
        }
        XCTAssertEqual(requestId, "req-1")
        XCTAssertEqual(qs.first?.kind?.consentScope, .task)
        XCTAssertNil(ComputerCopy.consentOutcome(resolved: resolved, answers: answers))

        let allowed = inputPart(questions: [consentQuestion], resolved: true,
                                answers: [["questionId": "consent", "labels": ["Allow"]]])
        let declined = inputPart(questions: [consentQuestion], resolved: true,
                                 answers: [["questionId": "consent", "labels": ["Decline"]]])
        let cancelled = inputPart(questions: [consentQuestion], resolved: true)
        for (part, expected) in [(allowed, ConsentOutcome.allowed), (declined, .declined), (cancelled, .cancelled)] {
            guard case .input(_, _, _, let resolved, let answers)? = part else { return XCTFail("input part") }
            XCTAssertEqual(ComputerCopy.consentOutcome(resolved: resolved, answers: answers), expected)
        }
    }

    func testConsentBecomesATranscriptCardAndNeverTakesTheComposer() throws {
        let store = store(offline: true)
        let consent = try XCTUnwrap(inputPart(questions: [consentQuestion], resolved: false))
        store.setEntries([MessageEntry(id: "m1", role: .assistant, parts: [consent],
                                       createdAt: 1, deviceId: "mac", status: .streaming)])
        XCTAssertNil(store.openInputRequest, "a consent question must not replace the composer")

        var parsers: [String: IncrementalMarkdownParser] = [:]
        var completed: [String: CompletedParse] = [:]
        let rows = TranscriptRowBuilder.rows(entries: store.entries, pendingSends: [],
                                             parsers: &parsers, completed: &completed)
        guard case .computerConsent(let requestId, let questionId, let scope, _, let outcome)? = rows.first?.kind else {
            return XCTFail("consent card row")
        }
        XCTAssertEqual(requestId, "req-1")
        XCTAssertEqual(questionId, "consent")
        XCTAssertEqual(scope, .task)
        XCTAssertNil(outcome)

        let ordinary = try XCTUnwrap(inputPart(questions: [[
            "id": "q1", "header": "Pick", "question": "Which?", "options": ["a", "b"],
        ]], resolved: false))
        store.setEntries([MessageEntry(id: "m1", role: .assistant, parts: [ordinary, consent],
                                       createdAt: 1, deviceId: "mac", status: .streaming)])
        XCTAssertEqual(store.openInputRequest?.questions.first?.id, "q1")
    }

    func testConsentAnswersUseTheEngineLabelsAndSettleLocally() throws {
        let store = store()
        store.answerConsent(requestId: "req-1", questionId: "consent", allow: true)
        store.answerConsent(requestId: "req-1", questionId: "consent", allow: false)
        store.answerConsent(requestId: "req-2", questionId: "consent", allow: false)
        XCTAssertEqual(store.consentSent, ["req-1": true, "req-2": false])

        let commands = try XCTUnwrap(store.doc.getDeepValue().mapValue?["commands"]?.listValue)
        XCTAssertEqual(commands.count, 2, "a second tap on a settled card sends nothing")
        let payloads = commands.compactMap { $0.mapValue?["payload"]?.mapValue }
        XCTAssertEqual(payloads.map { $0["kind"]?.stringValue }, ["respondInput", "respondInput"])
        XCTAssertEqual(payloads.map { $0["requestId"]?.stringValue }, ["req-1", "req-2"])
        let labels = payloads.map { payload in
            payload["answers"]?.listValue?.first?.mapValue?["labels"]?.listValue?.compactMap(\.stringValue)
        }
        XCTAssertEqual(labels, [["Allow"], ["Decline"]])
        XCTAssertEqual(commands.first?.mapValue?["payload"]?.mapValue?["answers"]?.listValue?
            .first?.mapValue?["questionId"]?.stringValue, "consent")
    }

    func testRunSteerAndQueueRowsCarryTheChatsComputerGeneration() throws {
        let store = store()
        XCTAssertEqual(store.computerGeneration, 0)
        try store.doc.getMap(id: "meta").insert(key: "computerGeneration", v: Int64(3))
        store.doc.commit()
        XCTAssertEqual(store.computerGeneration, 3)

        let chat = Chat(id: "chat-computer", deviceId: "mac", title: nil, archived: false,
                        cwd: "/repo", branch: nil, checkoutId: nil, config: nil,
                        lastMessagePreview: nil, lastMessageAt: nil, createdAt: 0,
                        spaceId: nil, lastSeenAt: nil)
        store.sendRun(prompt: "/computer-use open Calculator", chat: chat)
        store.sendSteer(prompt: "/computer-use and press 2")
        store.respondInput(requestId: "req", answers: [])
        store.sendInterrupt()
        store.enqueueMessage(text: "/computer-use later")

        let root = try XCTUnwrap(store.doc.getDeepValue().mapValue)
        let commands = try XCTUnwrap(root["commands"]?.listValue).compactMap(\.mapValue)
        let generation = Dictionary(uniqueKeysWithValues: commands.map {
            ($0["kind"]?.stringValue ?? "", $0["computerGeneration"]?.i64Value)
        })
        XCTAssertEqual(generation["run"], 3)
        XCTAssertEqual(generation["steer"], 3)
        XCTAssertEqual(generation["respondInput"], .some(nil), "the engine stamps messages only")
        XCTAssertEqual(generation["interrupt"], .some(nil))
        let row = try XCTUnwrap(root["queue"]?.listValue?.first?.mapValue)
        XCTAssertEqual(row["computerGeneration"]?.i64Value, 3)
    }

    func testEngineNotesBecomeCardsAndOtherSystemTextStaysMarkdown() {
        func rows(_ parts: [MessagePart]) -> [TranscriptRow] {
            var parsers: [String: IncrementalMarkdownParser] = [:]
            var completed: [String: CompletedParse] = [:]
            return TranscriptRowBuilder.rows(
                entries: [MessageEntry(id: "n", role: .system, parts: parts, createdAt: 1,
                                       deviceId: "mac", status: .complete)],
                pendingSends: [], parsers: &parsers, completed: &completed)
        }
        let cases: [(String, String?, ComputerNotice)] = [
            (ComputerCopy.controlOnDisclosure, nil, .disclosure),
            (ComputerCopy.escapeStopNotice, nil, .escapeStop),
            (ComputerCopy.setupNote, "Allow Accessibility for Zeron in System Settings, then continue.", .setupRequired),
            (ComputerCopy.unavailableNote, "OpenCode does not receive Zeron's Computer tools.", .unavailable),
        ]
        for (text, hint, expected) in cases {
            var parts: [MessagePart] = [.text(id: "t0", text: text)]
            if let hint { parts.append(.text(id: "t1", text: hint)) }
            let built = rows(parts)
            XCTAssertEqual(built.count, 1)
            guard case .computerNotice(let notice, let title, let body)? = built.first?.kind else {
                return XCTFail("notice card for \(text)")
            }
            XCTAssertEqual(notice, expected)
            XCTAssertEqual(title, text)
            XCTAssertEqual(body, hint)
        }
        guard case .markdown? = rows([.text(id: "t0", text: "Context compacted")]).first?.kind else {
            return XCTFail("other notes stay markdown")
        }
    }

    func testComputerToolsShowTitlesAndNeverTheirArguments() {
        let secret = "hunter2-typed-secret"
        let shapes = [
            RenderToolCall(tag: "mcp", fields: ["server": "zeron", "tool": "computer_type_text", "text": secret]),
            RenderToolCall(tag: "unknown", fields: ["name": "mcp__zeron__computer_type_text", "value": secret]),
            RenderToolCall(tag: "unknown", fields: ["name": "Tool: zeron/computer_type_text", "input": secret]),
        ]
        for call in shapes {
            XCTAssertEqual(call.computerTool, "computer_type_text")
            XCTAssertEqual(call.chipLabel, "Type")
            XCTAssertEqual(call.chipSymbol, "cursorarrow")
            for text in [call.chipLabel, call.chipDetail, call.expandedDetail] {
                XCTAssertFalse(text.contains(secret))
            }
        }
        XCTAssertEqual(RenderToolCall(tag: "mcp", fields: ["server": "zeron", "tool": "computer_browser_click"]).chipLabel,
                       "Click in the browser")
        XCTAssertEqual(RenderToolCall(tag: "mcp", fields: ["server": "zeron", "tool": "computer_read_clipboard"]).chipLabel,
                       "Read the clipboard")
        XCTAssertNil(RenderToolCall(tag: "mcp", fields: ["server": "github", "tool": "list_issues"]).computerTool)
        XCTAssertNil(RenderToolCall(tag: "unknown", fields: ["name": "precomputer_click"]).computerTool)
        XCTAssertEqual(ComputerCopy.toolTitles.count, 43)
    }

    func testCapabilityDenialAddsOneCardPerToolAfterItsChip() throws {
        func tool(_ id: String, _ name: String, output: String?) -> MessagePart? {
            var fields: [String: Any] = ["id": id, "kind": "tool", "isError": true,
                                         "call": ["kind": "mcp", "server": "zeron", "tool": name]]
            if let output { fields["output"] = output }
            return SessionStore.partFrom(.fromJSON(fields))
        }
        let denied = #"{"error":{"code":"capability_denied","message":"Computer control is off for this turn; no input was sent."}}"#
        let parts = try [
            XCTUnwrap(tool("c1", "computer_click", output: denied)),
            XCTUnwrap(tool("c2", "computer_click", output: denied)),
            XCTUnwrap(tool("c3", "computer_type_text", output: denied)),
            XCTUnwrap(tool("c4", "computer_click", output: "approval_denied")),
        ]
        guard case .tool(_, _, _, _, let firstDenied) = parts[0],
              case .tool(_, _, _, _, let otherError) = parts[3] else { return XCTFail("tool parts") }
        XCTAssertTrue(firstDenied)
        XCTAssertFalse(otherError)

        var parsers: [String: IncrementalMarkdownParser] = [:]
        var completed: [String: CompletedParse] = [:]
        let rows = TranscriptRowBuilder.rows(
            entries: [MessageEntry(id: "m", role: .assistant, parts: parts, createdAt: 1,
                                   deviceId: "mac", status: .complete)],
            pendingSends: [], parsers: &parsers, completed: &completed)
        let kinds = rows.map { row -> String in
            switch row.kind {
            case .toolGroup(let tools, _): return "tools:\(tools.count)"
            case .computerNotice(.controlDenied, let text, let hint):
                XCTAssertEqual(text, "Computer control is off")
                XCTAssertEqual(hint, "Turn it on in Settings to let the agent use the desktop.")
                return "denied"
            default: return "other"
            }
        }
        XCTAssertEqual(kinds, ["tools:1", "denied", "tools:2", "denied", "tools:1"])
    }

    func testStillFrameReplyDecodesTheBase64Image() throws {
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="
        let reply = try JSONDecoder().decode(ComputerStillFrameReply.self, from: Data("""
        {"frame":{"sequence":7,"timestampMs":12.5,"keyframe":true,"data":"\(png)"}}
        """.utf8))
        XCTAssertEqual(reply.image?.sequence, 7)
        XCTAssertEqual(reply.image?.image.size, CGSize(width: 1, height: 1))
        let none = try JSONDecoder().decode(ComputerStillFrameReply.self, from: Data(#"{"frame":null}"#.utf8))
        XCTAssertNil(none.image)
    }
}
