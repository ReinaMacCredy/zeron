import ActivityKit
import AppIntents
import SwiftUI
import UIKit

struct SessionActivityAttributes: ActivityAttributes, Hashable {
    struct ContentState: Codable, Hashable {
        var rows: [SessionActivityRow]
        var running: Int
        var asking: Int
    }

    var chatId: String?
    var trailing: SessionActivityTrailing
    var showStep: Bool
    var lockHide: Bool
}

struct SessionActivityRow: Codable, Hashable, Identifiable {
    var id: String
    var title: String
    var project: String
    var branch: String?
    var state: SessionActivityState
    var since: Double?
    var step: String?
}

enum SessionActivityState: String, Codable, Hashable {
    case working, asking, done, failed
}

enum SessionActivityTrailing: String, Codable, Hashable, CaseIterable {
    case count, clock, state
}

enum SessionActivityAction {
    @MainActor static var stop: ((String) -> Void)?
}

struct StopSessionIntent: LiveActivityIntent {
    static let title: LocalizedStringResource = "Stop Session"
    static var isDiscoverable: Bool { false }

    @Parameter(title: "Session")
    var chatId: String

    init() {}

    init(chatId: String) {
        self.chatId = chatId
    }

    func perform() async throws -> some IntentResult {
        let id = chatId
        await MainActor.run { SessionActivityAction.stop?(id) }
        return .result()
    }
}

enum SessionLink {
    static let sessions = URL(string: "zeron://sessions")!

    static func url(_ chatId: String?) -> URL {
        guard let chatId, let id = chatId.addingPercentEncoding(withAllowedCharacters: .alphanumerics.union(CharacterSet(charactersIn: "-_"))) else { return sessions }
        return URL(string: "zeron://session/\(id)") ?? sessions
    }
}

enum SessionTone {
    static let accent = dynamic(0x5B43E8, 0x8B7CF6)
    static let ask = dynamic(0xC2478F, 0xF08BC2)
    static let success = dynamic(0x15803D, 0x34D399)
    static let danger = dynamic(0xDC2626, 0xF87171)

    static func of(_ state: SessionActivityState) -> Color {
        switch state {
        case .working: accent
        case .asking: ask
        case .done: success
        case .failed: danger
        }
    }

    private static func dynamic(_ light: UInt32, _ dark: UInt32) -> Color {
        Color(UIColor { $0.userInterfaceStyle == .dark ? rgb(dark) : rgb(light) })
    }

    private static func rgb(_ hex: UInt32) -> UIColor {
        UIColor(red: CGFloat(hex >> 16 & 0xFF) / 255, green: CGFloat(hex >> 8 & 0xFF) / 255, blue: CGFloat(hex & 0xFF) / 255, alpha: 1)
    }
}

enum SessionFace {
    static func sans(_ size: CGFloat) -> Font { .custom("Geist-Medium", size: size) }
    static func mono(_ size: CGFloat) -> Font { .custom("GeistMono-Medium", size: size) }
}

extension SessionActivityAttributes.ContentState {
    var lead: SessionActivityRow? { rows.first }

    var summary: String {
        var parts: [String] = []
        if running > 0 || asking == 0 { parts.append("\(running) running") }
        if asking > 0 { parts.append("\(asking) need\(asking == 1 ? "s" : "") you") }
        return parts.joined(separator: " · ")
    }
}

extension SessionActivityState {
    var symbol: String {
        switch self {
        case .working: "sparkle"
        case .asking: "questionmark"
        case .done: "checkmark"
        case .failed: "exclamationmark"
        }
    }

    var word: String {
        switch self {
        case .working: "Working"
        case .asking: "Needs you"
        case .done: "Done"
        case .failed: "Failed"
        }
    }
}

struct SessionGlyph: View {
    let state: SessionActivityState
    var size: CGFloat = 15

    var body: some View {
        Image(systemName: state.symbol)
            .font(.system(size: size, weight: .bold))
            .foregroundStyle(SessionTone.of(state))
            .symbolEffect(.rotate, options: .repeat(.continuous), isActive: state == .working)
            .frame(width: size + 8, height: size + 8)
            .accessibilityLabel(state.word)
    }
}

struct SessionClock: View {
    let since: Double?

    var body: some View {
        if let since {
            Text(timerInterval: Date(timeIntervalSince1970: since)...Date.distantFuture, countsDown: false)
                .monospacedDigit()
                .multilineTextAlignment(.trailing)
        } else {
            Text("")
        }
    }
}

struct SessionRowTrailing: View {
    let row: SessionActivityRow

    var body: some View {
        switch row.state {
        case .working:
            SessionClock(since: row.since)
                .font(SessionFace.mono(13))
                .foregroundStyle(.secondary)
        default:
            Text(row.state.word)
                .font(SessionFace.sans(13))
                .foregroundStyle(SessionTone.of(row.state))
        }
    }
}

struct SessionCompactLeading: View {
    let state: SessionActivityAttributes.ContentState

    var body: some View {
        SessionGlyph(state: state.asking > 0 ? .asking : state.lead?.state ?? .working, size: 13)
    }
}

struct SessionCompactTrailing: View {
    let state: SessionActivityAttributes.ContentState
    let attributes: SessionActivityAttributes

