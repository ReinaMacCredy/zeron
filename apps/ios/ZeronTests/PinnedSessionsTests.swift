import XCTest
@testable import Zeron

final class PinnedSessionsTests: XCTestCase {
    func testOrderKeysMatchRustAndRemainDenseForConcurrentWriters() {
        // Shared byte-level vector: midpoint 8, fixed-width HLC encoding, terminator 8.
        let nonce = "0000000000001-000000-device-a"
        let encoded = nonce.utf8.map { String(format: "%02x", $0) }.joined()
        let expected = "8" + encoded + String(repeating: "0", count: (149 - nonce.utf8.count) * 2) + "8"
        let a = PinOrder.between(nil, nil, nonce: nonce)!
        let b = PinOrder.between(nil, nil, nonce: "0000000000001-000000-device-b")!
        XCTAssertEqual(a, expected)
        XCTAssertLessThan(a, b)
        let middle = PinOrder.between(a, b, nonce: "0000000000002-000000-device-c")!
        XCTAssertLessThan(a, middle)
        XCTAssertLessThan(middle, b)
        var upper = a
        for i in 0..<1000 {
            let key = PinOrder.between(nil, upper, nonce: "\(i)-device")!
            XCTAssertTrue(PinOrder.valid(key))
            XCTAssertLessThan(key, upper)
            upper = key
        }
        XCTAssertNil(PinOrder.between("80", nil, nonce: nonce))
        XCTAssertNil(PinOrder.between(a, a, nonce: nonce))
    }

    func testPerPinWritesPreserveMembershipAndSurviveRestart() throws {
        let doc = RegistryDoc(deviceId: "ios-test")
        doc.initializeSidebarPins()
        XCTAssertTrue(doc.changeSidebarPin(id: "a", pinned: true))
        XCTAssertTrue(doc.changeSidebarPin(id: "b", pinned: true))
        XCTAssertEqual(doc.orderedSidebarPins.map(\.id), ["a", "b"])
        XCTAssertTrue(doc.changeSidebarPin(id: "b", pinned: nil, before: "a"))
        XCTAssertEqual(doc.orderedSidebarPins.map(\.id), ["b", "a"])
        XCTAssertTrue(doc.changeSidebarPin(id: "b", pinned: false))
        XCTAssertFalse(doc.changeSidebarPin(id: "b", pinned: nil, before: "a"))
        doc.initializeSidebarPins()
        XCTAssertEqual(doc.orderedSidebarPins.map(\.id), ["a"])
        let restored = try RegistryDoc.from(data: doc.toData(), deviceId: "ios-test")
        XCTAssertEqual(restored.orderedSidebarPins.map(\.id), ["a"])
    }

    func testConcurrentMoveCannotUndoUnpin() {
        let seed = RegistryOp(kind: "sidebarPins", id: "a", op: .upsert,
                              set: ["pinned": .bool(true), "orderKey": .string("8")],
                              hlc: "0000000000001-000000-desktop", clocks: nil)
        let move = RegistryOp(kind: "sidebarPins", id: "a", op: .upsert,
                              set: ["orderKey": .string("c")], hlc: "0000000000003-000000-ios", clocks: nil)
        let unpin = RegistryOp(kind: "sidebarPins", id: "a", op: .upsert,
                               set: ["pinned": .bool(false)], hlc: "0000000000002-000000-desktop", clocks: nil)
        for ops in [[move, unpin], [unpin, move]] {
            var row = applyOp(nil, seed).row
            for op in ops + [seed] { row = applyOp(row, op).row }
            XCTAssertEqual(row?.fields["pinned"], .bool(false))
            XCTAssertEqual(row?.fields["orderKey"], .string("c"))
        }
    }

    func testLocalPinEditsFollowAnObservedFutureClock() {
        let doc = RegistryDoc(deviceId: "local")
        doc.initializeSidebarPins()
        let remote = RegistryOp(kind: "sidebarPins", id: "pin", op: .upsert,
                                set: ["pinned": .bool(true), "orderKey": .string("8")],
                                hlc: "9999999999999-000001-remote", clocks: nil)
        var row = applyOp(nil, remote).row!
        row.seq = 1
        _ = doc.applyRows(seq: 1, rows: [row])
        XCTAssertTrue(doc.changeSidebarPin(id: "pin", pinned: false))
        XCTAssertTrue(doc.orderedSidebarPins.isEmpty)
        XCTAssertTrue(doc.changeSidebarPin(id: "pin", pinned: true))
        XCTAssertEqual(doc.orderedSidebarPins.map(\.id), ["pin"])
        XCTAssertGreaterThan(doc.overlayRow(kind: "sidebarPins", id: "pin")!.clocks["pinned"]!, remote.hlc)
    }

    func testConcurrentMovesOfOnePinConvergeInEitherOrder() {
        let a = RegistryOp(kind: "sidebarPins", id: "pin", op: .upsert,
                           set: ["orderKey": .string("8")], hlc: "0000000000002-000000-a", clocks: nil)
        let b = RegistryOp(kind: "sidebarPins", id: "pin", op: .upsert,
                           set: ["orderKey": .string("c")], hlc: "0000000000002-000000-b", clocks: nil)
        for ops in [[a, b], [b, a]] {
            var row: RegistryRow?
            for op in ops { row = applyOp(row, op).row }
            XCTAssertEqual(row?.fields["orderKey"], .string("c"))
        }
    }
    private func chat(_ id: String, activity: Int64, archived: Bool = false) -> Chat {
        Chat(id: id, deviceId: "device", title: id, archived: archived,
             cwd: nil, branch: nil, checkoutId: nil, config: nil,
             lastMessagePreview: nil, lastMessageAt: activity, createdAt: activity,
             spaceId: "space", lastSeenAt: activity)
    }

    func testPinsLeadInSharedOrderWithoutDisturbingRecency() {
        let chats = [chat("recent", activity: 30), chat("middle", activity: 20),
                     chat("old", activity: 10)]
        XCTAssertEqual(
            sortPinnedFirst(chats, pinnedSessionIds: ["old", "missing", "old"])
                .map(\.id),
            ["old", "recent", "middle"]
        )
    }

    func testOldPreferencesAreNotImported() {
        let doc = RegistryDoc(deviceId: "ios-test")
        doc.write(kind: "preferences", id: "sidebar-v1", op: .upsert, set: [
            "pinnedSessionIds": .array([.string("chat-b"), .string("chat-a")]),
        ])
        doc.initializeSidebarPins()
        XCTAssertTrue(doc.sidebarPinsInitialized)
        XCTAssertTrue(doc.orderedSidebarPins.isEmpty)
    }
}
