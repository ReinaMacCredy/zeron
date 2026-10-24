import UIKit

/// Composer-facing state of a session (the transcript itself flows Rust→Rust
/// into the layout engine and never crosses FFI as rows).
struct SessionChrome: Equatable {
    enum Banner: Equatable {
        case none
        case working(since: Date?, word: String)
        case sending
        case offline
        case reconnecting(in: Int)
        case notDelivered
        case failed(String)
        case editing
        case limit(String)
        case notice(String)
        case offer(title: String, body: String)
    }

    struct Question: Equatable {
        let id: String
        let header: String
        let text: String
        let options: [String]
        let multiSelect: Bool
        var prefill: String? = nil
        var multiline: Bool = false
    }

    /// Attachment upload progress (0…1) while images are escorted to the
    /// host — drawn as a ring on the pending thumbnails, not a banner.
    var uploadProgress: Double?

    struct QueuedItem: Equatable {
        let id: String
        let text: String
        let thumbnail: UIImage?
        let gate: String?
        var from: String? = nil
        /// What the row's arrow does; nil while it cannot deliver.
        var action: QueuePrimaryAction? = nil
        /// An image attachment's reference, loaded as the row's thumbnail.
        var image: String? = nil
    }

    var title = ""
    var subtitle = ""
    var hostName: String?
    var running = false
    var resumable = false
    var canSteer = false
    var placeholder = "Message"
    var chips: [ComposerChip] = []
    var banner: Banner = .none
    var questions: (requestId: String, items: [Question])?
    var queue: [QueuedItem] = []
    var error: String?
    var limitPill: LimitPill?
    var prediction: String?

    struct Comments: Equatable {
        let title: String
        let clear: String
        let items: [StagedCommentChip]
    }

    var comments: Comments?

    struct Agents: Equatable {
        let summary: String
        let state: AgentRunState
    }

    var agents: Agents?

    static func == (a: SessionChrome, b: SessionChrome) -> Bool {
        a.title == b.title && a.subtitle == b.subtitle && a.hostName == b.hostName && a.running == b.running && a.canSteer == b.canSteer
            && a.placeholder == b.placeholder && a.chips == b.chips && a.banner == b.banner
            && a.questions?.requestId == b.questions?.requestId && a.questions?.items == b.questions?.items
            && a.queue == b.queue && a.error == b.error && a.limitPill == b.limitPill && a.prediction == b.prediction
            && a.comments == b.comments && a.agents == b.agents
    }
}

/// A session's data + commands, independent of where it comes from.
protocol SessionSource: AnyObject {
    var chrome: SessionChrome { get }
    var onChange: (() -> Void)? { get set }
    var onPresent: ((UIViewController) -> Void)? { get set }
    /// Bind the Rust layout engine to this session's transcript.
    func attach(_ engine: TranscriptView)
    func detach()
    /// The session's screen is showing again (another copy may have detached).
    func reattach()
    /// False when the message wasn't taken (it stays in the composer).
    @discardableResult
    func send(text: String, images: [StagedImage], mode: DeliveryMode) -> Bool
    func stop()
    func answer(requestId: String, answers: [(questionId: String, labels: [String])])
    func answerConsent(requestId: String, questionId: String, answer: String)
    func computerStill() async -> (sequence: UInt64, image: UIImage)?
    func computerStop() async throws -> Bool
    func terminals() async throws -> [TerminalTab]
    func terminal(_ id: String, dark: Bool) async throws -> TerminalView
    func projectActions() async throws -> ProjectActions?
    func runProjectAction(_ id: String) async throws -> String
    func queueAction(_ id: String, _ action: QueueAction)
    /// The queue error was shown; a repeat of it shows again.
    func clearQueueError()
    func retryDelivery()
    /// Lease a queued message for editing; returns its text when acquired.
    func beginEdit(_ id: String) async -> String?
    /// Commit (text) or cancel (nil) the active edit.
    func finishEdit(text: String?) async
    /// Menu for a composer chip (model, effort, branch…), or nil.
    func chipMenu(_ id: String) -> UIMenu?
    /// Where this chat's provider signs in (an error card's "Log in").
    func signInTarget() -> AgentAccountsViewController.Target?
    func chipSheet(_ id: String) -> UIViewController?
    func loadImage(_ reference: String, into view: UIImageView)
    /// An attachment's picture, for opening it full size (an image chip).
    func image(_ reference: String) async -> UIImage?
    func attachmentText(_ reference: String) async -> String?
    /// Workspace files for `@` mentions.
    func searchFiles(_ query: String) async -> [FileMatch]
    func branchBlocked(rewind: Bool) async -> String?
    func fork(from entryId: String) async throws -> String
    func rewind(to entryId: String) async throws -> String
    func skillFile(name: String, path: String?) async throws -> SkillFile
    func chatUsage() async -> UsageMeter?
    func limitCard(chatId: String) -> LimitCard?
    func resolveLimit(chatId: String, choice: String) async throws
    func setLimitResume(chatId: String, auto: Bool) async throws
    func htmlPage(deviceId: String, path: String) async -> String?
    func threadMentions(_ query: String) -> [ThreadMention]
    func invocationCatalog(skill: Bool) async -> InvocationCatalog?
    var compactBlocked: String? { get }
    func compact(focus: String?)
    func keepFullHistory()
    func handoffMenu(open: @escaping (String) -> Void) -> UIMenuElement?
    func agentsAction(open: @escaping (String) -> Void) -> UIAction?
    func agentsCard(open: @escaping (String) -> Void) -> AgentsCard?
    func reviewAction() -> UIAction?
    func chatChanges() async -> ChatChangesView?
    func openChanges(from host: UIViewController, title: String)
    func removeStagedComment(_ id: String)
    func clearStagedComments()
}

