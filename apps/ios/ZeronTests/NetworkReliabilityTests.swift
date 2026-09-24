// Tests for the network-reliability port (desktop PRs #159/#164/#165/#168/
// #170): version gates, pending:// ref round-trips, the upload chunk plan's
// wire invariants, the whole-attachment deadline, and the relay echo frame.

import Loro
import XCTest
@testable import Zeron

final class NetworkReliabilityTests: XCTestCase {
    func testChat2SnapshotRoundTripsVerifiedFlag() throws {
        let id = "verified-\(UUID().uuidString)"
        defer { try? FileManager.default.removeItem(at: DocDisk.chat2URL(for: id)) }
        let doc = LoroDoc()
        DocDisk.saveChat2(doc: doc, id: id, cursor: 42, verified: true)

        let loaded = DocDisk.loadChat2(into: LoroDoc(), id: id)
        XCTAssertEqual(loaded?.cursor, 42)
        XCTAssertEqual(loaded?.verified, true)
        XCTAssertEqual(loaded?.outbox.count, 0)
    }

    func testChat2Snapshot03RoundTripsOutboxAndSnapshot() throws {
        let id = "outbox-\(UUID().uuidString)"
        defer { try? FileManager.default.removeItem(at: DocDisk.chat2URL(for: id)) }
        let doc = LoroDoc()
        let map = doc.getMap(id: "test")
        try map.insert(key: "value", v: "persisted")
        doc.commit()
        let outbox: [(batchId: String, bytes: Data)] = [
            ("batch-a", Data([1, 2, 3])),
            ("batch-b", Data([4, 5])),
        ]
        XCTAssertTrue(DocDisk.saveChat2(doc: doc, id: id, cursor: 42, verified: true,
                                        outbox: outbox))

        let restoredDoc = LoroDoc()
        let loaded = try XCTUnwrap(DocDisk.loadChat2(into: restoredDoc, id: id))
        XCTAssertEqual(loaded.cursor, 42)
        XCTAssertTrue(loaded.verified)
        XCTAssertFalse(loaded.firstContactQueued)
        XCTAssertEqual(loaded.outbox.map(\.batchId), outbox.map(\.batchId))
        XCTAssertEqual(loaded.outbox.map(\.bytes), outbox.map(\.bytes))
        XCTAssertEqual(restoredDoc.getDeepValue().mapValue?["test"]?.mapValue?["value"]?.stringValue,
                       "persisted")
    }

    func testChat2PendingOutboxProtectsOldestSnapshotDuringPrune() throws {
        let ids = (0..<3).map { "prune-\($0)-\(UUID().uuidString)" }
        defer {
            ids.forEach { try? FileManager.default.removeItem(at: DocDisk.chat2URL(for: $0)) }
        }
        for (index, id) in ids.enumerated() {
            let outbox: [(batchId: String, bytes: Data)] = index == 0
                ? [("pending", Data([1]))] : []
            XCTAssertTrue(DocDisk.saveChat2(doc: LoroDoc(), id: id, cursor: 0,
                                            verified: false, outbox: outbox))
            let date = Date(timeIntervalSince1970: TimeInterval(index + 1))
            try FileManager.default.setAttributes([.modificationDate: date],
                                                  ofItemAtPath: DocDisk.chat2URL(for: id).path)
        }
        DocDisk.prune(keep: 1)
        XCTAssertTrue(FileManager.default.fileExists(atPath: DocDisk.chat2URL(for: ids[0]).path))
        XCTAssertFalse(FileManager.default.fileExists(atPath: DocDisk.chat2URL(for: ids[1]).path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: DocDisk.chat2URL(for: ids[2]).path))
    }

    func testBackfillDeadlineUsesSeparateCheckpointAndRowClocks() {
        let now = DispatchTime.now().uptimeNanoseconds
        let checkpoint = DispatchTime(uptimeNanoseconds: now - 150_000_000_000)
        let recentRows = DispatchTime(uptimeNanoseconds: now - 10_000_000_000)
        let oldRows = DispatchTime(uptimeNanoseconds: now - 130_000_000_000)
        XCTAssertFalse(ChatRoomClient.backfillExpired(
            now: .now(), fetchInFlight: false,
            backfillStartedAt: checkpoint, rowPhaseStartedAt: recentRows
        ))
        XCTAssertTrue(ChatRoomClient.backfillExpired(
            now: .now(), fetchInFlight: false,
            backfillStartedAt: checkpoint, rowPhaseStartedAt: oldRows
        ))
    }

