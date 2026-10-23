import UIKit

final class WhatsNewViewController: UIViewController {
    private let entries: [ChangelogEntry]
    private let full: Bool

    static func present(from host: UIViewController, entry: ChangelogEntry? = nil) {
        guard let entry = entry ?? whatsNew() else { return }
        let nav = UINavigationController(rootViewController: WhatsNewViewController(entries: [entry], full: false))
        nav.sheetPresentationController?.detents = [.medium(), .large()]
        nav.sheetPresentationController?.prefersGrabberVisible = true
        host.present(nav, animated: true)
    }

    init(entries: [ChangelogEntry], full: Bool) {
        self.entries = entries
        self.full = full
        super.init(nibName: nil, bundle: nil)
        title = full ? "Changelog" : "What's new?"
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background
        navigationItem.largeTitleDisplayMode = .never
        if !full {
            navigationItem.rightBarButtonItem = UIBarButtonItem(title: "Got it", primaryAction: UIAction { [weak self] _ in
                self?.dismiss(animated: true)
            })
        }
        let scroll = UIScrollView()
        scroll.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(scroll)
        let stack = UIStackView()
        stack.axis = .vertical
        stack.spacing = 18
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
        for entry in entries {
            stack.addArrangedSubview(label("v\(entry.version) · \(entry.date)", font: Fonts.ui(.sansMedium, 13), color: Palette.secondary))
            for feature in entry.features {
                let block = UIStackView(arrangedSubviews: [
                    label(feature.title, font: Fonts.ui(.sansMedium, 17), color: Palette.text),
                    label(feature.description, font: Fonts.ui(.sans, 15), color: Palette.secondary),
                ] + (feature.image.flatMap(picture).map { [$0] } ?? []))
                block.axis = .vertical
                block.spacing = 4
                block.setCustomSpacing(10, after: block.arrangedSubviews[1])
                stack.addArrangedSubview(block)
            }
        }
        if !full {
            var config = UIButton.Configuration.plain()
            config.title = "View changelog →"
            config.contentInsets = .zero
            let button = UIButton(configuration: config, primaryAction: UIAction { [weak self] _ in
                self?.navigationController?.pushViewController(WhatsNewViewController(entries: changelog(), full: true), animated: true)
            })
            button.tintColor = Palette.accent
            button.contentHorizontalAlignment = .leading
            button.accessibilityIdentifier = "whatsnew-changelog"
            stack.addArrangedSubview(button)
        }
    }

    private func label(_ text: String, font: UIFont, color: UIColor) -> UILabel {
        let label = UILabel()
        label.text = text
        label.font = font
        label.textColor = color
        label.numberOfLines = 0
        return label
    }

    private func picture(_ name: String) -> UIView? {
        guard let image = bundledImage(name) else { return nil }
        let view = UIImageView(image: image)
        view.contentMode = .scaleAspectFit
        view.clipsToBounds = true
        view.layer.cornerRadius = 12
        view.layer.cornerCurve = .continuous
        view.accessibilityIdentifier = "whatsnew-image-\(name)"
        view.heightAnchor.constraint(equalTo: view.widthAnchor, multiplier: image.size.height / image.size.width).isActive = true
        return view
    }
}

func bundledImage(_ name: String) -> UIImage? {
    docsImage(name: name).flatMap { UIImage(data: Data($0)) }
}

/// The post-update card the desktop shows once after an update, floating
/// over the sessions list until opened or dismissed.
final class WhatsNewCardView: UIView {
    private static let seenKey = "whatsNewLastSeen"
    private let version: String
    private weak var host: UIViewController?

    static func install(on host: UIViewController) {
        let check = whatsNewCard(lastSeen: UserDefaults.standard.string(forKey: seenKey))
        if check.advance { UserDefaults.standard.set(check.version, forKey: seenKey) }
        guard let entry = check.entry else { return }
        let card = WhatsNewCardView(entry: entry, version: check.version, host: host)
        host.view.addSubview(card)
        let guide = host.view.safeAreaLayoutGuide
        NSLayoutConstraint.activate([
            card.leadingAnchor.constraint(equalTo: guide.leadingAnchor, constant: 16),
            card.trailingAnchor.constraint(equalTo: guide.trailingAnchor, constant: -16),
            card.bottomAnchor.constraint(equalTo: guide.bottomAnchor, constant: -12),
        ])
    }

