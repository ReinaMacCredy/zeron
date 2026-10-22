import UIKit

final class LimitCardViewController: UIViewController {
    var onResolve: ((String) async -> Bool)?
    var onOpenChat: (() -> Void)?
    private var card: LimitCard
    private var selectedSwitch: Bool
    private let options = UIStackView()
    private let countdown = UILabel()
    private let cancel = UIButton(configuration: .plain())
    private let act = UIButton(configuration: .filled())
    private var timer: Timer?

    init(card: LimitCard) {
        self.card = card
        selectedSwitch = card.preselectSwitch && card.switch != nil
        super.init(nibName: nil, bundle: nil)
        title = "Usage limit"
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background
        let heading = UILabel()
        heading.font = Fonts.ui(.sansMedium, 15)
        heading.textColor = Palette.text
        heading.numberOfLines = 0
        heading.text = card.title
        let resets = UILabel()
        resets.font = Fonts.ui(.sans, 13)
        resets.textColor = Palette.tertiary
        resets.text = card.resets
        options.axis = .vertical
        options.spacing = 8
        countdown.font = Fonts.ui(.sans, 13)
        countdown.textColor = Palette.secondary
        cancel.configuration?.title = "Cancel"
        cancel.addAction(UIAction { [weak self] _ in self?.resolve("hold") }, for: .touchUpInside)
        act.configuration?.baseBackgroundColor = Palette.text
        act.configuration?.baseForegroundColor = Palette.background
        act.configuration?.cornerStyle = .capsule
        act.addAction(UIAction { [weak self] _ in
            guard let self else { return }
            self.resolve(self.selectedSwitch ? "switch" : "wait")
        }, for: .touchUpInside)
        let footer = UIStackView(arrangedSubviews: [countdown, cancel, UIView(), act])
        footer.alignment = .center
        footer.spacing = 8
        let stack = UIStackView(arrangedSubviews: [heading, resets, options, footer])
        stack.axis = .vertical
        stack.spacing = 10
        stack.setCustomSpacing(16, after: resets)
        stack.setCustomSpacing(16, after: options)
        stack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 12),
            stack.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 16),
            stack.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -16),
        ])
        navigationItem.leftBarButtonItem = UIBarButtonItem(title: "Open chat", primaryAction: UIAction { [weak self] _ in
            self?.dismiss(animated: true) { self?.onOpenChat?() }
        })
        navigationItem.rightBarButtonItem = UIBarButtonItem(systemItem: .done, primaryAction: UIAction { [weak self] _ in
            self?.dismiss(animated: true)
        })
        render()
        timer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in self?.renderCountdown() }
    }

    override func viewDidDisappear(_ animated: Bool) {
        super.viewDidDisappear(animated)
        timer?.invalidate()
    }

    private func render() {
        options.arrangedSubviews.forEach { $0.removeFromSuperview() }
        if let choice = card.switch { options.addArrangedSubview(option(choice, switching: true)) }
        options.addArrangedSubview(option(card.wait, switching: false))
        act.configuration?.attributedTitle = AttributedString(
            (selectedSwitch ? card.switch?.act : nil) ?? card.wait.act,
            attributes: AttributeContainer([.font: Fonts.ui(.sansMedium, 14)])
        )
        renderCountdown()
    }

    private func renderCountdown() {
        let left = card.actAtMs.map { Int(($0 - Int64(Date().timeIntervalSince1970 * 1000) + 999) / 1000) } ?? 0
        countdown.isHidden = left <= 0
        cancel.isHidden = left <= 0
        countdown.text = "\(card.countdownVerb) in \(left) s"
    }

    private func option(_ choice: LimitChoice, switching: Bool) -> UIView {
        let chosen = switching == selectedSwitch
        let box = UIControl()
        box.layer.cornerRadius = 10
        box.layer.borderWidth = 1
        box.layer.borderColor = (chosen ? Palette.warning.withAlphaComponent(0.7) : Palette.hairline).cgColor
        box.backgroundColor = chosen ? Palette.warning.withAlphaComponent(0.08) : .clear
        box.accessibilityTraits = chosen ? [.button, .selected] : .button
        box.accessibilityLabel = choice.title
        let radio = UIImageView(image: UIImage(systemName: chosen ? "largecircle.fill.circle" : "circle"))
        radio.tintColor = chosen ? Palette.warning : Palette.tertiary
        radio.setContentHuggingPriority(.required, for: .horizontal)
        let title = UILabel()
        title.font = Fonts.ui(.sansMedium, 14)
        title.textColor = Palette.text
        title.numberOfLines = 0
        title.text = choice.title
        let detail = UILabel()
        detail.font = Fonts.ui(.sans, 12.5)
        detail.textColor = Palette.secondary
        detail.numberOfLines = 0
        detail.text = choice.detail
        let text = UIStackView(arrangedSubviews: [title, detail])
        text.axis = .vertical
        text.spacing = 2
        let row = UIStackView(arrangedSubviews: [radio, text])
        row.spacing = 10
        row.alignment = .top
        row.isUserInteractionEnabled = false
        row.translatesAutoresizingMaskIntoConstraints = false
        box.addSubview(row)
        NSLayoutConstraint.activate([
            row.topAnchor.constraint(equalTo: box.topAnchor, constant: 10),
            row.bottomAnchor.constraint(equalTo: box.bottomAnchor, constant: -10),
            row.leadingAnchor.constraint(equalTo: box.leadingAnchor, constant: 12),
            row.trailingAnchor.constraint(equalTo: box.trailingAnchor, constant: -12),
        ])
        box.addAction(UIAction { [weak self] _ in self?.select(switching) }, for: .touchUpInside)
        return box
    }

    private func select(_ switching: Bool) {
        guard switching != selectedSwitch else { return }
        selectedSwitch = switching
        if switching != card.preselectSwitch, let at = card.actAtMs, at > Int64(Date().timeIntervalSince1970 * 1000) {
            resolve("hold")
        }
        render()
    }

    private func resolve(_ choice: String) {
        Task { @MainActor [weak self] in
            guard let self, let onResolve = self.onResolve else { return }
            guard await onResolve(choice) else { return }
            if choice == "hold" {
                self.card.actAtMs = nil
                self.renderCountdown()
            } else {
                self.dismiss(animated: true)
            }
        }
    }
}
