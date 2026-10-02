import CarapaceFFI
import CarapaceKit
import SwiftUI

@main
struct CounterMain: App {
    @State private var store: CounterStore

    init() {
        do {
            // The only line that knows the core is Rust: the C ABI backend.
            let store = try CounterStore(backend: RustBackend(), config: .init(start: 0))
            _store = State(initialValue: store)
        } catch {
            fatalError("Counter core did not start: \(error.localizedDescription)")
        }
    }

    var body: some Scene {
        WindowGroup("Counter") {
            ContentView(store: store)
                .frame(width: 380, height: 460)
        }
        .windowResizability(.contentSize)

        MenuBarExtra {
            MenuContent(store: store)
        } label: {
            Label("\(store.state.count)", systemImage: "number.circle")
        }
        .menuBarExtraStyle(.window)
    }
}