extension SessionSource {
    func clearQueueError() {}
    func chipSheet(_ id: String) -> UIViewController? { nil }
    func invocationCatalog(skill: Bool) async -> InvocationCatalog? { nil }
    var compactBlocked: String? { nil }
    func compact(focus: String?) {}
    func keepFullHistory() {}
    func handoffMenu(open: @escaping (String) -> Void) -> UIMenuElement? { nil }
    func agentsAction(open: @escaping (String) -> Void) -> UIAction? { nil }
    func agentsCard(open: @escaping (String) -> Void) -> AgentsCard? { nil }
    func reviewAction() -> UIAction? { nil }
    func chatChanges() async -> ChatChangesView? { nil }
    func openChanges(from host: UIViewController, title: String) {}
    func removeStagedComment(_ id: String) {}
    func clearStagedComments() {}
}

enum QueueAction {
    case sendNow
    case edit
    case moveUp
    case moveDown
    case remove
}

/// Scripted source over fixture markdown (lab + UI tests until the core's
/// demo session lands).
final class FixtureSessionSource: SessionSource {
    private(set) var chrome = SessionChrome()
    var onChange: (() -> Void)?
    var onPresent: ((UIViewController) -> Void)?
    private weak var engine: TranscriptView?
    private var entries: [DebugEntry] = []
    private var timer: Timer?
    private let fixture = layoutFixtureMarkdown()

    init(title: String, subtitle: String) {
        chrome.title = title
        chrome.subtitle = subtitle
        chrome.placeholder = "Message Claude"
        chrome.chips = [
            ComposerChip(id: "model", title: "Opus 4.5", symbol: nil, icon: BrandMarks.image(for: "claude-code", side: 13)),
            ComposerChip(id: "effort", title: "High", symbol: "gauge.with.dots.needle.67percent"),
            ComposerChip(id: "branch", title: "ios-rewrite", symbol: nil, icon: BranchIcon.sized()),
        ]
        for i in 0..<3 {
            entries.append(DebugEntry(id: "u\(i)", user: true, text: TranscriptLabViewController.prompts[i], streaming: false))
            entries.append(DebugEntry(id: "a\(i)", user: false, text: fixture, streaming: false))
        }
    }

    func attach(_ engine: TranscriptView) {
        self.engine = engine
        engine.setDebugEntries(entries: entries, working: chrome.running)
    }

    func detach() {
        timer?.invalidate()
    }

    func reattach() {}

    private func update(_ change: (inout SessionChrome) -> Void) {
        let old = chrome
        change(&chrome)
        if chrome != old { onChange?() }
    }

    func send(text: String, images: [StagedImage], mode: DeliveryMode) -> Bool {
        if chrome.running, mode == .queue {
            update { $0.queue.append(.init(id: UUID().uuidString, text: text, thumbnail: images.first?.thumbnail, gate: nil, action: images.isEmpty ? .steer : .sendNow)) }
            return true
        }
        timer?.invalidate()
        let n = entries.count
        entries.append(DebugEntry(id: "u\(n)", user: true, text: text, streaming: false))
        entries.append(DebugEntry(id: "a\(n)", user: false, text: "", streaming: true))
        update {
            $0.running = true
            $0.banner = .working(since: Date(), word: "Thinking")
        }
        let words = fixture.split(separator: " ", omittingEmptySubsequences: false).map(String.init)
        var i = 0
        engine?.setDebugEntries(entries: entries, working: true)
        timer = Timer.scheduledTimer(withTimeInterval: 0.05, repeats: true) { [weak self] t in
            guard let self else { return t.invalidate() }
            let step = Int.random(in: 2...4)
            let next = words[i..<min(words.count, i + step)].joined(separator: " ")
            i += step
            var last = self.entries[self.entries.count - 1]
            last.text += (last.text.isEmpty ? "" : " ") + next
            last.streaming = i < words.count
            self.entries[self.entries.count - 1] = last
            self.engine?.setDebugEntries(entries: self.entries, working: last.streaming)
            if !last.streaming { self.finish() }
        }
        return true
    }

