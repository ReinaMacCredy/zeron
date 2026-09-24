import XCTest
@testable import Zeron

@MainActor
final class SessionStoreLifecycleTests: XCTestCase {
    private func config() -> AppConfig {
        AppConfig(edgeURL: URL(string: "http://localhost:1")!, mode: .dev,
                  userId: "u", orgId: "o", deviceId: "phone",
                  deviceName: "phone", tokens: nil, devBearer: "u@o")
    }

    func testStopPreventsDeferredDialAndRoomGenUpdates() {
        let id = "lifecycle-stop-\(UUID().uuidString)"
        defer { try? FileManager.default.removeItem(at: DocDisk.chat2URL(for: id)) }
        let store = SessionStore(chatId: id, config: config())
        store.start(holdDial: true)
        store.updateRoomGen(2)
        XCTAssertFalse(store.roomActive)

        store.stop()
        store.releaseDial()
        store.kickRoom()
        store.updateRoomGen(3)

        XCTAssertTrue(store.stopped)
        XCTAssertFalse(store.roomActive)
    }

    func testStoppedStoreDoesNotReviveWhenReplacementStarts() {
        let id = "lifecycle-replacement-\(UUID().uuidString)"
        defer { try? FileManager.default.removeItem(at: DocDisk.chat2URL(for: id)) }
        let old = SessionStore(chatId: id, config: config())
        old.start(holdDial: true)
        old.updateRoomGen(2)
        old.stop()

        let replacement = SessionStore(chatId: id, config: config())
        replacement.start()
        replacement.updateRoomGen(2)
        XCTAssertTrue(replacement.roomActive)

        old.releaseDial()
        XCTAssertFalse(old.roomActive)
        replacement.stop()
    }

    func testStartAfterStopIsNoOp() {
        let id = "lifecycle-restart-\(UUID().uuidString)"
        defer { try? FileManager.default.removeItem(at: DocDisk.chat2URL(for: id)) }
        let store = SessionStore(chatId: id, config: config())
        store.start(holdDial: true)
        store.stop()
        store.start()
        store.updateRoomGen(2)

        XCTAssertFalse(store.roomActive)
        XCTAssertTrue(store.stopped)
    }
}