    func testChat2Snapshot02LoadsWithEmptyOutbox() throws {
        let id = "snap02-\(UUID().uuidString)"
        defer { try? FileManager.default.removeItem(at: DocDisk.chat2URL(for: id)) }
        let snapshot = try LoroDoc().export(mode: .snapshot)
        var data = Data("C2SNAP02".utf8)
        var cursor = UInt64(23).littleEndian
        withUnsafeBytes(of: &cursor) { data.append(contentsOf: $0) }
        data.append(1)
        data.append(snapshot)
        try data.write(to: DocDisk.chat2URL(for: id), options: .atomic)

        let loaded = try XCTUnwrap(DocDisk.loadChat2(into: LoroDoc(), id: id))
        XCTAssertEqual(loaded.cursor, 23)
        XCTAssertTrue(loaded.verified)
        XCTAssertTrue(loaded.outbox.isEmpty)
    }

    func testChat2SnapshotReadsLegacyUnverifiedFormat() throws {
        let id = "legacy-\(UUID().uuidString)"
        defer { try? FileManager.default.removeItem(at: DocDisk.chat2URL(for: id)) }
        let doc = LoroDoc()
        let snapshot = try doc.export(mode: .snapshot)
        var data = Data("C2SNAP01".utf8)
        var cursor = UInt64(17).littleEndian
        withUnsafeBytes(of: &cursor) { data.append(contentsOf: $0) }
        data.append(snapshot)
        try data.write(to: DocDisk.chat2URL(for: id), options: .atomic)

        let loaded = DocDisk.loadChat2(into: LoroDoc(), id: id)
        XCTAssertEqual(loaded?.cursor, 17)
        XCTAssertEqual(loaded?.verified, false)
        XCTAssertTrue(loaded?.outbox.isEmpty ?? false)
    }

    @MainActor
    func testChat2RejectsEmptyOrUnreadableFrontier() {
        let doc = LoroDoc()
        XCTAssertFalse(SessionStore.containsFrontier(Data(), in: doc))
        XCTAssertFalse(SessionStore.containsFrontier(Data([0xff, 0x00]), in: doc))
    }

    func testModelCacheRoundTrips() throws {
        let deviceId = "cache-device-\(UUID().uuidString)"
        let harness = "claude-code"
        defer { try? FileManager.default.removeItem(at: DocDisk.modelsURL(deviceId: deviceId,
                                                                           harness: harness)) }
        let models = HarnessCatalog.models(for: harness)
        XCTAssertTrue(DocDisk.saveModels(models, deviceId: deviceId, harness: harness))
        XCTAssertEqual(DocDisk.loadModels(deviceId: deviceId, harness: harness), models)
    }

    @MainActor
    func testRestartedSessionRetainsOutboxIdsAndRetiresThem() throws {
        let id = "session-outbox-\(UUID().uuidString)"
        let config = AppConfig(edgeURL: URL(string: "http://localhost:1")!, mode: .dev,
                               userId: "u", orgId: "o", deviceId: "phone",
                               deviceName: "phone", tokens: nil, devBearer: "u@o")
        defer { try? FileManager.default.removeItem(at: DocDisk.chat2URL(for: id)) }
        let doc = LoroDoc()
        let outbox: [(batchId: String, bytes: Data)] = [
            ("stable-a", Data([1])),
            ("stable-b", Data([2])),
        ]
        XCTAssertTrue(DocDisk.saveChat2(doc: doc, id: id, cursor: 7, verified: true,
                                        outbox: outbox))

        let store = SessionStore(chatId: id, config: config)
        store.start(holdDial: true)
        XCTAssertEqual(store.outbox.map(\.batchId), ["stable-a", "stable-b"])
        XCTAssertEqual(store.outbox.map(\.bytes), [Data([1]), Data([2])])
        store.retirePush(batchId: "stable-a")
        XCTAssertEqual(store.outbox.map(\.batchId), ["stable-b"])
    }

    @MainActor
    func testHeldSessionReleasesPendingOutboxToTransport() throws {
        let id = "held-outbox-\(UUID().uuidString)"
        let config = AppConfig(edgeURL: URL(string: "http://localhost:1")!, mode: .dev,
                               userId: "u", orgId: "o", deviceId: "phone",
                               deviceName: "phone", tokens: nil, devBearer: "u@o")
        defer { try? FileManager.default.removeItem(at: DocDisk.chat2URL(for: id)) }
        XCTAssertTrue(DocDisk.saveChat2(doc: LoroDoc(), id: id, cursor: 7, verified: true,
                                        outbox: [("stable-a", Data([1]))]))

        let store = SessionStore(chatId: id, config: config)
        store.start(holdDial: true)
        store.updateRoomGen(2)
        XCTAssertFalse(store.roomActive)
        XCTAssertTrue(store.admittedBatchIDs.isEmpty)

        store.releaseDial()
        XCTAssertTrue(store.roomActive)
        XCTAssertEqual(store.admittedBatchIDs, ["stable-a"])
        store.stop()
    }