    private init(entry: ChangelogEntry, version: String, host: UIViewController) {
        self.version = version
        self.host = host
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        accessibilityIdentifier = "whatsnew-card"
        backgroundColor = Palette.elevated
        layer.cornerRadius = 16
        layer.cornerCurve = .continuous
        layer.shadowColor = UIColor.black.cgColor
        layer.shadowOpacity = 0.18
        layer.shadowRadius = 16
        layer.shadowOffset = CGSize(width: 0, height: 6)

        let stack = UIStackView()
        stack.axis = .vertical
        stack.spacing = 6
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)
        if let image = entry.hero.flatMap(bundledImage) {
            let hero = UIImageView(image: image)
            hero.contentMode = .scaleAspectFill
            hero.clipsToBounds = true
            hero.layer.cornerRadius = 10
            hero.layer.cornerCurve = .continuous
            hero.heightAnchor.constraint(equalToConstant: 120).isActive = true
            stack.addArrangedSubview(hero)
            stack.setCustomSpacing(12, after: hero)
        }
        let badge = UILabel()
        badge.text = "New · v\(entry.version)"
        badge.font = Fonts.ui(.sansMedium, 12)
        badge.textColor = Palette.accent
        let title = UILabel()
        title.text = entry.features.first?.title ?? "What's new in v\(entry.version)"
        title.font = Fonts.ui(.sansMedium, 16)
        title.textColor = Palette.text
        title.numberOfLines = 2
        var config = UIButton.Configuration.plain()
        config.title = "See what’s new →"
        config.contentInsets = .zero
        let open = UIButton(configuration: config, primaryAction: UIAction { [weak self] _ in
            guard let self, let host = self.host else { return }
            self.close()
            WhatsNewViewController.present(from: host, entry: entry)
        })
        open.tintColor = Palette.accent
        open.contentHorizontalAlignment = .leading
        open.accessibilityIdentifier = "whatsnew-card-open"
        [badge, title, open].forEach(stack.addArrangedSubview)

        let dismiss = UIButton(type: .system, primaryAction: UIAction(image: UIImage(systemName: "xmark")) { [weak self] _ in self?.close() })
        dismiss.tintColor = Palette.secondary
        dismiss.accessibilityLabel = "Dismiss"
        dismiss.accessibilityIdentifier = "whatsnew-card-dismiss"
        dismiss.translatesAutoresizingMaskIntoConstraints = false
        addSubview(dismiss)
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: topAnchor, constant: 12),
            stack.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -12),
            stack.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 14),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -14),
            dismiss.topAnchor.constraint(equalTo: topAnchor, constant: 4),
            dismiss.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -4),
            dismiss.widthAnchor.constraint(equalToConstant: 36),
            dismiss.heightAnchor.constraint(equalToConstant: 36),
        ])
        bringSubviewToFront(dismiss)
    }

    required init?(coder: NSCoder) { fatalError() }

    private func close() {
        UserDefaults.standard.set(version, forKey: Self.seenKey)
        UIView.animate(withDuration: 0.2, animations: { self.alpha = 0 }, completion: { _ in self.removeFromSuperview() })
    }
}

final class DocsViewController: UIViewController, UICollectionViewDelegate {
    private let sections = docsSections()
    private var collectionView: UICollectionView!

    override func viewDidLoad() {
        super.viewDidLoad()
        title = "Docs"
        view.backgroundColor = Palette.background
        navigationItem.largeTitleDisplayMode = .never
        var config = UICollectionLayoutListConfiguration(appearance: .insetGrouped)
        config.backgroundColor = .clear
        config.headerMode = .supplementary
        config.footerMode = .supplementary
        collectionView = UICollectionView(frame: view.bounds, collectionViewLayout: UICollectionViewCompositionalLayout.list(using: config))
        collectionView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        collectionView.backgroundColor = .clear
        collectionView.delegate = self
        collectionView.dataSource = self
        collectionView.register(UICollectionViewListCell.self, forCellWithReuseIdentifier: "cell")
        collectionView.register(UICollectionViewListCell.self, forSupplementaryViewOfKind: UICollectionView.elementKindSectionHeader, withReuseIdentifier: "header")
        collectionView.register(UICollectionViewListCell.self, forSupplementaryViewOfKind: UICollectionView.elementKindSectionFooter, withReuseIdentifier: "footer")
        view.addSubview(collectionView)
    }

