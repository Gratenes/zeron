import XCTest
import UIKit
import SwiftUI
@testable import Zeron

final class HarnessCatalogTests: XCTestCase {
    private func runChat(config: ChatConfig) -> Chat {
        Chat(id: "chat", deviceId: "device", title: nil, archived: false, cwd: "/repo",
             branch: nil, checkoutId: nil, config: config, lastMessagePreview: nil,
             lastMessageAt: nil, createdAt: 0, spaceId: nil, lastSeenAt: nil)
    }

    @MainActor
    func testVerifiedEffortAndDisplayedDefaultRideTheActualRunPayload() throws {
        let saved = ChatConfig(harness: "mimir", model: "provider/b", reasoning: "medium", sandbox: "workspace-write")
        var catalog = ModelCatalogState()
        catalog.update([ModelInfo(id: "provider/b", label: "B", description: nil, reasoningLevels: [])])
        var snapshot = runChat(config: catalog.resolvedConfig(saved))
        XCTAssertEqual(SessionStore.runRequest(prompt: "pending", chat: snapshot).reasoning, "medium")

        catalog.update([ModelInfo(id: "provider/b", label: "B", description: nil, reasoningLevels: ["off"])],
                       selectedModel: "provider/b")
        snapshot.config = catalog.resolvedConfig(saved)
        let request = SessionStore.runRequest(prompt: "verified", chat: snapshot, attachments: ["pending://image"])
        let payload = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(request)) as? [String: Any])
        XCTAssertEqual(payload["model"] as? String, "provider/b")
        XCTAssertEqual(payload["reasoning"] as? String, "off")
        XCTAssertEqual(request.reasoning, snapshot.config?.reasoning, "display and submit consume the same resolved value")
        XCTAssertEqual(payload["attachments"] as? [String], ["pending://image"])

        var unspecified = saved
        unspecified.reasoning = nil
        catalog.update([ModelInfo(id: "provider/b", label: "B", description: nil, reasoningLevels: ["low", "high"])],
                       selectedModel: "provider/b")
        snapshot.config = catalog.resolvedConfig(unspecified)
        let defaultRequest = SessionStore.runRequest(prompt: "default", chat: snapshot)
        XCTAssertEqual(defaultRequest.reasoning, "high", "a displayed High default cannot silently submit nil")
        let defaultPayload = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(defaultRequest)) as? [String: Any])
        XCTAssertEqual(defaultPayload["reasoning"] as? String, "high")

        // Catalog details for B never clamp a new, not-yet-verified C pick.
        var newer = saved
        newer.model = "provider/c"
        snapshot.config = catalog.resolvedConfig(newer)
        XCTAssertEqual(SessionStore.runRequest(prompt: "newer", chat: snapshot).model, "provider/c")
        XCTAssertEqual(SessionStore.runRequest(prompt: "newer", chat: snapshot).reasoning, "medium")
    }

    @MainActor
    func testExplicitBuildSurvivesDefaultSelectionPruningAndRunEncoding() throws {
        let mode = ModelOptionInfo(id: "mimir.mode", label: "Mode", choices: [
            ModelOptionChoiceInfo(id: "mode-build", label: "Build"),
            ModelOptionChoiceInfo(id: "mode-plan", label: "Plan")
        ], defaultChoice: "mode-build")
        var saved = ChatConfig(harness: "mimir", model: "provider/b", reasoning: "medium", sandbox: "workspace-write")
        saved.modelOptions["mimir.mode"] = .string("mode-plan")
        var picked = HarnessCatalog.selectingOption(mode, choiceId: "mode-build", in: saved)
        XCTAssertEqual(picked.modelOptions["mimir.mode"]?.stringValue, "mode-build")
        // A model change can arrive before its workflow-option metadata does.
        let pendingModel = ModelInfo(id: "provider/b", label: "B", description: nil, reasoningLevels: [])
        picked.modelOptions = HarnessCatalog.prunedOptions(picked.modelOptions, for: pendingModel, harness: "mimir")
        let request = SessionStore.runRequest(prompt: "build", chat: runChat(config: picked))
        let payload = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(request)) as? [String: Any])
        XCTAssertEqual((payload["modelOptions"] as? [String: Any])?["mimir.mode"] as? String, "mode-build")
        XCTAssertFalse(HarnessCatalog.preservesDefaultOption("other", harness: "mimir"))
        XCTAssertFalse(HarnessCatalog.preservesDefaultOption("mimir.mode", harness: "codex"))
    }


    @MainActor
    func testMimirModelPickCommitsModelAndPendingEffortOnce() {
        let original = ChatConfig(harness: "mimir", model: "provider/a",
                                  reasoning: "medium", sandbox: "workspace-write")
        let destination = ModelInfo(id: "provider/b", label: "B", description: nil, reasoningLevels: [])
        var committed = original
        var writes = 0
        let sheet = ModelPickerSheet(harness: .constant("mimir"), modelId: "provider/a",
                                     reasoning: "medium", onSelect: { modelId, effort in
            // Like ComposerView.writeConfig, this callback captures the old
            // chat value. One atomic call must replace BOTH fields together.
            committed = original
            committed.model = modelId
            committed.reasoning = effort
            writes += 1
        }, lockedHarness: true)
        sheet.select(harness: "mimir", model: destination)
        XCTAssertEqual(writes, 1)
        XCTAssertEqual(committed.model, "provider/b")
        XCTAssertEqual(committed.reasoning, "medium")
    }

    func testFailedMimirDetailsPreserveCatalogSelectionAndEffort() {
        let destination = ModelInfo(id: "provider/b", label: "B", description: nil, reasoningLevels: [])
        var catalog = ModelCatalogState()
        catalog.update([destination])
        catalog.update(nil, selectedModel: destination.id)
        XCTAssertEqual(catalog.models, [destination])
        XCTAssertNotNil(catalog.error)
        XCTAssertNil(catalog.verifiedModel)
        let selected = HarnessCatalog.selectedModel(in: catalog.models, id: destination.id, harness: "mimir")
        XCTAssertEqual(selected.id, destination.id)
        XCTAssertEqual(catalog.reasoning(for: selected, harness: "mimir", current: "medium"), "medium")
        // Even a failed cold load must not silently replace a remembered id
        // with nil and thereby run the agent's unrelated configured default.
        XCTAssertEqual(HarnessCatalog.selectedModel(in: [], id: destination.id, harness: "mimir").id,
                       destination.id)
    }

    func testMimirEmptyLadderIsUnknownUntilSelectedDetailsComplete() {
        let model = ModelInfo(id: "chatgpt/gpt-5.6-luna", label: "Luna", description: nil, reasoningLevels: [])
        var catalog = ModelCatalogState()
        catalog.update([model])
        XCTAssertEqual(catalog.reasoning(for: model, harness: "mimir", current: "medium"), "medium")
        catalog.update([model], selectedModel: model.id)
        XCTAssertNil(catalog.reasoning(for: model, harness: "mimir", current: "medium"))
        catalog.update(nil, selectedModel: "provider/other")
        XCTAssertEqual(catalog.verifiedModel, model.id, "failed details cannot verify another model")
        XCTAssertNotNil(catalog.error)
        catalog.update([model], selectedModel: model.id)
        XCTAssertNil(catalog.error)
    }


    func testMimirUsesItsOwnIdentityAndLiveModelCatalog() {
        XCTAssertEqual(HarnessCatalog.label(for: "mimir"), "Mimir")
        XCTAssertTrue(HarnessCatalog.models(for: "mimir").isEmpty)
        let model = HarnessCatalog.defaultModel(for: "mimir")
        XCTAssertEqual(model.id, "")
        XCTAssertEqual(model.label, "Configured model")
        XCTAssertTrue(model.reasoningLevels.isEmpty)
        XCTAssertFalse(HarnessCatalog.harnesses.contains { $0.id == "mimir" },
                       "Mimir availability must come from the run device")
        XCTAssertNotNil(UIImage(named: "MimirMark", in: Bundle(for: AppModel.self), compatibleWith: nil))
    }

    func testCodexFallbackStartsWithAstraAndExposesItsTraits() {
        let models = HarnessCatalog.models(for: "codex")
        let astra = models.first

        XCTAssertEqual(astra?.id, "gpt-6-astra")
        XCTAssertEqual(astra?.label, "GPT-6-Astra")
        XCTAssertEqual(astra?.reasoningLevels,
                       ["low", "medium", "high", "xhigh", "max", "ultra"])
        XCTAssertEqual(astra?.options.first?.id, "serviceTier")
        XCTAssertEqual(astra?.options.first?.choices.map(\.id), ["default", "fast"])
    }

    func testOffReasoningHasAnExplicitLabelWithoutChangingDefaults() {
        XCTAssertEqual(HarnessCatalog.reasoningLabel("off"), "Off")
        XCTAssertEqual(TraitPickerSheet.effortHint("off"), "Reasoning disabled")
        let model = ModelInfo(id: "chatgpt/gpt-5.6-luna", label: "Luna", description: nil,
                              reasoningLevels: ["off", "minimal", "low", "medium", "high", "xhigh", "max"])
        XCTAssertEqual(HarnessCatalog.defaultReasoning(for: model), "high")
        for harness in ["claude-code", "codex", "pi", "grok", "hermes", "opencode"] {
            XCTAssertTrue(HarnessCatalog.models(for: harness).allSatisfy { !$0.reasoningLevels.contains("off") })
        }
    }


    func testDefaultReasoningMatchesDesktopPreference() {
        let astra = HarnessCatalog.defaultModel(for: "codex")
        XCTAssertEqual(HarnessCatalog.defaultReasoning(for: astra), "high")

        let short = ModelInfo(id: "short", label: "Short", description: nil,
                              reasoningLevels: ["low", "medium"])
        XCTAssertEqual(HarnessCatalog.defaultReasoning(for: short), "medium")
    }

    func testChoiceFallsBackToAdvertisedDefault() {
        let option = HarnessCatalog.defaultModel(for: "codex").options[0]
        XCTAssertEqual(HarnessCatalog.selectedChoice(for: option, selectedId: "fast").label, "Fast")
        XCTAssertEqual(HarnessCatalog.selectedChoice(for: option, selectedId: "stale").id, "default")
    }
}