    @MainActor
    func testCursorZeroFirstContactPreservesRestoredBatchesAndFlag() throws {
        let id = "first-contact-\(UUID().uuidString)"
        let config = AppConfig(edgeURL: URL(string: "http://localhost:1")!, mode: .dev,
                               userId: "u", orgId: "o", deviceId: "phone",
                               deviceName: "phone", tokens: nil, devBearer: "u@o")
        defer { try? FileManager.default.removeItem(at: DocDisk.chat2URL(for: id)) }
        let doc = LoroDoc()
        try doc.getMap(id: "test").insert(key: "value", v: "local")
        doc.commit()
        let restored: [(batchId: String, bytes: Data)] = [
            ("restored-a", Data([1])),
            ("restored-b", Data([2])),
        ]
        XCTAssertTrue(DocDisk.saveChat2(doc: doc, id: id, cursor: 0, verified: false,
                                        outbox: restored))

        let store = SessionStore(chatId: id, config: config)
        store.start()
        store.updateRoomGen(2)
        XCTAssertEqual(store.outbox.count, 3)
        XCTAssertTrue(store.outbox.map(\.batchId).contains("restored-a"))
        XCTAssertTrue(store.outbox.map(\.batchId).contains("restored-b"))
        let loaded = try XCTUnwrap(DocDisk.loadChat2(into: LoroDoc(), id: id))
        XCTAssertTrue(loaded.firstContactQueued)

        store.stop()
        let restarted = SessionStore(chatId: id, config: config)
        restarted.start()
        restarted.updateRoomGen(2)
        XCTAssertEqual(restarted.outbox.count, 3)
        restarted.stop()
    }

    // MARK: versionTriple (proto version_triple port)

    func testVersionTripleParsesPlainAndSuffixed() {
        XCTAssertTrue(versionTriple("0.2.12")! == (0, 2, 12))
        XCTAssertTrue(versionTriple("1.10.3-beta.1")! == (1, 10, 3))
        XCTAssertTrue(versionTriple("0.2.12+build7")! == (0, 2, 12))
    }

    func testVersionTripleRejectsJunk() {
        XCTAssertNil(versionTriple(""))
        XCTAssertNil(versionTriple("0.2"))
        XCTAssertNil(versionTriple("a.b.c"))
        XCTAssertNil(versionTriple("0.2.x"))
        XCTAssertNil(versionTriple("0.2.12rc1"), "junk directly after the patch digits is not a version")
    }

    func testVersionGateComparison() {
        let min = (0, 2, 12)
        XCTAssertTrue(versionTriple("0.2.12")! >= min)
        XCTAssertTrue(versionTriple("0.3.0")! >= min)
        XCTAssertTrue(versionTriple("1.0.0")! >= min)
        XCTAssertFalse(versionTriple("0.2.11")! >= min)
    }

    // MARK: pending:// refs (uploads.rs PENDING_REF_PREFIX)

    func testPendingRefRoundTrip() {
        let ref = UploadStash.pendingRef(uploadId: "abc-123", name: "photo one.png")
        XCTAssertEqual(ref, "pending://abc-123/photo one.png")
        let parsed = UploadStash.parseRef(ref)
        XCTAssertEqual(parsed?.uploadId, "abc-123")
        XCTAssertEqual(parsed?.name, "photo one.png")
    }

    func testParseRefRejectsForeignAndMalformedRefs() {
        XCTAssertNil(UploadStash.parseRef("/Users/x/uploads/abc-photo.png"))
        XCTAssertNil(UploadStash.parseRef("pending://no-slash"))
        XCTAssertNil(UploadStash.parseRef("pending:///name.png"))
        XCTAssertNil(UploadStash.parseRef("pending://id/"))
    }

    func testStashSaveLoadDelete() {
        let id = "test-\(UUID().uuidString.lowercased())"
        let bytes = Data([1, 2, 3, 4])
        UploadStash.save(uploadId: id, data: bytes)
        XCTAssertEqual(UploadStash.load(uploadId: id), bytes)
        UploadStash.delete(uploadId: id)
        XCTAssertNil(UploadStash.load(uploadId: id))
    }

    // MARK: upload chunk plan (attachments.rs PR #164 invariants)

    func testChunkSizeWireInvariants() {
        // % 4 == 0: every base64 slice decodes independently.
        XCTAssertEqual(uploadChunkB64Chars % 4, 0)
        // The binary slice boundary is % 3 == 0, so per-slice base64
        // concatenates to the whole file's encoding.
        XCTAssertEqual((uploadChunkB64Chars / 4 * 3) % 3, 0)
        // Envelope headroom under Cloudflare's 1 MiB WS message cap.
        XCTAssertLessThan(uploadChunkB64Chars + 1_024, 1_048_576)
    }

