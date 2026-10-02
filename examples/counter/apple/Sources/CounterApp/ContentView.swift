import CarapaceKit
import SwiftUI

struct ContentView: View {
    let store: CounterStore
    @State private var banner: String?

    var body: some View {
        VStack(spacing: 20) {
            Text(store.state.label ?? "Counter")
                .font(.headline)
                .foregroundStyle(.secondary)

            Text("\(store.state.count)")
                .font(.system(size: 72, weight: .semibold, design: .rounded))
                .monospacedDigit()
                .animation(.snappy, value: store.state.count)

            // A pure query into the Rust core: no state involved, safe to call while rendering.
            Text((try? store.query(.describe(value: store.state.count)))?.text ?? "")
                .font(.callout)
                .foregroundStyle(.secondary)

            HStack(spacing: 12) {
                Button { store.send(.decrement) } label: { Image(systemName: "minus") }
                Button { store.send(.increment) } label: { Image(systemName: "plus") }
                    .keyboardShortcut(.defaultAction)
            }
            .controlSize(.large)
            .buttonStyle(.bordered)

            Stepper(
                "Step \(store.state.step)",
                value: store.binding(\.step) { .setStep(step: $0) },
                in: 1...100
            )

            HStack {
                Button(store.state.mode == .ticking ? "Stop ticking" : "Tick every 250 ms") {
                    store.send(store.state.mode == .ticking ? .stopTicking : .startTicking)
                }
                Button("Fetch") { store.send(.fetch) }
                    .disabled(store.state.mode == .fetching)
                Button("Reset", role: .destructive) { store.send(.reset) }
            }

            HistoryBars(values: store.state.history)
                .frame(height: 44)

            if let banner {
                Label(banner, systemImage: "bell.fill")
                    .font(.callout)
                    .padding(8)
                    .background(.thinMaterial, in: Capsule())
                    .transition(.move(edge: .bottom).combined(with: .opacity))
            }
            if let fault = store.faults.last {
                Text(fault).font(.caption).foregroundStyle(.red)
            }
        }
        .padding(24)
        .onAppear {
            // Events are requests from the core to the platform; here: show a banner.
            store.onEvent = { event in
                switch event {
                case let .notify(title, body):
                    withAnimation { banner = "\(title): \(body)" }
                    Task {
                        try? await Task.sleep(for: .seconds(2.5))
                        withAnimation { banner = nil }
                    }
                case .summary:
                    break
                }
            }
        }
    }
}

struct HistoryBars: View {
    let values: [Int]
    var body: some View {
        GeometryReader { geo in
            let peak = max(values.map { abs($0) }.max() ?? 1, 1)
            HStack(alignment: .bottom, spacing: 4) {
                ForEach(Array(values.enumerated()), id: \.offset) { _, v in
                    RoundedRectangle(cornerRadius: 3)
                        .fill(v >= 0 ? Color.accentColor : Color.orange)
                        .frame(height: max(4, geo.size.height * CGFloat(abs(v)) / CGFloat(peak)))
                }
            }
            .frame(maxWidth: .infinity, alignment: .trailing)
        }
        .accessibilityLabel("Recent values")
    }
}

struct MenuContent: View {
    let store: CounterStore
    var body: some View {
        VStack(spacing: 10) {
            Text("\(store.state.count)").font(.title.monospacedDigit())
            HStack {
                Button("−") { store.send(.decrement) }
                Button("+") { store.send(.increment) }
            }
            Divider()
            Button("Quit") { NSApplication.shared.terminate(nil) }
        }
        .padding(14)
        .frame(width: 160)
    }
}
