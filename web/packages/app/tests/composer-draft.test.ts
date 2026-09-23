import { describe, expect, it } from "vitest";
import type { ChatConfig, HarnessDescriptor, HarnessId, Model } from "@zeron/proto";
import type { DraftConfig } from "../src/lib/composer-actions";
import { SidebarStore } from "../src/lib/sidebar-store";
import {
  applyDraftUpdate,
  composerDefaults,
  defaultDraft,
  draftFromChat,
  draftsEqual,
  isHarnessLocked,
  reconcileDraftModel,
  rememberNoProject,
} from "../src/lib/composer-draft";
import type { StorageLike } from "../src/lib/engine-store";

function memoryStorage(): StorageLike {
  const map = new Map<string, string>();
  return {
    getItem: (key) => (map.has(key) ? map.get(key)! : null),
    setItem: (key, value) => void map.set(key, value),
    removeItem: (key) => void map.delete(key),
  };
}

function chat(overrides: Partial<{ config: ChatConfig | null }> = {}): Parameters<typeof draftFromChat>[0] {
  return {
    id: "chat-1",
    deviceId: "device-1",
    title: null,
    archived: false,
    cwd: "/Users/me/proj",
    branch: null,
    checkoutId: null,
    config: overrides.config !== undefined ? overrides.config : null,
    lastMessagePreview: null,
    lastMessageAt: null,
    createdAt: "2026-09-16T10:00:00Z",
  };
}

const HARNESSES: readonly HarnessDescriptor[] = [
  { id: "claude-code", name: "Claude Code", supportsSteering: true, steeringMode: "step-boundary", reasoningLevels: ["low", "medium", "high"], installed: true, enabled: true },
  { id: "codex", name: "Codex", supportsSteering: false, steeringMode: "turn-boundary", reasoningLevels: ["medium"], installed: true, enabled: true },
];

const MODELS: readonly Model[] = [
  { id: "sonnet", label: "Sonnet", reasoningLevels: ["low", "medium", "high"], options: [] },
];

describe("defaultDraft", () => {
  // Ticket 77 intentional correction: the old seed was the model's FIRST
  // level ("low") or a synthetic "medium", bypassing native default
  // selection. The native default is High when offered (else Medium, else
  // the first advertised level); an unresolved ladder keeps the remembered
  // preference verbatim instead of inventing a value.
  it("picks the first harness + first model + the native default reasoning + workspace-write sandbox", () => {
    const draft = defaultDraft(HARNESSES, MODELS);
    expect(draft.harness).toBe("claude-code");
    expect(draft.model).toBe("sonnet");
    expect(draft.reasoning).toBe("high");
    expect(draft.sandbox).toBe("workspace-write");
  });

  it("falls back to a sensible harness + null model when the catalog is empty", () => {
    const draft = defaultDraft([], []);
    expect(draft.harness).toBe("claude-code");
    expect(draft.model).toBeNull();
    // No effective ladder resolves: no synthetic value is invented.
    expect(draft.reasoning).toBeNull();
    expect(draft.sandbox).toBe("workspace-write");
  });

  it("keeps a remembered level the effective ladder offers (new-chat preference layer)", () => {
    expect(defaultDraft(HARNESSES, MODELS, "low").reasoning).toBe("low");
  });

  it("heals a remembered level the effective ladder excludes to the native default", () => {
    expect(defaultDraft(HARNESSES, MODELS, "ultra").reasoning).toBe("high");
  });

  it("retains the remembered level verbatim while no ladder has resolved", () => {
    expect(defaultDraft([], [], "low").reasoning).toBe("low");
  });

  it("resolves the default against the descriptor when the first model's list is empty", () => {
    const haiku: Model = { id: "haiku", label: "Haiku", reasoningLevels: [], options: [] };
    expect(defaultDraft(HARNESSES, [haiku], null).reasoning).toBe("high");
  });
});

