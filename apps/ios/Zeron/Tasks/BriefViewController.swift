import UIKit

final class BriefViewController: UIViewController {
    private let app: AppModel
    private let taskId: String
    private let stepId: String?
    private let stack = UIStackView()
    private let spec = PlanMarkdownView()
    private let messages = UIStackView()

    init(app: AppModel, taskId: String, stepId: String?) {
        self.app = app
        self.taskId = taskId
        self.stepId = stepId
        super.init(nibName: nil, bundle: nil)
        navigationItem.largeTitleDisplayMode = .never
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background
        guard let brief = app.client?.brief(taskId: taskId, stepId: stepId) else {
            title = "Brief"
            stack.addArrangedSubview(Self.label("This brief is gone.", font: Fonts.ui(.sans, 15), color: Palette.secondary))
            layout()
            return
        }
        title = brief.title
        let heading = Self.label(brief.title, font: Fonts.ui(.sansSemibold, 18), color: Palette.text)
        heading.accessibilityIdentifier = "brief-title"
        stack.addArrangedSubview(heading)
        if !brief.meta.isEmpty {
            stack.addArrangedSubview(Self.label(brief.meta.joined(separator: " · "), font: Fonts.ui(.sans, 13), color: Palette.secondary))
        }
        stack.setCustomSpacing(20, after: stack.arrangedSubviews.last!)
        if !brief.task.isEmpty {
            stack.addArrangedSubview(Self.section("TASK"))
            stack.addArrangedSubview(Self.label(brief.task, font: Fonts.ui(.sans, 15), color: Palette.text))
        }
        if !brief.spec.isEmpty {
            stack.addArrangedSubview(Self.section("SPEC"))
            spec.show(brief.spec)
            spec.onHeight = { [weak self] in self?.view.setNeedsLayout() }
            stack.addArrangedSubview(spec)
        }
        stack.addArrangedSubview(Self.section("MESSAGES"))
        messages.axis = .vertical
        messages.spacing = 12
        messages.addArrangedSubview(Self.label("Loading…", font: Fonts.ui(.sans, 14), color: Palette.secondary))
        stack.addArrangedSubview(messages)
        layout()
        load(brief)
    }

    private func layout() {
        let scroll = UIScrollView()
        scroll.alwaysBounceVertical = true
        scroll.accessibilityIdentifier = "brief"
        scroll.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(scroll)
        stack.axis = .vertical
        stack.spacing = 8
        stack.translatesAutoresizingMaskIntoConstraints = false
        scroll.addSubview(stack)
        NSLayoutConstraint.activate([
            scroll.topAnchor.constraint(equalTo: view.topAnchor),
            scroll.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            scroll.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            stack.topAnchor.constraint(equalTo: scroll.contentLayoutGuide.topAnchor, constant: 16),
            stack.bottomAnchor.constraint(equalTo: scroll.contentLayoutGuide.bottomAnchor, constant: -24),
            stack.leadingAnchor.constraint(equalTo: scroll.frameLayoutGuide.leadingAnchor, constant: 20),
            stack.trailingAnchor.constraint(equalTo: scroll.frameLayoutGuide.trailingAnchor, constant: -20),
        ])
    }

    private func load(_ brief: Brief) {
        guard let client = app.client else { return }
        let task = taskId
        let step = stepId
        Task { @MainActor in
            let got = (try? await client.briefMessages(taskId: task, stepId: step)) ?? []
            messages.arrangedSubviews.forEach { $0.removeFromSuperview() }
            if got.isEmpty {
                messages.addArrangedSubview(Self.label(brief.threadChatId == nil ? "Not started yet." : "No messages.", font: Fonts.ui(.sans, 14), color: Palette.secondary))
            }
            for message in got {
                let tag = Self.label(message.tag, font: Fonts.ui(.sansMedium, 12), color: message.fromYou ? Palette.accent : Palette.secondary)
                let body = Self.label(message.text, font: Fonts.ui(.sans, 15), color: Palette.text)
                let item = UIStackView(arrangedSubviews: [tag, body])
                item.axis = .vertical
                item.spacing = 3
                messages.addArrangedSubview(item)
            }
        }
    }

    private static func section(_ name: String) -> UILabel {
        label(name, font: Fonts.ui(.sansSemibold, 12), color: Palette.secondary)
    }

    private static func label(_ text: String, font: UIFont, color: UIColor) -> UILabel {
        let label = UILabel()
        label.text = text
        label.font = font
        label.textColor = color
        label.numberOfLines = 0
        return label
    }
}
