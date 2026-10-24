import UIKit

final class HtmlSheetViewController: UIViewController {
    private let pageTitle: String
    private let page: HtmlPageView
    private let source = UITextView()
    private let segments = UISegmentedControl(items: ["Live", "Source"])
    private var saveButton: UIBarButtonItem?

    init(title: String, html: @escaping () async -> String?, openURL: @escaping (URL) -> Void) {
        pageTitle = title
        page = HtmlPageView(source: html)
        super.init(nibName: nil, bundle: nil)
        page.scrolls = true
        page.accessibilityIdentifier = "html-sheet-page"
        page.openURL = { [weak self] url in self?.dismiss(animated: true) { openURL(url) } }
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background
        navigationItem.title = pageTitle
        segments.selectedSegmentIndex = 0
        segments.accessibilityIdentifier = "html-sheet-view"
        segments.addAction(UIAction { [weak self] _ in self?.showSelected() }, for: .valueChanged)
        let close = UIBarButtonItem(systemItem: .close, primaryAction: UIAction { [weak self] _ in self?.dismiss(animated: true) })
        close.accessibilityIdentifier = "html-sheet-close"
        let save = UIBarButtonItem(title: "Save .html", primaryAction: UIAction { [weak self] _ in self?.save() })
        save.accessibilityIdentifier = "html-sheet-save"
        save.isEnabled = page.html != nil
        saveButton = save
        page.onLoad = { [weak self] in
            self?.saveButton?.isEnabled = true
            self?.showSelected()
        }
        navigationItem.leftBarButtonItem = close
        navigationItem.rightBarButtonItem = save
        toolbarItems = [.flexibleSpace(), UIBarButtonItem(customView: segments), .flexibleSpace()]
        navigationController?.isToolbarHidden = false

        source.isEditable = false
        source.backgroundColor = .clear
        source.textColor = Palette.text
        source.font = UIFontMetrics(forTextStyle: .body).scaledFont(for: .monospacedSystemFont(ofSize: 12, weight: .regular))
        source.textContainerInset = UIEdgeInsets(top: 16, left: 12, bottom: 16, right: 12)
        source.accessibilityIdentifier = "html-sheet-source"
        source.isHidden = true
        for v in [page, source] as [UIView] {
            v.translatesAutoresizingMaskIntoConstraints = false
            view.addSubview(v)
            NSLayoutConstraint.activate([
                v.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor),
                v.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor),
                v.leadingAnchor.constraint(equalTo: view.leadingAnchor),
                v.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            ])
        }
    }

    private func showSelected() {
        guard let html = page.html, segments.selectedSegmentIndex == 1 else {
            page.isHidden = false
            source.isHidden = true
            return
        }
        if source.text.isEmpty { source.text = htmlRenderSource(html: html) }
        page.isHidden = true
        source.isHidden = false
    }

    private func save() {
        guard let html = page.html else { return }
        let url = FileManager.default.temporaryDirectory.appendingPathComponent(htmlRenderFileName(title: pageTitle))
        guard (try? html.write(to: url, atomically: true, encoding: .utf8)) != nil else { return }
        present(UIDocumentPickerViewController(forExporting: [url], asCopy: true), animated: true)
    }
}