describe("draftFromChat", () => {
  it("replays the persisted ChatConfig when one exists", () => {
    const persisted: ChatConfig = {
      harness: "codex",
      model: "gpt-5",
      reasoning: "medium",
      modelOptions: { mode: "fast" },
      sandbox: "read-only",
    };
    expect(draftFromChat(chat({ config: persisted }), HARNESSES, MODELS)).toEqual({
      harness: "codex",
      model: "gpt-5",
      reasoning: "medium",
      modelOptions: { mode: "fast" },
      sandbox: "read-only",
    });
  });

  it("falls back to defaults when ChatConfig is null", () => {
    expect(draftFromChat(chat(), HARNESSES, MODELS)).toEqual({
      harness: "claude-code",
      model: "sonnet",
      reasoning: "high",
      modelOptions: {},
      sandbox: "workspace-write",
    });
  });

  it("applies the remembered reasoning for a fresh chat (native new-chat precedence)", () => {
    expect(draftFromChat(chat(), HARNESSES, MODELS, "low").reasoning).toBe("low");
  });

  it("an established chat's persisted reasoning wins over the remembered level outright", () => {
    const persisted: ChatConfig = {
      harness: "codex",
      model: "gpt-5",
      reasoning: "medium",
      modelOptions: {},
      sandbox: "read-only",
    };
    expect(draftFromChat(chat({ config: persisted }), HARNESSES, MODELS, "low").reasoning).toBe("medium");
  });
});

describe("isHarnessLocked", () => {
  it("locks the harness when the chat row carries a ChatConfig", () => {
    expect(isHarnessLocked(chat({ config: { harness: "claude-code", model: null, reasoning: null, modelOptions: {}, sandbox: "workspace-write" } }))).toBe(true);
  });

  it("unlocks the harness for a fresh chat (no ChatConfig yet)", () => {
    expect(isHarnessLocked(chat({ config: null }))).toBe(false);
  });

  it("treats a null chat as unlocked (the engine is unavailable)", () => {
    expect(isHarnessLocked(null)).toBe(false);
  });
});

describe("draftsEqual", () => {
  it("matches structurally equal drafts", () => {
    expect(
      draftsEqual(
        { harness: "claude-code", model: "sonnet", reasoning: "low", modelOptions: {}, sandbox: "workspace-write" },
        { harness: "claude-code", model: "sonnet", reasoning: "low", modelOptions: {}, sandbox: "workspace-write" },
      ),
    ).toBe(true);
  });

  it("differs on any effective field", () => {
    expect(
      draftsEqual(
        { harness: "claude-code", model: "sonnet", reasoning: "low", modelOptions: {}, sandbox: "workspace-write" },
        { harness: "codex", model: "sonnet", reasoning: "low", modelOptions: {}, sandbox: "workspace-write" },
      ),
    ).toBe(false);
    expect(
      draftsEqual(
        { harness: "claude-code", model: "sonnet", reasoning: "low", modelOptions: {}, sandbox: "workspace-write" },
        { harness: "claude-code", model: "opus", reasoning: "low", modelOptions: {}, sandbox: "workspace-write" },
      ),
    ).toBe(false);
  });
});

