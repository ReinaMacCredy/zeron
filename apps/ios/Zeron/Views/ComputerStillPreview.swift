import SwiftUI
import UIKit

struct ComputerStillFrameReply: Decodable {
    struct Frame: Decodable {
        var sequence: UInt64
        var data: String
    }
    var frame: Frame?

    var image: (sequence: UInt64, image: UIImage)? {
        guard let frame, let bytes = Data(base64Encoded: frame.data),
              let image = UIImage(data: bytes) else { return nil }
        return (frame.sequence, image)
    }
}

struct ComputerStillPreview: View {
    let working: Bool
    let hostOnline: () -> Bool
    let fetch: () async -> ComputerStillFrameReply?

    static let pollInterval: Duration = .seconds(1)

    static func poll(hostOnline: () -> Bool,
                     fetch: () async -> ComputerStillFrameReply?) async -> ComputerStillFrameReply? {
        guard hostOnline() else { return nil }
        return await fetch()
    }

    @State private var still: (sequence: UInt64, image: UIImage)?
    @State private var hidden = false

    var body: some View {
        ZStack(alignment: .topTrailing) {
            Color.clear.frame(width: 0, height: 0)
            if let still, !hidden {
                ZStack(alignment: .topTrailing) {
                    Image(uiImage: still.image)
                        .resizable()
                        .aspectRatio(contentMode: .fit)
                        .frame(maxWidth: 168, maxHeight: 120)
                        .background(Color.black)
                        .clipShape(RoundedRectangle(cornerRadius: 10))
                        .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(whiteAlpha(0.12), lineWidth: 1))
                        .accessibilityLabel("Computer preview")
                    Button {
                        hidden = true
                    } label: {
                        Image(systemName: "xmark")
                            .font(.system(size: 10, weight: .semibold))
                            .foregroundStyle(Theme.text)
                            .frame(width: 22, height: 22)
                            .background(Color.black.opacity(0.6), in: Circle())
                            .frame(width: 44, height: 44)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .offset(x: 11, y: -11)
                    .accessibilityLabel("Hide Computer preview")
                }
                .padding(12)
                .transition(.opacity)
            }
        }
        .motionAnimation(Motion.fadeQuick, value: still == nil || hidden)
        .task(id: working) {
            guard working else {
                still = nil
                hidden = false
                return
            }
            while !Task.isCancelled {
                let reply = await Self.poll(hostOnline: hostOnline, fetch: fetch)
                guard !Task.isCancelled else { return }
                if let next = reply?.image {
                    if next.sequence != still?.sequence { still = next }
                } else {
                    still = nil
                }
                try? await Task.sleep(for: Self.pollInterval)
            }
        }
    }
}
