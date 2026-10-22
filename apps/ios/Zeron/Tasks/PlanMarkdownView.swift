import UIKit

final class PlanMarkdownView: UIView {
    var onHeight: (() -> Void)?
    private let fonts = StyleFonts()
    private lazy var relay = FrameRelay { [weak self] in self?.apply() }
    private lazy var engine = TranscriptView(text: TextEngine.shared, listener: relay)
    private var rows: [RowView] = []
    private var height: CGFloat = 0
    private var viewport: (CGFloat, CGFloat) = (0, 0)
    private var markdown = ""

    override init(frame: CGRect) {
        super.init(frame: frame)
        isAccessibilityElement = true
        accessibilityIdentifier = "plan-markdown"
    }

    required init?(coder: NSCoder) { fatalError() }

    deinit { engine.close() }

    func show(_ text: String) {
        guard text != markdown else { return }
        markdown = text
        accessibilityLabel = text
        engine.setDebugEntries(entries: [DebugEntry(id: "plan", user: false, text: text, streaming: false)], working: false)
    }

    override var intrinsicContentSize: CGSize {
        CGSize(width: UIView.noIntrinsicMetric, height: height)
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        let scale = UIFontMetrics(forTextStyle: .body).scaledValue(for: 17) / 17
        if bounds.width > 0, viewport != (bounds.width, scale) {
            viewport = (bounds.width, scale)
            engine.setViewport(width: Float(bounds.width), textScale: Float(scale))
        }
    }

    private func apply() {
        let frame = engine.frame()
        if frame.styleCount() != fonts.count { fonts.update(frame.styles()) }
        let placements = frame.rowsIn(y0: 0, y1: .greatestFiniteMagnitude)
        while rows.count < placements.count {
            let row = RowView()
            addSubview(row)
            rows.append(row)
        }
        for (ix, row) in rows.enumerated() {
            guard ix < placements.count, let display = frame.display(index: placements[ix].index) else {
                row.isHidden = true
                continue
            }
            let p = placements[ix]
            row.isHidden = false
            row.configure(RowModel(display: display, fonts: fonts), kind: p.kind)
            row.frame = CGRect(x: 0, y: CGFloat(p.y), width: bounds.width, height: CGFloat(p.height))
        }
        let next = CGFloat(frame.totalHeight())
        if next != height {
            height = next
            invalidateIntrinsicContentSize()
            onHeight?()
        }
    }
}
