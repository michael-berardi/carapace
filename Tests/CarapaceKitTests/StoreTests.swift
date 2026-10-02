import Foundation
import Testing
@testable import CarapaceKit

enum Demo: CarapaceApp {
    static let name = "Demo"
    static let schemaHash: UInt64 = 0xABCD
    struct State: Codable, Sendable, Equatable { var n: Int }
    enum Action: Codable, Sendable { case add(Int) }
    enum Event: Codable, Sendable, Equatable { case ping(String) }
    struct Config: Codable, Sendable { var start: Int }
    typealias Query = NoQuery
    typealias Answer = NoAnswer
}

/// A fake core: counts, echoes events, and can be told to misbehave.
final class MockCore: CarapaceCore, @unchecked Sendable {
    private let lock = NSLock()
    private var n: Int
    private var handlers: [Int: @Sendable (CarapaceNotice) -> Void] = [:]
    private var next = 0
    private(set) var stopped = false
    var rejects: String?
    init(n: Int) { self.n = n }

    func dispatch(_ action: Data) throws {
        if let rejects { throw CarapaceError.core(rejects) }
        let value = (try? JSONDecoder().decode([String: [String: Int]].self, from: action))?["add"]?["_0"] ?? 0
        lock.lock(); n += value; let snapshot = Data("{\"n\":\(n)}".utf8); let hs = Array(handlers.values); lock.unlock()
        hs.forEach { $0(.state(snapshot)) }
    }
    func dispatchAndWait(_ action: Data) throws { try dispatch(action) }
    func snapshot() throws -> Data { lock.lock(); defer { lock.unlock() }; return Data("{\"n\":\(n)}".utf8) }
    func subscribe(_ handler: @escaping @Sendable (CarapaceNotice) -> Void) -> Int {
        lock.lock(); next += 1; handlers[next] = handler; let id = next; lock.unlock(); return id
    }
    func unsubscribe(_ id: Int) { lock.lock(); handlers[id] = nil; lock.unlock() }
    func stop() { lock.lock(); stopped = true; lock.unlock() }
    func emit(_ notice: CarapaceNotice) { lock.lock(); let hs = Array(handlers.values); lock.unlock(); hs.forEach { $0(notice) } }
}

struct MockBackend: CarapaceBackend {
    var abiVersion: UInt32 = CarapaceKitABI
    var schemaHash: UInt64 = Demo.schemaHash
    let core: MockCore
    func start(config: Data?) throws -> any CarapaceCore { core }
    func query(_ query: Data) throws -> Data { throw CarapaceError.core("no queries") }
}

@MainActor
func settle() async {
    try? await Task.sleep(for: .milliseconds(80))
}

@MainActor
@Suite struct StoreTests {
    @Test func rendersInitialStateAndFollowsUpdates() async throws {
        let core = MockCore(n: 5)
        let store = try Store<Demo>(backend: MockBackend(core: core))
        #expect(store.state.n == 5)
        store.send(.add(2))
        await settle()
        #expect(store.state.n == 7)
    }

    @Test func sendSyncAppliesTheNewStateBeforeReturning() throws {
        let core = MockCore(n: 0)
        let store = try Store<Demo>(backend: MockBackend(core: core))
        store.sendSync(.add(3))
        #expect(store.state.n == 3)
        store.sendSync(.add(4))
        #expect(store.state.n == 7)
    }

    @Test func burstsCoalesceToTheLatestState() async throws {
        let core = MockCore(n: 0)
        let store = try Store<Demo>(backend: MockBackend(core: core))
        for _ in 0..<500 { store.send(.add(1)) }
        await settle()
        #expect(store.state.n == 500)
    }

    @Test func eventsAreHeldUntilAHandlerIsSetAndNeverDropped() async throws {
        let core = MockCore(n: 0)
        let store = try Store<Demo>(backend: MockBackend(core: core))
        core.emit(.event(Data("{\"ping\":{\"_0\":\"a\"}}".utf8)))
        core.emit(.event(Data("{\"ping\":{\"_0\":\"b\"}}".utf8)))
        await settle()
        #expect(store.faults.isEmpty)
        var got: [Demo.Event] = []
        store.onEvent = { got.append($0) }
        #expect(got == [.ping("a"), .ping("b")])
    }

    @Test func undecodableEventsBecomeFaults() async throws {
        let core = MockCore(n: 0)
        let store = try Store<Demo>(backend: MockBackend(core: core))
        core.emit(.event(Data("{\"nope\":1}".utf8)))
        await settle()
        #expect(store.faults.count == 1)
    }

    @Test func staleBindingsAreRejectedLoudly() {
        let backend = MockBackend(schemaHash: 0x1234, core: MockCore(n: 0))
        #expect(throws: CarapaceError.staleBindings(app: "Demo", expected: 0xABCD, found: 0x1234)) {
            _ = try MainActor.assumeIsolated { try Store<Demo>(backend: backend) }
        }
    }

    @Test func abiMismatchIsRejected() {
        let backend = MockBackend(abiVersion: 99, core: MockCore(n: 0))
        #expect(throws: CarapaceError.abiMismatch(expected: CarapaceKitABI, found: 99)) {
            _ = try MainActor.assumeIsolated { try Store<Demo>(backend: backend) }
        }
    }

    @Test func rejectedActionsBecomeFaultsNotSilence() async throws {
        let core = MockCore(n: 0)
        core.rejects = "cannot decode action"
        let store = try Store<Demo>(backend: MockBackend(core: core))
        var seen: [String] = []
        store.onFault = { seen.append($0) }
        store.send(.add(1))
        #expect(store.faults.count == 1)
        #expect(seen.first?.contains("cannot decode action") == true)
    }

    @Test func coreFaultsAreSurfaced() async throws {
        let core = MockCore(n: 0)
        let store = try Store<Demo>(backend: MockBackend(core: core))
        core.emit(.fault("Demo: update panicked: boom"))
        await settle()
        #expect(store.faults == ["Demo: update panicked: boom"])
    }

    @Test func coreStopsWhenStoreIsReleased() async throws {
        let core = MockCore(n: 0)
        var store: Store<Demo>? = try Store<Demo>(backend: MockBackend(core: core))
        #expect(store != nil)
        store = nil
        await settle()
        #expect(core.stopped)
    }
}
