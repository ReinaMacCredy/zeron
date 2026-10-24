import UIKit

final class RunGlyph: UIView {
    var state: AgentRunState = .working { didSet { if state != oldValue { update() } } }
    private let grid = StatusGlyph(.trailer)
    private let mark = UIImageView()

    override init(frame: CGRect) {
        super.init(frame: frame)
        isUserInteractionEnabled = false
        mark.contentMode = .center
        for v in [grid, mark] {
            v.translatesAutoresizingMaskIntoConstraints = false
            addSubview(v)
            NSLayoutConstraint.activate([
                v.centerXAnchor.constraint(equalTo: centerXAnchor),
                v.centerYAnchor.constraint(equalTo: centerYAnchor),
                v.widthAnchor.constraint(equalToConstant: 12),
                v.heightAnchor.constraint(equalToConstant: 12),
            ])
        }
        update()
    }

    required init?(coder: NSCoder) { fatalError() }

    override var intrinsicContentSize: CGSize { CGSize(width: 12, height: 12) }

    private func update() {
        let symbol: (String, UIColor)? = switch state {
        case .working: nil
        case .completed: ("checkmark", StatusTone.done)
        case .failed: ("xmark", Palette.danger)
        case .stopped: ("stop.fill", Palette.secondary)
        }
        grid.isHidden = symbol != nil
        grid.kind = symbol == nil ? .trailer : .none
        mark.image = symbol.flatMap { UIImage(systemName: $0.0, withConfiguration: UIImage.SymbolConfiguration(pointSize: 10, weight: .bold)) }
        mark.tintColor = symbol?.1
    }
}

final class AgentsPill: UIControl {
    private let glass = Glass.surface()
    private let glyph = RunGlyph()
    private let label = UILabel()

    override init(frame: CGRect) {
        super.init(frame: frame)
        glass.isUserInteractionEnabled = false
        glass.translatesAutoresizingMaskIntoConstraints = false
        addSubview(glass)
        label.font = Fonts.ui(.sansMedium, 13)
        label.textColor = Palette.secondary
        label.lineBreakMode = .byTruncatingTail
        label.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        for v in [glyph, label] as [UIView] {
            v.translatesAutoresizingMaskIntoConstraints = false
            glass.contentView.addSubview(v)
        }
        NSLayoutConstraint.activate([
            glass.topAnchor.constraint(equalTo: topAnchor),
            glass.bottomAnchor.constraint(equalTo: bottomAnchor),
            glass.leadingAnchor.constraint(equalTo: leadingAnchor),
            glass.trailingAnchor.constraint(equalTo: trailingAnchor),
            heightAnchor.constraint(equalToConstant: 30),
            glyph.leadingAnchor.constraint(equalTo: glass.contentView.leadingAnchor, constant: 11),
            glyph.centerYAnchor.constraint(equalTo: glass.contentView.centerYAnchor),
            label.leadingAnchor.constraint(equalTo: glyph.trailingAnchor, constant: 8),
            label.trailingAnchor.constraint(equalTo: glass.contentView.trailingAnchor, constant: -12),
            label.centerYAnchor.constraint(equalTo: glass.contentView.centerYAnchor),
        ])
        isAccessibilityElement = true
        accessibilityTraits = .button
        accessibilityIdentifier = "agents-pill"
    }

    required init?(coder: NSCoder) { fatalError() }

    func configure(_ agents: SessionChrome.Agents) {
        glyph.state = agents.state
        label.text = agents.summary
        accessibilityLabel = agents.summary
    }

    override var isSelected: Bool {
        didSet {
            guard isSelected != oldValue else { return }
            glass.effect = Glass.effect(tint: isSelected ? Palette.text.withAlphaComponent(0.1) : nil)
            label.textColor = isSelected ? Palette.text : Palette.secondary
            accessibilityValue = isSelected ? "Open" : nil
        }
    }

    override var isHighlighted: Bool {
        didSet { UIView.animate(withDuration: 0.15) { self.alpha = self.isHighlighted ? 0.7 : 1 } }
    }
}

final class AgentsCard: UIView {
    var onClose: (() -> Void)?
    private let handle: SessionHandle
    private let open: (String) -> Void
    private let findings: (String) -> UIViewController?
    private let present: (UIViewController) -> Void
    private let glass = Glass.surface(radius: 18)
    private let heading = UILabel()
    private let scroll = UIScrollView()
    private let rows = UIStackView()
    private var shape: [String] = []
    private var timer: Timer?