    var body: some View {
        if state.asking > 0 {
            if attributes.chatId == nil, attributes.trailing == .count {
                HStack(spacing: 4) {
                    Circle().fill(SessionTone.ask).frame(width: 6, height: 6)
                    Text("\(state.asking)")
                }
                .font(SessionFace.mono(14))
                .foregroundStyle(SessionTone.ask)
            } else {
                Text("?")
                    .font(SessionFace.mono(15))
                    .foregroundStyle(SessionTone.ask)
            }
        } else {
            switch attributes.trailing {
            case .count:
                Text("\(attributes.chatId == nil ? state.running : 1)")
                    .font(SessionFace.mono(14))
            case .clock:
                SessionClock(since: state.lead?.since)
                    .font(SessionFace.mono(13))
                    .frame(width: 44, alignment: .trailing)
            case .state:
                Text(state.lead?.state.word ?? "")
                    .font(SessionFace.sans(12))
                    .foregroundStyle(SessionTone.of(state.lead?.state ?? .working))
                    .lineLimit(1)
                    .frame(maxWidth: 52, alignment: .trailing)
            }
        }
    }
}

struct SessionMinimal: View {
    let state: SessionActivityAttributes.ContentState

    var body: some View {
        if state.asking > 0 {
            Text("?")
                .font(SessionFace.mono(15))
                .foregroundStyle(SessionTone.ask)
        } else {
            SessionGlyph(state: state.lead?.state ?? .working, size: 12)
        }
    }
}

struct SessionExpandedLeading: View {
    let state: SessionActivityAttributes.ContentState
    let attributes: SessionActivityAttributes

    var body: some View {
        HStack(spacing: 6) {
            SessionGlyph(state: state.asking > 0 ? .asking : state.lead?.state ?? .working, size: 14)
            if attributes.chatId == nil {
                Text("Zeron")
                    .font(SessionFace.sans(14))
            }
        }
    }
}

struct SessionExpandedTrailing: View {
    let state: SessionActivityAttributes.ContentState
    let attributes: SessionActivityAttributes

    var body: some View {
        if attributes.chatId == nil {
            Text(state.summary)
                .font(SessionFace.sans(12))
                .foregroundStyle(state.asking > 0 ? AnyShapeStyle(SessionTone.ask) : AnyShapeStyle(.secondary))
                .lineLimit(1)
        } else if let row = state.lead {
            SessionRowTrailing(row: row)
        }
    }
}

struct SessionExpandedBody: View {
    let state: SessionActivityAttributes.ContentState
    let attributes: SessionActivityAttributes
    var hidesTitles = false
    var buttons = true

    var body: some View {
        if attributes.chatId == nil {
            VStack(alignment: .leading, spacing: 7) {
                ForEach(state.rows.prefix(3)) { row in
                    HStack(spacing: 8) {
                        SessionGlyph(state: row.state, size: 12)
                        line(row)
                            .lineLimit(1)
                        Spacer(minLength: 6)
                        SessionRowTrailing(row: row)
                    }
                }
                if state.rows.isEmpty {
                    Text("No sessions running")
                        .font(SessionFace.sans(13))
                        .foregroundStyle(.secondary)
                }
            }
        } else if let row = state.lead {
            VStack(alignment: .leading, spacing: 4) {
                Text(hidesTitles ? place(row) : row.title)
                    .font(SessionFace.sans(16))
                    .lineLimit(1)
                if !hidesTitles {
                    Text(place(row))
                        .font(SessionFace.mono(12))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                if attributes.showStep, !hidesTitles, let step = row.step, !step.isEmpty {
                    Text(step)
                        .font(SessionFace.sans(13))
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                        .padding(.top, 2)
                }
                if buttons {
                    HStack(spacing: 10) {
                        Link(destination: SessionLink.url(row.id)) {
                            SessionButtonFace(symbol: "arrow.up.forward.app", label: "Open")
                                .foregroundStyle(.primary)
                                .background(Color.primary.opacity(0.14), in: Capsule())
                        }
                        if row.state == .working || row.state == .asking {
                            Button(intent: StopSessionIntent(chatId: row.id)) {
                                SessionButtonFace(symbol: "stop.fill", label: "Stop")
                                    .foregroundStyle(.white)
                                    .background(SessionTone.danger, in: Capsule())
                            }
                            .buttonStyle(.plain)
                        }
                    }
                    .padding(.top, 6)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func line(_ row: SessionActivityRow) -> Text {
        if hidesTitles {
            return Text(place(row)).font(SessionFace.sans(14))
        }
        guard attributes.showStep, let step = row.step, !step.isEmpty else {
            return Text(row.title).font(SessionFace.sans(14))
        }
        return Text("\(Text(row.title))\(Text(" · \(step)").foregroundColor(.secondary))").font(SessionFace.sans(14))
    }

    private func place(_ row: SessionActivityRow) -> String {
        [row.project, row.branch].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · ")
    }
}

struct SessionButtonFace: View {
    let symbol: String
    let label: String

    var body: some View {
        Label(label, systemImage: symbol)
            .font(SessionFace.sans(14))
            .labelStyle(.titleAndIcon)
            .frame(maxWidth: .infinity)
            .frame(height: 38)
    }
}

struct SessionLockScreen: View {
    let state: SessionActivityAttributes.ContentState
    let attributes: SessionActivityAttributes

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                SessionExpandedLeading(state: state, attributes: attributes)
                Spacer(minLength: 8)
                SessionExpandedTrailing(state: state, attributes: attributes)
            }
            SessionExpandedBody(state: state, attributes: attributes, hidesTitles: attributes.lockHide)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 14)
    }
}
