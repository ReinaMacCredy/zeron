import UIKit

final class TerminalViewController: UIViewController {
    static let pollInterval: Duration = .seconds(1)

    private let source: SessionSource
    private var selected: String?
    private var tabs: [TerminalTab] = []
    private var poll: Task<Void, Never>?
    private let tabScroll = UIScrollView()
    private let tabStack = UIStackView()
    private let output = UITextView()
    private let empty = UILabel()
    private let footer = UILabel()

    init(source: SessionSource, terminalId: String? = nil) {
        self.source = source
        selected = terminalId
        super.init(nibName: nil, bundle: nil)
        title = "Terminal"
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background
        navigationItem.rightBarButtonItem = UIBarButtonItem(systemItem: .done, primaryAction: UIAction { [weak self] _ in
            self?.dismiss(animated: true)
        })

        tabStack.axis = .horizontal
        tabStack.spacing = 8
        tabStack.translatesAutoresizingMaskIntoConstraints = false
        tabScroll.showsHorizontalScrollIndicator = false
        tabScroll.translatesAutoresizingMaskIntoConstraints = false
        tabScroll.addSubview(tabStack)

        output.isEditable = false
        output.isSelectable = true
        output.font = Fonts.ui(.mono, 12)
        output.textContainerInset = UIEdgeInsets(top: 12, left: 10, bottom: 12, right: 10)
        output.layer.cornerRadius = 12
        output.layer.cornerCurve = .continuous
        output.accessibilityIdentifier = "terminal-output"
        output.translatesAutoresizingMaskIntoConstraints = false

        empty.text = "No terminals in this chat yet."
        empty.font = Fonts.ui(.sans, 15)
        empty.textColor = Palette.secondary
        empty.textAlignment = .center
        empty.isHidden = true
        empty.accessibilityIdentifier = "terminal-empty"
        empty.translatesAutoresizingMaskIntoConstraints = false

        footer.text = "Output only. Type into this terminal on the Mac."
        footer.font = Fonts.ui(.sans, 12)
        footer.textColor = Palette.secondary
        footer.numberOfLines = 0
        footer.accessibilityIdentifier = "terminal-footer"
        footer.translatesAutoresizingMaskIntoConstraints = false

        [tabScroll, output, empty, footer].forEach(view.addSubview)
        let guide = view.safeAreaLayoutGuide
        NSLayoutConstraint.activate([
            tabScroll.topAnchor.constraint(equalTo: guide.topAnchor, constant: 8),
            tabScroll.leadingAnchor.constraint(equalTo: guide.leadingAnchor),
            tabScroll.trailingAnchor.constraint(equalTo: guide.trailingAnchor),
            tabScroll.heightAnchor.constraint(equalToConstant: 34),
            tabStack.topAnchor.constraint(equalTo: tabScroll.contentLayoutGuide.topAnchor),
            tabStack.bottomAnchor.constraint(equalTo: tabScroll.contentLayoutGuide.bottomAnchor),
            tabStack.leadingAnchor.constraint(equalTo: tabScroll.contentLayoutGuide.leadingAnchor, constant: 16),
            tabStack.trailingAnchor.constraint(equalTo: tabScroll.contentLayoutGuide.trailingAnchor, constant: -16),
            tabStack.heightAnchor.constraint(equalTo: tabScroll.frameLayoutGuide.heightAnchor),
            output.topAnchor.constraint(equalTo: tabScroll.bottomAnchor, constant: 10),
            output.leadingAnchor.constraint(equalTo: guide.leadingAnchor, constant: 12),
            output.trailingAnchor.constraint(equalTo: guide.trailingAnchor, constant: -12),
            footer.topAnchor.constraint(equalTo: output.bottomAnchor, constant: 8),
            footer.leadingAnchor.constraint(equalTo: guide.leadingAnchor, constant: 16),
            footer.trailingAnchor.constraint(equalTo: guide.trailingAnchor, constant: -16),
            footer.bottomAnchor.constraint(equalTo: guide.bottomAnchor, constant: -8),
            empty.centerXAnchor.constraint(equalTo: output.centerXAnchor),
            empty.centerYAnchor.constraint(equalTo: output.centerYAnchor),
        ])
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        poll?.cancel()
        poll = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                await self?.refresh()
                try? await Task.sleep(for: Self.pollInterval)
            }
        }
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        poll?.cancel()
        poll = nil
    }

    private func refresh() async {
        do {
            let tabs = try await source.terminals()
            if tabs != self.tabs {
                self.tabs = tabs
                renderTabs()
            }
            if selected.map({ id in !tabs.contains { $0.id == id } }) ?? true { selected = tabs.first?.id }
            empty.isHidden = !tabs.isEmpty
            output.isHidden = tabs.isEmpty
            guard let id = selected else { return }
            let screen = try await source.terminal(id, dark: traitCollection.userInterfaceStyle == .dark)
            guard id == selected else { return }
            paint(screen)
        } catch is CancellationError {
        } catch {
            Toast.show("Couldn't read the terminal. Try again.", in: view.window)
            poll?.cancel()
        }
    }

    private func renderTabs() {
        tabStack.arrangedSubviews.forEach { $0.removeFromSuperview() }
        for tab in tabs {
            var config = UIButton.Configuration.gray()
            config.cornerStyle = .capsule
            config.title = tab.title
            config.subtitle = tab.state == .running ? nil : tab.status
            config.image = UIImage(systemName: "circle.fill", withConfiguration: UIImage.SymbolConfiguration(pointSize: 6))
            config.imagePadding = 6
            config.imageColorTransformer = UIConfigurationColorTransformer { _ in Self.stateColor(tab.state) }
            config.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { attrs in
                var attrs = attrs
                attrs.font = Fonts.ui(.sansMedium, 13)
                return attrs
            }
            config.subtitleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { attrs in
                var attrs = attrs
                attrs.font = Fonts.ui(.sans, 10)
                return attrs
            }
            config.titleAlignment = .leading
            let button = UIButton(configuration: config, primaryAction: UIAction { [weak self] _ in
                self?.select(tab.id)
            })
            button.isSelected = tab.id == selected
            button.accessibilityIdentifier = "terminal-tab-\(tab.id)"
            button.accessibilityValue = tab.status
            tabStack.addArrangedSubview(button)
        }
    }

    private func select(_ id: String) {
        selected = id
        renderTabs()
        Task { @MainActor [weak self] in await self?.refresh() }
    }

    private static func stateColor(_ state: RunState) -> UIColor {
        switch state {
        case .running: Palette.accent
        case .done: Palette.success
        case .failed: Palette.danger
        }
    }

    private func paint(_ screen: TerminalView) {
        output.backgroundColor = Self.color(screen.background)
        let regular = Fonts.ui(.mono, 12)
        let text = NSMutableAttributedString()
        for (i, line) in screen.lines.enumerated() {
            if i > 0 { text.append(NSAttributedString(string: "\n", attributes: [.font: regular])) }
            for span in line {
                var attrs: [NSAttributedString.Key: Any] = [
                    .font: Self.font(bold: span.bold, italic: span.italic),
                    .foregroundColor: Self.color(span.fg),
                ]
                if let bg = span.bg { attrs[.backgroundColor] = Self.color(bg) }
                if span.underline { attrs[.underlineStyle] = NSUnderlineStyle.single.rawValue }
                text.append(NSAttributedString(string: span.text, attributes: attrs))
            }
        }
        let atBottom = output.contentOffset.y + output.bounds.height >= output.contentSize.height - 24
        output.attributedText = text
        output.accessibilityValue = screen.tab.status
        if atBottom {
            output.layoutIfNeeded()
            output.scrollRangeToVisible(NSRange(location: text.length, length: 0))
        }
    }

    private static func font(bold: Bool, italic: Bool) -> UIFont {
        switch (bold, italic) {
        case (true, _): Fonts.ui(.monoSemibold, 12)
        case (false, true): Fonts.ui(.monoItalic, 12)
        case (false, false): Fonts.ui(.mono, 12)
        }
    }

    static func color(_ rgba: UInt32) -> UIColor {
        UIColor(
            red: CGFloat(rgba >> 24 & 0xFF) / 255,
            green: CGFloat(rgba >> 16 & 0xFF) / 255,
            blue: CGFloat(rgba >> 8 & 0xFF) / 255,
            alpha: CGFloat(rgba & 0xFF) / 255
        )
    }
}
