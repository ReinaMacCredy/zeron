import XCTest
@testable import Zeron

final class StoreResidencyTests: XCTestCase {
    func testWarmPreloadSelectsCapAndPendingOutboxChats() {
        let chats = (0..<30).map { index in
            Chat(id: "chat-\(index)", deviceId: "device", title: nil, archived: false,
                 cwd: nil, branch: nil, checkoutId: nil, config: nil,
                 lastMessagePreview: nil, lastMessageAt: nil, createdAt: Int64(index),
                 spaceId: nil, lastSeenAt: nil, roomGen: 2)
        }
        let ids = AppModel.warmPreloadIDs(
            chats: chats,
            hasPendingOutbox: { $0 == "chat-20" || $0 == "chat-29" },
            cap: 12
        )
        XCTAssertEqual(ids.count, 14)
        XCTAssertEqual(Array(ids.prefix(12)), (0..<12).map { "chat-\($0)" })
        XCTAssertEqual(Set(ids.suffix(2)), ["chat-20", "chat-29"])
    }

    func testWarmDialSelectsPendingOutboxChatsBeyondCap() {
        let ids = (0..<9).map { "chat-\($0)" }
        let released = AppModel.warmDialIDs(
            ids: ids,
            hasPendingOutbox: { $0 == "chat-8" },
            cap: 8
        )
        XCTAssertEqual(released, ids)
    }

    func testEvictionOrderIsLruFirstAndSkipsProtectedStores() {
        let order = AppModel.evictionOrder(
            lastUsed: ["old": 1, "protected": 2, "new": 3, "middle": 4]
        ) { $0 == "protected" }
        XCTAssertEqual(order, ["old", "new", "middle"])
    }

    func testEvictionPlanRespectsByteBudgetWithinCountCap() {
        let plan = AppModel.evictionPlan(
            lastUsed: ["old": 1, "middle": 2, "new": 3],
            estimates: ["old": 60, "middle": 50, "new": 40],
            protected: { _ in false },
            countCap: 3,
            byteBudget: 80
        )

        XCTAssertEqual(plan, ["old", "middle"])
    }

    func testEvictionPlanKeepsNewestAndProtectedStores() {
        let plan = AppModel.evictionPlan(
            lastUsed: ["old": 1, "protected": 2, "new": 3],
            estimates: ["old": 60, "protected": 60, "new": 60],
            protected: { $0 == "protected" },
            countCap: 1,
            byteBudget: 60
        )

        XCTAssertEqual(plan, ["old"])
        XCTAssertFalse(plan.contains("protected"))
        XCTAssertFalse(plan.contains("new"))
    }

    func testWarmPreloadStopsAtByteBudgetButIncludesPendingChat() {
        let chats = (0..<4).map { index in
            Chat(id: "chat-\(index)", deviceId: "device", title: nil, archived: false,
                 cwd: nil, branch: nil, checkoutId: nil, config: nil,
                 lastMessagePreview: nil, lastMessageAt: nil, createdAt: Int64(index),
                 spaceId: nil, lastSeenAt: nil, roomGen: 2)
        }
        let ids = AppModel.warmPreloadIDs(
            chats: chats,
            hasPendingOutbox: { $0 == "chat-3" },
            cap: 3,
            snapshotBytes: { _ in 200_000 },
            byteBudget: 2_500_000
        )

        XCTAssertEqual(ids, ["chat-0", "chat-1", "chat-3"])
    }
}
