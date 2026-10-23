import UIKit

extension ReviewSeverity {
    var ink: UIColor {
        switch self {
        case .bug: Palette.danger
        case .risk: Palette.danger.withAlphaComponent(0.6)
        case .nit: Palette.secondary
        }
    }

    var wash: UIColor {
        switch self {
        case .bug: Palette.danger.withAlphaComponent(0.14)
        case .risk: Palette.danger.withAlphaComponent(0.08)
        case .nit: Palette.controlFill
        }
    }
}

final class FindingCardCell: UITableViewCell {
    enum Action { case fix, dismiss }
    typealias Act = (DiffFindingCard, Action) -> Void

    static let id = "diff-finding"
    private let bar = UIView()
    private let header = UIStackView()
    private let dot = UIView()
    private let word = UILabel()
    private let title = UILabel()
    private let detail = UILabel()
    private let fix = UIButton(type: .system)
    private let dismissButton = UIButton(type: .system)
    private var card: DiffFindingCard?
    private var act: Act?

    override init(style: UITableViewCell.CellStyle, reuseIdentifier: String?) {
        super.init(style: style, reuseIdentifier: reuseIdentifier)
        selectionStyle = .none
        backgroundColor = .clear
        contentView.backgroundColor = Palette.controlFill
        dot.layer.cornerRadius = 3
        dot.translatesAutoresizingMaskIntoConstraints = false
        dot.widthAnchor.constraint(equalToConstant: 6).isActive = true
        dot.heightAnchor.constraint(equalToConstant: 6).isActive = true
        word.font = Fonts.ui(.sansSemibold, 12)
        title.font = Fonts.ui(.sansMedium, 13)
        title.textColor = Palette.text
        title.numberOfLines = 0
        detail.font = Fonts.ui(.sans, 13)
        detail.textColor = Palette.secondary
        detail.numberOfLines = 0
        header.axis = .horizontal
        header.spacing = 6
        header.alignment = .center
        header.addArrangedSubview(dot)
        header.addArrangedSubview(word)
        header.addArrangedSubview(title)
        var primary = UIButton.Configuration.gray()
        primary.buttonSize = .small
        primary.cornerStyle = .medium
        fix.configuration = primary
        var quiet = UIButton.Configuration.plain()
        quiet.buttonSize = .small
        quiet.baseForegroundColor = Palette.secondary
        dismissButton.configuration = quiet
        fix.addAction(UIAction { [weak self] _ in self?.fire(.fix) }, for: .touchUpInside)
        dismissButton.addAction(UIAction { [weak self] _ in self?.fire(.dismiss) }, for: .touchUpInside)
        let actions = UIStackView(arrangedSubviews: [fix, dismissButton, UIView()])
        actions.spacing = 6
        let stack = UIStackView(arrangedSubviews: [header, detail, actions])
        stack.axis = .vertical
        stack.spacing = 6
        stack.alignment = .fill
        for v in [bar, stack] {
            v.translatesAutoresizingMaskIntoConstraints = false
            contentView.addSubview(v)
        }
        NSLayoutConstraint.activate([
            bar.leadingAnchor.constraint(equalTo: contentView.leadingAnchor),
            bar.topAnchor.constraint(equalTo: contentView.topAnchor),
            bar.bottomAnchor.constraint(equalTo: contentView.bottomAnchor),
            bar.widthAnchor.constraint(equalToConstant: 2),
            stack.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 16),
            stack.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -12),
            stack.topAnchor.constraint(equalTo: contentView.topAnchor, constant: 10),
            stack.bottomAnchor.constraint(equalTo: contentView.bottomAnchor, constant: -8),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }

    func show(_ card: DiffFindingCard, act: Act?) {
        self.card = card
        self.act = act
        bar.backgroundColor = card.severity.ink
        dot.backgroundColor = card.severity.ink
        word.text = card.severityWord
        word.textColor = card.severity.ink
        title.text = card.title
        detail.text = card.detail
        detail.isHidden = card.detail.isEmpty
        fix.configuration?.title = card.fixLabel
        dismissButton.configuration?.title = card.dismissLabel
        fix.accessibilityIdentifier = "finding-fix-\(card.id)"
        dismissButton.accessibilityIdentifier = "finding-dismiss-\(card.id)"
        fix.superview?.isHidden = act == nil
        accessibilityIdentifier = "diff-finding-\(card.id)"
    }

    private func fire(_ action: Action) {
        guard let card else { return }
        act?(card, action)
    }
}

extension DiffSheetViewController {
    static func presentFindings(_ handle: SessionHandle, from host: UIViewController, title: String, subtitle: String?, focus: String?) {
        present(from: host, title: title, subtitle: subtitle, focus: focus, finding: { card, action in
            switch action {
            case .fix: _ = handle.fixReviewFindings(only: card.id)
            case .dismiss: handle.dismissReviewFinding(id: card.id)
            }
        }) { try await handle.reviewChanges() }
    }
}