    private func finish() {
        timer?.invalidate()
        update {
            $0.running = false
            $0.banner = .none
        }
        if let next = chrome.queue.first {
            update { $0.queue.removeFirst() }
            send(text: next.text, images: [], mode: .queue)
        }
    }

    func stop() {
        guard var last = entries.last, last.streaming else { return }
        last.streaming = false
        entries[entries.count - 1] = last
        engine?.setDebugEntries(entries: entries, working: false)
        timer?.invalidate()
        update {
            $0.running = false
            $0.banner = .none
        }
    }

    func answer(requestId: String, answers: [(questionId: String, labels: [String])]) {
        update { $0.questions = nil }
    }

    func answerConsent(requestId: String, questionId: String, answer: String) {}
    func computerStill() async -> (sequence: UInt64, image: UIImage)? { nil }
    func computerStop() async throws -> Bool { false }
    func terminals() async throws -> [TerminalTab] { [] }
    func terminal(_ id: String, dark: Bool) async throws -> TerminalView { throw CancellationError() }
    func projectActions() async throws -> ProjectActions? { nil }
    func runProjectAction(_ id: String) async throws -> String { throw CancellationError() }

    func queueAction(_ id: String, _ action: QueueAction) {
        update { chrome in
            guard let i = chrome.queue.firstIndex(where: { $0.id == id }) else { return }
            switch action {
            case .remove: chrome.queue.remove(at: i)
            case .moveUp where i > 0: chrome.queue.swapAt(i, i - 1)
            case .moveDown where i + 1 < chrome.queue.count: chrome.queue.swapAt(i, i + 1)
            default: break
            }
        }
    }

    func retryDelivery() {}
    func beginEdit(_ id: String) async -> String? { chrome.queue.first { $0.id == id }?.text }
    func finishEdit(text: String?) async {}
    func chipMenu(_ id: String) -> UIMenu? {
        switch id {
        case "model":
            return UIMenu(title: "Model", children: ["Opus 4.5", "Sonnet 4.5", "Haiku 4.5"].map { m in
                UIAction(title: m, state: chrome.chips.first { $0.id == "model" }?.title == m ? .on : .off) { [weak self] _ in
                    self?.update { c in
                        if let i = c.chips.firstIndex(where: { $0.id == "model" }) { c.chips[i] = ComposerChip(id: "model", title: m, symbol: nil, icon: BrandMarks.image(for: "claude-code", side: 13)) }
                    }
                }
            })
        case "effort":
            return UIMenu(title: "Reasoning effort", children: ["Low", "Medium", "High", "Max"].map { e in
                UIAction(title: e, state: chrome.chips.first { $0.id == "effort" }?.title == e ? .on : .off) { [weak self] _ in
                    self?.update { c in
                        if let i = c.chips.firstIndex(where: { $0.id == "effort" }) { c.chips[i] = ComposerChip(id: "effort", title: e, symbol: "gauge.with.dots.needle.67percent") }
                    }
                }
            })
        default:
            return nil
        }
    }
    func loadImage(_ reference: String, into view: UIImageView) {}
    func image(_ reference: String) async -> UIImage? { nil }
    func attachmentText(_ reference: String) async -> String? { nil }
    func searchFiles(_ query: String) async -> [FileMatch] { [] }
    func branchBlocked(rewind: Bool) async -> String? { "Not available in the lab" }
    func fork(from entryId: String) async throws -> String { throw CancellationError() }
    func rewind(to entryId: String) async throws -> String { throw CancellationError() }
    func skillFile(name: String, path: String?) async throws -> SkillFile { throw CancellationError() }
    func chatUsage() async -> UsageMeter? { nil }
    func limitCard(chatId: String) -> LimitCard? { nil }
    func resolveLimit(chatId: String, choice: String) async throws { throw CancellationError() }
    func setLimitResume(chatId: String, auto: Bool) async throws { throw CancellationError() }
    func htmlPage(deviceId: String, path: String) async -> String? { nil }
    func threadMentions(_ query: String) -> [ThreadMention] { [] }
}

extension SessionSource {
    func signInTarget() -> AgentAccountsViewController.Target? { nil }
}