    func collectionView(_ collectionView: UICollectionView, didSelectItemAt path: IndexPath) {
        collectionView.deselectItem(at: path, animated: true)
        let article = sections[path.section].articles[path.item]
        navigationController?.pushViewController(DocArticleViewController(id: article.id, title: article.title), animated: true)
    }
}

extension DocsViewController: UICollectionViewDataSource {
    func numberOfSections(in collectionView: UICollectionView) -> Int { sections.count }

    func collectionView(_ collectionView: UICollectionView, numberOfItemsInSection section: Int) -> Int {
        sections[section].articles.count
    }

    func collectionView(_ collectionView: UICollectionView, cellForItemAt path: IndexPath) -> UICollectionViewCell {
        let cell = collectionView.dequeueReusableCell(withReuseIdentifier: "cell", for: path) as! UICollectionViewListCell
        let article = sections[path.section].articles[path.item]
        var c = UIListContentConfiguration.subtitleCell()
        c.text = article.title
        c.textProperties.font = Fonts.ui(.sansMedium, 16)
        c.textProperties.color = Palette.text
        c.secondaryText = article.blurb
        c.secondaryTextProperties.font = Fonts.ui(.sans, 13)
        c.secondaryTextProperties.color = Palette.secondary
        cell.contentConfiguration = c
        var bg = UIBackgroundConfiguration.listGroupedCell()
        bg.backgroundColor = Palette.elevated
        cell.backgroundConfiguration = bg
        cell.accessories = [.disclosureIndicator()]
        cell.accessibilityIdentifier = "doc-\(article.id)"
        return cell
    }

    func collectionView(_ collectionView: UICollectionView, viewForSupplementaryElementOfKind kind: String, at path: IndexPath) -> UICollectionReusableView {
        let header = kind == UICollectionView.elementKindSectionHeader
        let view = collectionView.dequeueReusableSupplementaryView(ofKind: kind, withReuseIdentifier: header ? "header" : "footer", for: path) as! UICollectionViewListCell
        var c = header ? UIListContentConfiguration.groupedHeader() : UIListContentConfiguration.groupedFooter()
        c.text = header ? sections[path.section].title : (path.section == sections.count - 1 ? "Links to settings point at the Mac app; links to other articles open here." : nil)
        view.contentConfiguration = c
        return view
    }
}

final class DocArticleViewController: UIViewController {
    private let id: String
    private lazy var relay = FrameRelay { [weak self] in self?.applyFrame() }
    private lazy var engine = TranscriptView(text: TextEngine.shared, listener: relay)
    private lazy var list = TranscriptListView(engine: engine)
    private var placed = false

    init(id: String, title: String) {
        self.id = id
        super.init(nibName: nil, bundle: nil)
        self.title = title
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background
        navigationItem.largeTitleDisplayMode = .never
        list.frame = view.bounds
        list.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        list.accessibilityIdentifier = "doc-article"
        list.onLink = { [weak self] url in self?.open(url) ?? false }
        view.addSubview(list)
        setContentScrollView(list, for: .top)
        list.imageLoader = { name, view in view.image = bundledImage(name) }
        MathSettings.apply(to: engine)
        engine.setDebugEntries(entries: [DebugEntry(id: id, user: false, text: docsBody(id: id) ?? "", streaming: false)], working: false)
    }

    private func applyFrame() {
        list.apply(engine.frame())
        if !placed, engine.frame().rowCount() > 0 {
            placed = true
            list.setContentOffset(CGPoint(x: 0, y: -list.adjustedContentInset.top), animated: false)
        }
    }

    private func open(_ url: URL) -> Bool {
        guard url.scheme == "zeron" else { return false }
        if let id = docsLink(url: url.absoluteString), let article = docsSections().flatMap(\.articles).first(where: { $0.id == id }) {
            navigationController?.pushViewController(DocArticleViewController(id: id, title: article.title), animated: true)
        } else {
            Toast.show("Open this in Zeron on the Mac", in: view.window)
        }
        return true
    }
}