    func testAttachmentDeadlineFormula() {
        XCTAssertEqual(attachmentDeadlineSeconds(chunkCount: 1), 135)
        XCTAssertEqual(attachmentDeadlineSeconds(chunkCount: 10), 270)
        XCTAssertEqual(attachmentDeadlineSeconds(chunkCount: 1_000), 900, "capped at 15 minutes")
    }

    // MARK: relay echo frame (device_room.rs ECHO_KIND)

    func testEchoFrameRoundTrip() {
        let frame = DeviceRelayClient.encodeFrame(header: #"{"s":"echo","k":"echo"}"#,
                                                  payload: Data())
        let decoded = DeviceRelayClient.decodeFrame(frame)
        XCTAssertEqual(decoded?.0.k, DeviceRelayClient.echoKind)
        XCTAssertEqual(decoded?.1.count, 0)
    }

    func testPendingOwnsTracksLifecycle() {
        let pending = DeviceRpcPending()
        XCTAssertFalse(pending.owns(id: 1))
        pending.registerUnary(id: 1) { _ in }
        XCTAssertTrue(pending.owns(id: 1))
        pending.fail(id: 1, error: .timeout)
        XCTAssertFalse(pending.owns(id: 1), "a resolved call must not read as pending — the timeout task keys link teardown on it")
    }

    // MARK: RunRequest wire shape (proto agent.rs, PR #159)

    func testWorktreeSpecOmittedWhenNil() throws {
        let request = RunRequest(prompt: "hi", cwd: "/repo")
        let json = String(data: try JSONEncoder().encode(request), encoding: .utf8)!
        XCTAssertFalse(json.contains("worktree"), "old hosts must see the legacy shape")
    }

    // MARK: retry re-issue (PR #172 phone half)

    @MainActor
    func testRetryReissuesADeadSendAttemptExactlyOnce() throws {
        let config = AppConfig(edgeURL: URL(string: "http://localhost:1")!, mode: .dev,
                               userId: "u", orgId: "o", deviceId: "phone",
                               deviceName: "phone", tokens: nil, devBearer: "u@o")
        let store = SessionStore(chatId: "c-reissue", config: config)
        let commands = store.doc.getList(id: "commands")
        let dead = try commands.pushContainer(child: LoroMap())
        try dead.insert(key: "id", v: "cmd-old")
        try dead.insert(key: "kind", v: "run")
        try dead.insert(key: "payload", v: LoroValue.fromJSON([
            "kind": "run",
            "request": ["prompt": "hi", "cwd": "/repo"],
            "messageId": "msg-1",
        ]))
        try dead.insert(key: "issuedBy", v: "phone")
        try dead.insert(key: "issuedAt", v: Int64(1_000))
        try dead.insert(key: "expiresAt", v: nowMs() + 60_000)
        try dead.insert(key: "status", v: "rejected")
        store.doc.commit()

        store.reissueDeadSends()

        let after = try XCTUnwrap(store.doc.getDeepValue().mapValue?["commands"]?.listValue)
        XCTAssertEqual(after.count, 2, "a rejected attempt whose message never landed re-issues")
        let fresh = try XCTUnwrap(after[1].mapValue)
        XCTAssertEqual(fresh["status"]?.stringValue, "pending")
        XCTAssertNotEqual(fresh["id"]?.stringValue, "cmd-old", "exactly-once is per command id — retry mints a fresh one")
        XCTAssertEqual(fresh["payload"]?.mapValue?["messageId"]?.stringValue, "msg-1",
                       "the message id survives so the host's user-entry pre-write dedupes")

        // A live pending attempt for the message makes a re-issue a duplicate.
        store.reissueDeadSends()
        let again = try XCTUnwrap(store.doc.getDeepValue().mapValue?["commands"]?.listValue)
        XCTAssertEqual(again.count, 2, "a second retry with a live attempt queued must not duplicate")
    }

    // MARK: RunRequest wire shape (proto agent.rs, PR #159)

    func testWorktreeSpecEncodesCamelCase() throws {
        var request = RunRequest(prompt: "hi", cwd: "/repo")
        request.worktree = WorktreeSpec(repoPath: "/repo", base: "HEAD")
        let data = try JSONEncoder().encode(request)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let worktree = try XCTUnwrap(object["worktree"] as? [String: Any])
        XCTAssertEqual(worktree["repoPath"] as? String, "/repo")
        XCTAssertEqual(worktree["base"] as? String, "HEAD")
    }
}
