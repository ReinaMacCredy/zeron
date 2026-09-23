import SwiftUI

enum ComputerCardTone {
    case info, success, warning

    var accent: Color {
        switch self {
        case .info: return Theme.textMuted
        case .success: return Theme.statusCompleted
        case .warning: return Theme.warning
        }
    }

    var tile: Color {
        switch self {
        case .info: return whiteAlpha(0.09)
        case .success, .warning: return accent.opacity(0.12)
        }
    }
}

struct ComputerCardAction {
    var id: String
    var label: String
    var primary: Bool
    var action: () -> Void
}

struct ComputerCard: View {
    let tone: ComputerCardTone
    let symbol: String
    let title: String
    var lines: [String] = []
    var actions: [ComputerCardAction] = []
    var status: String?

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: symbol)
                .font(.system(size: 13, weight: .medium))
                .foregroundStyle(tone.accent)
                .frame(width: 28, height: 28)
                .background(tone.tile, in: RoundedRectangle(cornerRadius: 8))
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 3) {
                Text(title)
                    .font(Theme.sans(14, weight: .medium))
                    .foregroundStyle(Theme.text.opacity(0.9))
                    .fixedSize(horizontal: false, vertical: true)
                ForEach(Array(lines.enumerated()), id: \.offset) { _, line in
                    Text(line)
                        .font(Theme.sans(13))
                        .foregroundStyle(Theme.textMuted)
                        .fixedSize(horizontal: false, vertical: true)
                }
                if !actions.isEmpty || status != nil {
                    footer.padding(.top, 6)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(12)
        .background(whiteAlpha(0.045), in: RoundedRectangle(cornerRadius: 12))
        .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(whiteAlpha(0.08), lineWidth: 1))
        .accessibilityElement(children: .contain)
    }

    private var footer: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 8) { footerItems }
            VStack(alignment: .leading, spacing: 8) { footerItems }
        }
    }

    @ViewBuilder
    private var footerItems: some View {
        ForEach(actions, id: \.id) { action in
            Button(action: action.action) {
                Text(action.label)
                    .font(Theme.sans(13, weight: .medium))
                    .foregroundStyle(action.primary ? Theme.bg : Theme.text.opacity(0.9))
                    .padding(.horizontal, 14)
                    .frame(minHeight: 36)
                    .background(action.primary ? Theme.text : Color.clear, in: Capsule())
                    .overlay(Capsule().strokeBorder(action.primary ? Color.clear : whiteAlpha(0.14),
                                                    lineWidth: 1))
                    .contentShape(Capsule())
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier(action.id)
        }
        if let status {
            Text(status)
                .font(Theme.sans(13, weight: .medium))
                .foregroundStyle(tone.accent)
        }
    }
}

struct ComputerConsentCard: View {
    let store: SessionStore
    let requestId: String
    let questionId: String
    let scope: ComputerConsentScope
    let question: String
    let outcome: ConsentOutcome?

    var body: some View {
        let settled = outcome ?? store.consentSent[requestId].map { $0 ? .allowed : .declined }
        let detail = ComputerCopy.consentDetail(scope, question: question)
        var lines = detail.isEmpty ? [] : [detail]
        if settled == nil { lines.append(ComputerCopy.consentAllowDescription(scope)) }
        return ComputerCard(
            tone: settled == nil ? .warning : (settled == .allowed ? .success : .info),
            symbol: "desktopcomputer",
            title: ComputerCopy.consentTitle(scope),
            lines: lines,
            actions: settled == nil ? [
                ComputerCardAction(id: "computer-consent-allow", label: ComputerCopy.consentAllowLabel(scope),
                                   primary: true) { answer(true) },
                ComputerCardAction(id: "computer-consent-decline", label: ComputerCopy.consentDecline,
                                   primary: false) { answer(false) },
            ] : [],
            status: settled.map(ComputerCopy.outcomeLabel))
    }

    private func answer(_ allow: Bool) {
        store.answerConsent(requestId: requestId, questionId: questionId, allow: allow)
    }
}

struct ComputerNoticeCard: View {
    let notice: ComputerNotice
    let text: String
    let hint: String?
    var onEnable: (() -> Void)?

    var body: some View {
        switch notice {
        case .disclosure:
            ComputerCard(tone: .info, symbol: "cursorarrow", title: text)
        case .escapeStop:
            ComputerCard(tone: .warning, symbol: "stop.circle", title: text)
        case .setupRequired, .unavailable:
            ComputerCard(tone: .warning, symbol: "desktopcomputer", title: text,
                         lines: hint.map { [$0] } ?? [])
        case .controlDenied:
            ComputerCard(tone: .warning, symbol: "desktopcomputer", title: text,
                         lines: hint.map { [$0] } ?? [],
                         actions: onEnable.map {
                             [ComputerCardAction(id: "computer-denied-enable", label: ComputerCopy.deniedEnable,
                                                 primary: false, action: $0)]
                         } ?? [])
        }
    }
}