describe("applyDraftUpdate", () => {
  const HAIKU: Model = { id: "haiku", label: "Haiku", reasoningLevels: [], options: [] };
  const SONNET: Model = { id: "sonnet", label: "Sonnet", reasoningLevels: ["low", "medium", "high"], options: [] };
  const OPUS: Model = { id: "opus", label: "Opus", reasoningLevels: ["low", "high"], options: [] };
  const GPT: Model = { id: "gpt-5", label: "GPT-5", reasoningLevels: [], options: [] };
  const CLAUDE: HarnessDescriptor = HARNESSES[0]!; // claude-code, [low, medium, high]
  const CODEX: HarnessDescriptor = HARNESSES[1]!; // codex, [medium]

  const resolvers = (
    byHarness: Record<string, readonly Model[]>,
    descriptors: readonly HarnessDescriptor[],
  ): {
    resolveModel: (harness: HarnessId, modelId: string | null) => Model | null;
    resolveDescriptor: (harness: HarnessId) => HarnessDescriptor | null;
  } => ({
    resolveModel: (harness, modelId) =>
      modelId === null ? null : (byHarness[harness] ?? []).find((model) => model.id === modelId) ?? null,
    resolveDescriptor: (harness) => descriptors.find((row) => row.id === harness) ?? null,
  });

  const baseDraft = (overrides: Partial<DraftConfig> = {}): DraftConfig => ({
    harness: "claude-code",
    model: "haiku",
    reasoning: "low",
    sandbox: "workspace-write",
    modelOptions: {},
    ...overrides,
  });

  it("keeps a descriptor-backed pick the empty model list cannot offer (the ticket-77 retention)", () => {
    const { resolveModel, resolveDescriptor } = resolvers({ "claude-code": [HAIKU] }, [CLAUDE]);
    const next = applyDraftUpdate(baseDraft(), { reasoning: "high" }, resolveModel, resolveDescriptor);
    expect(next.reasoning).toBe("high");
  });

  it("clamps against the model's own ladder, never a union with the descriptor's", () => {
    const { resolveModel, resolveDescriptor } = resolvers(
      { "claude-code": [OPUS] },
      [{ ...CLAUDE, reasoningLevels: ["medium", "max"] }],
    );
    // medium is descriptor-only: foreign to the model ladder [low, high],
    // heals to its native default (High).
    const next = applyDraftUpdate(baseDraft({ model: "opus" }), { reasoning: "medium" }, resolveModel, resolveDescriptor);
    expect(next.reasoning).toBe("high");
  });

  it("resolves the descriptor for the NEXT harness after a switch, not the one being left", () => {
    const { resolveModel, resolveDescriptor } = resolvers(
      { "claude-code": [HAIKU], codex: [GPT] },
      [CLAUDE, CODEX],
    );
    // low survives against claude-code's ladder; codex offers only medium.
    const next = applyDraftUpdate(
      baseDraft(),
      { harness: "codex", model: "gpt-5", reasoning: "low" },
      resolveModel,
      resolveDescriptor,
    );
    expect(next.harness).toBe("codex");
    expect(next.reasoning).toBe("medium");
  });

  it("retains the stored preference while the model and descriptor are both unavailable", () => {
    const { resolveModel, resolveDescriptor } = resolvers({}, []);
    const next = applyDraftUpdate(baseDraft(), { modelOptions: {} }, resolveModel, resolveDescriptor);
    expect(next.reasoning).toBe("low");
  });

  it("retains the stored preference when both lists are empty (no destructive nulling)", () => {
    const { resolveModel, resolveDescriptor } = resolvers(
      { "claude-code": [HAIKU] },
      [{ ...CLAUDE, reasoningLevels: [] }],
    );
    const next = applyDraftUpdate(baseDraft(), { modelOptions: {} }, resolveModel, resolveDescriptor);
    expect(next.reasoning).toBe("low");
  });

  it("keeps a still-offered level across a model change, and heals an excluded one to the native default", () => {
    const { resolveModel, resolveDescriptor } = resolvers({ "claude-code": [SONNET, OPUS] }, [CLAUDE]);
    expect(applyDraftUpdate(baseDraft({ model: "sonnet" }), { model: "sonnet" }, resolveModel, resolveDescriptor).reasoning).toBe("low");
    // opus offers [low, high]: low survives the switch.
    expect(applyDraftUpdate(baseDraft({ model: "sonnet" }), { model: "opus" }, resolveModel, resolveDescriptor).reasoning).toBe("low");
    // …but high on a [minimal]-only ladder heals to its first entry.
    const minimal: Model = { id: "basic", label: "Basic", reasoningLevels: ["minimal"], options: [] };
    const again = resolvers({ "claude-code": [minimal] }, [{ ...CLAUDE, reasoningLevels: [] }]);
    expect(applyDraftUpdate(baseDraft({ model: "basic", reasoning: "high" }), { modelOptions: {} }, again.resolveModel, again.resolveDescriptor).reasoning).toBe("minimal");
  });

  it("still drops option picks the new model does not offer", () => {
    const withOption: Model = {
      id: "sonnet",
      label: "Sonnet",
      reasoningLevels: ["low", "medium", "high"],
      options: [
        {
          id: "context",
          label: "Context",
          choices: [
            { id: "standard", label: "Standard" },
            { id: "1m", label: "1M" },
          ],
          defaultChoice: "standard",
        },
      ],
    };
    const { resolveModel, resolveDescriptor } = resolvers({ "claude-code": [withOption, HAIKU] }, [CLAUDE]);
    const next = applyDraftUpdate(
      baseDraft({ model: "sonnet", modelOptions: { context: "1m" } }),
      { model: "haiku" },
      resolveModel,
      resolveDescriptor,
    );
    expect(next.modelOptions).toEqual({});
    // haiku's empty list falls back to the descriptor: low stays.
    expect(next.reasoning).toBe("low");
  });
});

