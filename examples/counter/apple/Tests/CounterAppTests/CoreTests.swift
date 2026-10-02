import CarapaceFFI
import CarapaceKit
import Foundation
import Testing
@testable import CounterApp

/// These run the real Rust core through the real C ABI.
@MainActor
@Suite struct RealCoreTests {
    func wait(_ what: String, _ ok: () -> Bool) async {
        for _ in 0..<200 {
            if ok() { return }
            try? await Task.sleep(for: .milliseconds(10))
        }
        Issue.record("timed out waiting for \(what)")
    }

    @Test func schemaHashMatchesTheGeneratedBindings() {
        #expect(RustBackend().schemaHash == Counter.schemaHash)
    }

    @Test func startsFromConfigAndCountsByStep() async throws {
        let store = try CounterStore(backend: RustBackend(), config: .init(start: 10))
        #expect(store.state.count == 10)
        store.send(.setStep(step: 5))
        store.send(.increment)
        await wait("count 15") { store.state.count == 15 }
        #expect(store.state.history == [15])
    }

    @Test func timersAndBackgroundWorkRunInTheCore() async throws {
        let store = try CounterStore(backend: RustBackend())
        store.send(.startTicking)
        await wait("three ticks") { store.state.count >= 3 }
        store.send(.stopTicking)
        store.send(.fetch)
        await wait("fetch result") { store.state.count == 100 && store.state.mode == .idle }
    }

    @Test func coreEventsReachTheShellIncludingTheOneFromInit() async throws {
        let store = try CounterStore(backend: RustBackend())
        var titles: [String] = []
        store.onEvent = { if case let .notify(title, _) = $0 { titles.append(title) } }
        #expect(titles == ["Counter"])
        store.send(.fetch)
        await wait("fetched event") { titles.contains("Fetched") }
    }

    @Test func optionalFieldsRoundTrip() async throws {
        let store = try CounterStore(backend: RustBackend())
        store.send(.rename(label: "Groceries"))
        await wait("label") { store.state.label == "Groceries" }
        store.send(.rename(label: nil))
        await wait("label cleared") { store.state.label == nil }
    }

    @Test func pureQueriesRunInTheCoreWithoutState() throws {
        let store = try CounterStore(backend: RustBackend())
        #expect(try store.query(.describe(value: -3)).text == "negative, odd")
        #expect(try store.query(.describe(value: 0)).text == "zero, even")
    }

    @Test func manyStartStopCyclesDoNotLeakOrCrash() throws {
        for _ in 0..<50 {
            let store = try CounterStore(backend: RustBackend())
            store.send(.startTicking)
        }
    }
}