    init(handle: SessionHandle, open: @escaping (String) -> Void, findings: @escaping (String) -> UIViewController?, present: @escaping (UIViewController) -> Void) {
        self.handle = handle
        self.open = open
        self.findings = findings
        self.present = present
        super.init(frame: .zero)
        accessibilityIdentifier = "agents-card"
        glass.translatesAutoresizingMaskIntoConstraints = false
        addSubview(glass)
        heading.font = Fonts.ui(.sansMedium, 13)
        heading.textColor = Palette.text
        heading.lineBreakMode = .byTruncatingTail
        heading.accessibilityIdentifier = "agents-card-heading"
        var close = UIButton.Configuration.plain()
        close.image = UIImage(systemName: "xmark", withConfiguration: UIImage.SymbolConfiguration(pointSize: 12, weight: .semibold))
        close.baseForegroundColor = Palette.secondary
        let closeButton = UIButton(configuration: close, primaryAction: UIAction { [weak self] _ in self?.onClose?() })
        closeButton.accessibilityIdentifier = "agents-card-close"
        closeButton.accessibilityLabel = "Close"
        rows.axis = .vertical
        scroll.alwaysBounceVertical = false
        for v in [heading, closeButton, scroll] as [UIView] {
            v.translatesAutoresizingMaskIntoConstraints = false
            glass.contentView.addSubview(v)
        }
        rows.translatesAutoresizingMaskIntoConstraints = false
        scroll.addSubview(rows)
        let fit = scroll.heightAnchor.constraint(equalTo: rows.heightAnchor)
        fit.priority = .defaultHigh
        NSLayoutConstraint.activate([
            glass.topAnchor.constraint(equalTo: topAnchor),
            glass.bottomAnchor.constraint(equalTo: bottomAnchor),
            glass.leadingAnchor.constraint(equalTo: leadingAnchor),
            glass.trailingAnchor.constraint(equalTo: trailingAnchor),
            heading.leadingAnchor.constraint(equalTo: glass.contentView.leadingAnchor, constant: 16),
            heading.centerYAnchor.constraint(equalTo: closeButton.centerYAnchor),
            heading.trailingAnchor.constraint(lessThanOrEqualTo: closeButton.leadingAnchor, constant: -8),
            closeButton.topAnchor.constraint(equalTo: glass.contentView.topAnchor, constant: 4),
            closeButton.trailingAnchor.constraint(equalTo: glass.contentView.trailingAnchor, constant: -4),
            closeButton.widthAnchor.constraint(equalToConstant: 40),
            closeButton.heightAnchor.constraint(equalToConstant: 40),
            scroll.topAnchor.constraint(equalTo: closeButton.bottomAnchor),
            scroll.leadingAnchor.constraint(equalTo: glass.contentView.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: glass.contentView.trailingAnchor),
            scroll.bottomAnchor.constraint(equalTo: glass.contentView.bottomAnchor, constant: -6),
            rows.topAnchor.constraint(equalTo: scroll.contentLayoutGuide.topAnchor),
            rows.bottomAnchor.constraint(equalTo: scroll.contentLayoutGuide.bottomAnchor),
            rows.leadingAnchor.constraint(equalTo: scroll.contentLayoutGuide.leadingAnchor),
            rows.trailingAnchor.constraint(equalTo: scroll.contentLayoutGuide.trailingAnchor),
            rows.widthAnchor.constraint(equalTo: scroll.frameLayoutGuide.widthAnchor),
            fit,
        ])
        reload()
    }