describe("reconcileDraftModel", () => {
  const HAIKU: Model = { id: "haiku", label: "Haiku", reasoningLevels: [], options: [] };
  const SONNET: Model = { id: "sonnet", label: "Sonnet", reasoningLevels: ["low", "medium", "high"], options: [] };
  const CLAUDE: HarnessDescriptor = HARNESSES[0]!;

  const baseDraft = (overrides: Partial<DraftConfig> = {}): DraftConfig => ({
    harness: "claude-code",
    model: "haiku",
    reasoning: "low",
    sandbox: "workspace-write",
    modelOptions: {},
    ...overrides,
  });

  it("keeps a still-offered fallback level through a catalog refresh — identity unchanged", () => {
    const current = baseDraft();
    // The effective ladder is the descriptor's; low is offered: no change.
    expect(reconcileDraftModel(current, [HAIKU], CLAUDE, null)).toBe(current);
  });

  it("heals an absent level to the native default once the effective ladder resolves", () => {
    const next = reconcileDraftModel(baseDraft({ reasoning: null }), [HAIKU], CLAUDE, null);
    expect(next.reasoning).toBe("high");
  });

  it("retains the preference (identity unchanged) while the effective ladder is empty", () => {
    const current = baseDraft({ reasoning: "low" });
    expect(reconcileDraftModel(current, [HAIKU], { ...CLAUDE, reasoningLevels: [] }, null)).toBe(current);
    expect(reconcileDraftModel(current, [HAIKU], null, null)).toBe(current);
  });

  it("seeds the first model and the effective default reasoning when the draft names none", () => {
    const next = reconcileDraftModel(baseDraft({ model: null, reasoning: null }), [HAIKU], CLAUDE, null);
    expect(next.model).toBe("haiku");
    expect(next.reasoning).toBe("high");
  });

  it("seeds the remembered model when the list still offers it, keeping an offered level", () => {
    const next = reconcileDraftModel(
      baseDraft({ model: null, reasoning: "low" }),
      [HAIKU, SONNET],
      CLAUDE,
      { id: "sonnet", label: "Sonnet" },
    );
    expect(next.model).toBe("sonnet");
    expect(next.reasoning).toBe("low");
  });

  it("ignores a remembered model the refreshed list no longer offers", () => {
    const next = reconcileDraftModel(
      baseDraft({ model: null, reasoning: null }),
      [HAIKU],
      CLAUDE,
      { id: "sonnet", label: "Sonnet" },
    );
    expect(next.model).toBe("haiku");
  });
});

describe("projectless_new_session_restores_opt_out_and_clears_sidebar_filter", () => {
  // shell.rs:9191, the web mirror (§2.4 / §3.5): picking "Don't work in a
  // project" on the canvas restores the opt-out target AND takes the
  // sidebar's space filter — "retaining a project filter would hide the
  // session on its first send" (shell.rs:1767-1774): a projectless row
  // carries no spaceId, and the active list is narrowed by the filter.
  it("the no-project pick persists the opt-out default and clears the filter", () => {
    const storage = memoryStorage();
    const sidebar = new SidebarStore({ storage });
    sidebar.setSpaceFilter("space-1");
    rememberNoProject("device-1", sidebar);
    expect(composerDefaults.getSnapshot().noProject).toBe(true);
    expect(composerDefaults.getSnapshot().project).toBe(null);
    expect(composerDefaults.getSnapshot().device).toBe("device-1");
    expect(sidebar.getSnapshot().spaceFilter).toBe(null);
    // The clear persists through the ui-settings store, so it survives a
    // refresh.
    expect(new SidebarStore({ storage }).getSnapshot().spaceFilter).toBe(null);
  });
});
