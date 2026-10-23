import UIKit

final class MissionIntroViewController: UIViewController {
    static let seenKey = "missionIntroSeen"
    private static var shown = false

    static func due(app: AppModel) -> Bool {
        guard !shown else { return false }
        if ProcessInfo.processInfo.arguments.contains("-missionIntro") { return true }
        guard !UserDefaults.standard.bool(forKey: seenKey), let client = app.client else { return false }
        return client.missionIntroDue()
    }

    private let intro = missionIntro()
    private let onTry: () -> Void
    private var page = 0
    private let visual = UIImageView()
    private let titleLabel = UILabel()
    private let body = UILabel()
    private let dots = UIPageControl()
    private let nextButton = UIButton(type: .system)
    private static let glyphs = ["glyph-mission", "list.number", "point.3.connected.trianglepath.dotted", "glyph-supervisor"]

    init(onTry: @escaping () -> Void) {
        self.onTry = onTry
        super.init(nibName: nil, bundle: nil)
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        Self.shown = true
        view.backgroundColor = Palette.background
        view.accessibilityIdentifier = "mission-intro-sheet"

        let card = UIView()
        card.backgroundColor = Palette.elevated
        card.layer.cornerRadius = 14
        card.layer.cornerCurve = .continuous
        card.layer.borderWidth = 1
        card.layer.borderColor = Palette.hairline.cgColor
        visual.contentMode = .scaleAspectFit
        visual.tintColor = Palette.accent
        visual.translatesAutoresizingMaskIntoConstraints = false
        card.addSubview(visual)

        titleLabel.font = Fonts.ui(.sansSemibold, TypeScale.size(20))
        titleLabel.textColor = Palette.text
        titleLabel.numberOfLines = 0
        titleLabel.accessibilityIdentifier = "mission-intro-title"
        body.font = Fonts.ui(.sans, TypeScale.size(15))
        body.textColor = Palette.secondary
        body.numberOfLines = 0

        dots.numberOfPages = intro.pages.count
        dots.currentPageIndicatorTintColor = Palette.text
        dots.pageIndicatorTintColor = Palette.text.withAlphaComponent(0.18)
        dots.isUserInteractionEnabled = false
        dots.accessibilityIdentifier = "mission-intro-pages"

        var dismissConfig = UIButton.Configuration.gray()
        dismissConfig.title = intro.dismiss
        dismissConfig.cornerStyle = .capsule
        let dismissButton = UIButton(configuration: dismissConfig, primaryAction: UIAction { [weak self] _ in self?.close(trying: false) })
        dismissButton.accessibilityIdentifier = "mission-intro-dismiss"
        var nextConfig = UIButton.Configuration.filled()
        nextConfig.cornerStyle = .capsule
        nextConfig.baseBackgroundColor = Palette.text
        nextConfig.baseForegroundColor = Palette.background
        nextButton.configuration = nextConfig
        nextButton.addAction(UIAction { [weak self] _ in self?.advance() }, for: .primaryActionTriggered)

        let buttons = UIStackView(arrangedSubviews: [dots, UIView(), dismissButton, nextButton])
        buttons.spacing = 8
        buttons.alignment = .center
        let stack = UIStackView(arrangedSubviews: [card, titleLabel, body, UIView(), buttons])
        stack.axis = .vertical
        stack.spacing = 12
        stack.setCustomSpacing(20, after: card)
        stack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(stack)
        NSLayoutConstraint.activate([
            card.heightAnchor.constraint(equalToConstant: 150),
            visual.centerXAnchor.constraint(equalTo: card.centerXAnchor),
            visual.centerYAnchor.constraint(equalTo: card.centerYAnchor),
            visual.widthAnchor.constraint(equalToConstant: 56),
            visual.heightAnchor.constraint(equalToConstant: 56),
            stack.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 28),
            stack.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 20),
            stack.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -20),
            stack.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor, constant: -12),
        ])
        for direction in [UISwipeGestureRecognizer.Direction.left, .right] {
            let swipe = UISwipeGestureRecognizer(target: self, action: #selector(swiped(_:)))
            swipe.direction = direction
            view.addGestureRecognizer(swipe)
        }
        show(0, forward: true)
    }

    @objc private func swiped(_ swipe: UISwipeGestureRecognizer) {
        let delta = swipe.direction == .left ? 1 : -1
        let target = min(max(page + delta, 0), intro.pages.count - 1)
        if target != page { show(target, forward: delta > 0) }
    }

    private func advance() {
        if page + 1 < intro.pages.count { show(page + 1, forward: true) } else { close(trying: true) }
    }

    private func show(_ ix: Int, forward: Bool) {
        page = ix
        let last = ix + 1 == intro.pages.count
        let glyph = Self.glyphs[ix]
        visual.image = TaskGlyph.asset(glyph, side: 56) ?? UIImage(systemName: glyph, withConfiguration: UIImage.SymbolConfiguration(pointSize: 44, weight: .regular))
        titleLabel.text = intro.pages[ix].title
        body.text = intro.pages[ix].body
        dots.currentPage = ix
        dots.accessibilityValue = "Page \(ix + 1) of \(intro.pages.count)"
        nextButton.configuration?.title = last ? intro.tryIt : intro.next
        nextButton.accessibilityIdentifier = last ? "mission-intro-try" : "mission-intro-next"
        for v in [visual, titleLabel, body] as [UIView] {
            v.alpha = 0
            v.transform = CGAffineTransform(translationX: forward ? 8 : -8, y: 0)
        }
        UIView.animate(withDuration: 0.25, delay: 0, options: .curveEaseOut) {
            for v in [self.visual, self.titleLabel, self.body] as [UIView] {
                v.alpha = 1
                v.transform = .identity
            }
        }
    }

    private func close(trying: Bool) {
        UserDefaults.standard.set(true, forKey: Self.seenKey)
        let onTry = onTry
        dismiss(animated: true) { if trying { onTry() } }
    }

    override func viewDidDisappear(_ animated: Bool) {
        super.viewDidDisappear(animated)
        UserDefaults.standard.set(true, forKey: Self.seenKey)
    }
}