    required init?(coder: NSCoder) { fatalError() }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        timer?.invalidate()
        timer = nil
        guard window != nil else { return }
        timer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.reload() }
        }
    }

    func reload() {
        let roster = handle.backgroundRoster(nowMs: nowMs())
        heading.text = roster.agents.isEmpty ? roster.shellsSection : roster.heading
        let caption = roster.agents.isEmpty || roster.shells.isEmpty ? nil : roster.shellsSection
        let next = roster.agents.map { "a\($0.taskId)" } + (caption == nil ? [] : ["c"]) + roster.shells.map { "s\($0.taskId)" }
        if next != shape {
            shape = next
            rows.arrangedSubviews.forEach { $0.removeFromSuperview() }
            for i in roster.agents.indices {
                rows.addArrangedSubview(RosterRow(id: "agents-card-row-\(i)", mono: false))
            }
            if caption != nil { rows.addArrangedSubview(SectionCaption()) }
            for i in roster.shells.indices {
                rows.addArrangedSubview(RosterRow(id: "agents-card-shell-\(i)", mono: true))
            }
        }
        var views = rows.arrangedSubviews.makeIterator()
        for (i, run) in roster.agents.enumerated() {
            guard let row = views.next() as? RosterRow else { return }
            row.configure(state: run.state, title: run.title, line: [run.line, run.review].compactMap { $0 }.joined(separator: " · "))
            row.setStop(run.canStop ? stopAction(run.taskId, delegated: run.delegated) : nil, id: "agents-card-stop-\(i)")
            row.onTap = { [weak self] in self?.tap(run) }
        }
        if let caption { (views.next() as? SectionCaption)?.label.text = caption }
        for (i, shell) in roster.shells.enumerated() {
            guard let row = views.next() as? RosterRow else { return }
            row.configure(state: .working, title: shell.label, line: shell.line)
            row.setStop(shell.canStop ? stopAction(shell.taskId, delegated: false) : nil, id: "agents-card-shell-stop-\(i)")
            row.onTap = { [weak self] in self?.openShell(shell) }
        }
    }

    private func stopAction(_ taskId: String, delegated: Bool) -> () -> Void {
        { [weak self] in
            guard let self else { return }
            BackgroundRuns.stop(self.handle, taskId: taskId, delegated: delegated, present: self.present) { [weak self] in self?.reload() }
        }
    }

    private func tap(_ run: AgentRunRow) {
        switch BackgroundRuns.target(run, findings: findings) {
        case let .findings(sheet)?:
            present(sheet)
        case let .session(id)?:
            onClose?()
            open(id)
        case nil:
            break
        }
    }

    private func openShell(_ shell: ShellRow) {
        weak var nav: UINavigationController?
        let output = ShellOutputViewController(handle: handle, shell: shell) { [weak self] in
            guard let self else { return }
            BackgroundRuns.stop(self.handle, taskId: shell.taskId, delegated: false, present: self.present) { [weak self] in self?.reload() }
            nav?.dismiss(animated: true)
        }
        let sheet = UINavigationController(rootViewController: output)
        if let s = sheet.sheetPresentationController {
            s.detents = [.medium(), .large()]
            s.prefersGrabberVisible = true
        }
        nav = sheet
        present(sheet)
    }
}

private final class SectionCaption: UIView {
    let label = UILabel()

    override init(frame: CGRect) {
        super.init(frame: frame)
        label.font = Fonts.ui(.sansMedium, 12)
        label.textColor = Palette.secondary
        label.translatesAutoresizingMaskIntoConstraints = false
        addSubview(label)
        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: 30),
            label.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 14),
            label.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -14),
            label.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -4),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }
}

private final class RosterRow: UIControl {
    var onTap: (() -> Void)?
    private var stopAction: (() -> Void)?

    func setStop(_ action: (() -> Void)?, id: String) {
        stopAction = action
        if action != nil, stopButton == nil {
            let button = BackgroundRuns.stopButton(id: id) { [weak self] in self?.stopAction?() }
            button.setContentCompressionResistancePriority(.required, for: .horizontal)
            stack.addArrangedSubview(button)
            stopButton = button
        }
        stopButton?.isHidden = action == nil
    }

    private let glyph = RunGlyph()
    private let title = UILabel()
    private let line = UILabel()
    private let stack: UIStackView
    private var stopButton: UIButton?

    init(id: String, mono: Bool) {
        stack = UIStackView(arrangedSubviews: [glyph, title, line])
        super.init(frame: .zero)
        accessibilityIdentifier = id
        isAccessibilityElement = false
        title.font = mono ? Fonts.ui(.mono, 12.5) : Fonts.ui(.sans, 14)
        title.textColor = Palette.text
        title.lineBreakMode = .byTruncatingTail
        title.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        title.setContentHuggingPriority(.defaultLow, for: .horizontal)
        title.accessibilityIdentifier = "\(id)-title"
        line.font = Fonts.ui(.sans, 12)
        line.textColor = Palette.secondary
        line.setContentCompressionResistancePriority(.defaultHigh, for: .horizontal)
        line.setContentHuggingPriority(.required, for: .horizontal)
        line.accessibilityIdentifier = "\(id)-line"
        stack.spacing = 8
        stack.alignment = .center
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)
        NSLayoutConstraint.activate([
            heightAnchor.constraint(greaterThanOrEqualToConstant: 40),
            glyph.widthAnchor.constraint(equalToConstant: 14),
            stack.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 14),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            stack.topAnchor.constraint(equalTo: topAnchor, constant: 4),
            stack.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -4),
        ])
        addAction(UIAction { [weak self] _ in self?.onTap?() }, for: .touchUpInside)
    }

    required init?(coder: NSCoder) { fatalError() }

    func configure(state: AgentRunState, title text: String, line detail: String) {
        glyph.state = state
        if title.text != text { title.text = text }
        if line.text != detail { line.text = detail }
    }

    override func hitTest(_ point: CGPoint, with event: UIEvent?) -> UIView? {
        let hit = super.hitTest(point, with: event)
        return hit is UIButton ? hit : hit.map { _ in self }
    }

    override var isHighlighted: Bool {
        didSet { backgroundColor = isHighlighted ? Palette.text.withAlphaComponent(0.06) : nil }
    }
}
