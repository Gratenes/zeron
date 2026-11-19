# Pi custom OpenAI-compatible providers

Branch: `pi-custom-provider` (from `android-usage-provider` @ 2aff3db0, which already contains `origin/main`).
Worktree: `/home/vm/code/comet/zeron-pi-provider`. Personal build only; not going upstream.

## Status (2026-10-21)

Built and tested:
- **Engine:** `crates/engine/src/agent_accounts/pi_providers.rs`, with 10 unit tests in `pi_providers/tests.rs`.
  - `stores::merge_json` is a generalised `merge_json_entry` (the existing accounts tests still pass).
- **Proto:** `PiProvider`, `PiProviderModel`, `PiKeySource` and `SavePiProvider` in `crates/proto/src/entities.rs`.
- **RPC:** `ListPiProviders`, `SavePiProvider`, `RemovePiProvider` and `DiscoverPiProviderModels`.
  - They are forwardable to the host device; save and remove also retire voice.
  - Round-trip test in `crates/engine/tests/pi_providers_rpc.rs`, which points `PI_CODING_AGENT_DIR` at a tempdir.
- **Client:** wrappers in `crates/client/src/agent_accounts.rs`.
- **Android:** `ui/PiProviders.kt`, shown under Pi on Settings → device → Accounts. Calls go through
  the untyped `hostCall` passthrough, so the phone needs **no FFI/native changes**.

Changed from the plan below:
- **No uniffi FFI.** Android already speaks raw RPC via `AppModel.hostCall`.
- **No client catalog invalidation.** `Client::list_models` always asks the engine live first and
  only falls back to the disk cache on failure. The engine's catalog is keyed on the
  `models.json` hash, so a save picks up new models with no extra plumbing.
- **Key order.** The workspace `serde_json` has no `preserve_order` feature, so a rewrite sorts
  `models.json` keys alphabetically. The content is preserved. Turning `preserve_order` on
  workspace-wide wasn't worth the risk.
- **Pi version.** The host's Pi was 0.84.2, which the engine rejected; that is why Pi only ever
  showed "Pi default". Upgraded to **1.3.0** (`npm i -g @earendil-works/pi-coding-agent@1.3.0`
  under nvm v23.11.1). Its `models.json` format is unchanged, and the 5 hand-written providers
  now list.
- Desktop UI is not done (optional).

## Goal

From Zeron (Android first, desktop optional), add/edit/remove a Pi provider with a
**name, base URL, API key, and model list**. Its models then appear in the model picker
for Pi chats on that device.

## How it works

Pi already supports this natively through `{agent_dir}/models.json`:

```json
{ "providers": { "<name>": {
    "baseUrl": "https://api.example.com/v1",
    "api": "openai-completions",
    "apiKey": "sk-…",
    "models": [{ "id": "model-a" }, { "id": "model-b", "name": "Model B" }]
} } }
```

- Models only show up in `get_available_models` if auth resolves. A literal `apiKey` is enough.
- Pi reloads the file; it never writes or locks `models.json`.
- Zeron's Pi model cache is keyed on a hash of `models.json` (`harness/src/model_context.rs`).
  So does the engine disk catalog (`engine/src/model_catalogs.rs`, path is `{hash}.json`).
  **Any edit already invalidates the catalog.** No new refresh plumbing is needed in the engine.

So the feature is: **a safe editor for `models.json`, plus RPC and UI.** No account slots and
no `auth.json` changes. `models.json` is the single source of truth.

Your real `~/.pi/agent/models.json` already has 5 hand-written providers in this shape. The
editor must preserve them, and preserve any field it doesn't understand.

## Decisions (defaults chosen; change if you disagree)

1. **Surface is a separate "Custom providers" section, not `AgentAccount` rows.** Account rows
   carry slot/usage/switch semantics that don't apply here. Separate types keep the existing
   accounts code untouched.
2. **List every provider in `models.json`, including hand-written ones.** Each can be edited
   and removed. Entries using `!command` or `$ENV` keys show "key from command/env" and keep
   that value unless you enter a new key.
3. **Create form supports `openai-completions` (default) and `openai-responses`.** Existing
   `anthropic-messages` entries are listed, editable, and keep their `api`.
4. **URL rules: user-entered, so any host is allowed.** `https` anywhere. `http` is also
   allowed, for LAN/local servers. Ports and paths (`/v1`) are allowed. Rejected: userinfo,
   query, fragment, anything other than http/https. This deliberately does *not* go through
   `trusted_base`/`HostPolicy`, which exist to distrust URLs coming *from credential files*.
5. **The key never leaves the engine.** RPC responses carry only `keyTail` (last 4) and a
   `keySource` (`literal | env | command | none`).
6. **Models come from `GET {baseUrl}/models`** (OpenAI shape `{data:[{id}]}`) via a "Fetch
   models" action. The user picks from the result. A manual comma/newline list is the fallback
   when the endpoint has no `/models`.
7. **Name rules:** `[a-z0-9][a-z0-9._-]{0,63}`. Must not collide with Pi built-ins
   (`openai`, `anthropic`, `openai-codex`, `github-copilot`, `google`, `google-gemini-cli`,
   `openrouter`, `groq`, `xai`, …). Pi would treat a built-in name as an *override*, which is
   confusing.

## Engine (`crates/engine`)

New module `src/pi_providers.rs` (kept out of the 5k-line `agent_accounts.rs`):

- `PiProviders::new(agent_dir: PathBuf)`. Built in `lib.rs` from `AgentAccountsConfig.pi_agent_dir`,
  so it follows the same `PI_CODING_AGENT_DIR` rule. Tests use `isolated(root)`.
- `list() -> Vec<PiCustomProvider>`: reads `models.json`. A missing file means an empty list.
  If the file doesn't parse, return an error; never overwrite it.
- `save(input: SavePiProvider) -> Vec<PiCustomProvider>`: validates name/URL/models. Then:
  - Merge into `providers.<name>`, keeping unknown keys on that entry and every other provider.
  - Keep the existing `apiKey` when `input.api_key` is `None`.
  - Handle `previous_name` for renames.
- `remove(name) -> Vec<PiCustomProvider>`.
- `discover_models(base_url, api_key | name) -> Vec<String>`: `GET {base}/models` with Bearer,
  10s timeout, cap 500 ids. With only `name`, use the stored key so the client never needs it.
  Literal keys only; `$ENV`/`!command` → "enter the key to fetch models".
- **Writes:**
  - Add a nested-path variant beside `stores::merge_json_entry` (it only replaces top-level
    keys). Better: generalise it to take a `FnOnce(&mut Map)` mutator. Reuse its lock,
    re-read/compare/retry and `stage_file_atomic(secret=true)` (0600).
  - Lock with `StoreLock::File("models.json.zeron-lock")`. Pi never locks this file, so this
    only serialises Zeron writers.
  - Pretty-print with 2-space indent. The file is hand-edited, so keep it readable.
- Errors go through `EngineError` with user-facing messages ("models.json isn't valid JSON —
  fix or move it first").

## Proto + RPC

`crates/proto/src/entities.rs`:

```rust
pub struct PiCustomProvider { name, base_url, api: String, key_tail: Option<String>,
                              key_source: PiKeySource, models: Vec<PiProviderModel> }
pub struct PiProviderModel { id: String, name: Option<String> }
pub enum PiKeySource { Literal, Env, Command, None }
```

`crates/rpc/src/lib.rs` method strings:
- `ListPiProviders`
- `SavePiProvider { name, previousName?, baseUrl, api, apiKey?, models }`
- `RemovePiProvider { name }`
- `DiscoverPiProviderModels { baseUrl, apiKey?, name? }`

`crates/engine/src/rpc.rs`: param structs and handlers. Add them to the **forwardable** list
next to the accounts methods (`rpc.rs:~1663`), so a phone can manage the provider on the
device that runs Pi. Add them to the voice-retired list too; voice shouldn't edit keys.

## Client + FFI

- `crates/client/src/agent_accounts.rs` (or new `pi_providers.rs`): `list/save/remove/discover`
  wrappers over the device RPC.
- After save/remove, drop that device's Pi entry in `catalog::DiskCatalog` and re-run
  `list_models`. The engine hash changes on its own, but the client's disk catalog might
  otherwise serve stale models until the next refresh.
- `crates/mobile/src/client_ffi/accounts.rs`: uniffi records + 4 async methods.

## Android (`apps/android/.../ui/`)

In `AgentAccountsScreen`, when the harness is Pi, add a **Custom providers** section below the
accounts:

- One row per provider: name, host, `•••• tail`, model count. Tap to edit, overflow menu to remove.
- "Add custom provider" opens a dialog, a Material3 `AlertDialog` like `AgentLoginDialog`:
  - Fields: Name, Base URL, API key (`PasswordVisualTransformation`; empty while editing means "keep").
  - API type dropdown.
  - "Fetch models" button that fills a checklist.
  - Manual models text field as fallback.
  - Save.
- Inline error text from the engine message. After save, refresh the list and invalidate the
  model list. The Composer/NewSession pickers re-query.

## Desktop (optional, after Android)

In `ui/src/settings/accounts.rs` `render_embedded_provider` for Pi, use the same section.
Reuse the rename-device dialog pattern (`settings/devices.rs:90-221`) with several
`ComposerInput`s. `ComposerInput` has no masked mode, so either add a simple `masked` flag or
show the key field only while it is empty.

## Tests

- **Engine unit tests** (tempdir, `isolated`):
  - Save into a missing file.
  - Save keeps other providers and unknown fields, byte-for-byte for the untouched entries.
  - Edit without a key keeps the key. Rename. Remove.
  - Invalid JSON gets refused and the file is unchanged.
  - Name/URL validation table. File mode is 0600.
  - A concurrent writer is blocked by the zeron lock.
- **Discover:** `MockServer` from `provider_tests.rs` serving `/v1/models`. Check the Bearer
  header and the 401 → message path.
- **RPC round-trip** in `engine/tests` (list → save → list → remove). Check that the key is
  absent from every response.
- **Harness:** `model_context` key changes after a save. This is already covered generically.
  Add one case that writes through `PiProviders`.

## Steps

1. Generalise `merge_json_entry` to a mutator, plus `PiProviders` with list/save/remove and unit tests.
2. Discovery + mock-server test.
3. Proto types, RPC methods, forwarding, RPC round-trip test.
4. Client wrappers + disk-catalog invalidation, then FFI.
5. Android section + dialog. Manual test against your existing `models.json`; back it up first.
6. Build and update your engine (see below), then add a provider from the phone and pick its
   model in a Pi chat.
7. (Optional) desktop UI.

## End-to-end test target: Omni router

Your friend's router is already configured in mimir.

- **Config:** `~/.mimir/agent/models.json` → `custom-omni`.
- **Key:** `~/.mimir/agent/auth.json` → `custom-omni.key` (`type: api_key`).
- **Base URL:** `https://omni.syek.tech/v1`. `GET /models` returns 439 ids in the OpenAI shape,
  so it works for "Fetch models".
- **Model:** `dashscope-intl/deepseek-v4.1-flash-1017`, the newest DeepSeek 4.1. The other 4.1
  ids are `dashscope-intl/deepseek-v4.1-flash`, `dashscope-cn/deepseek-v4.1-flash` and
  `dashscope-cn/vanchin/deepseek-v4.1-flash`.
- **Verified 2026-10-21:** `/chat/completions` with that model replied `pong`.

Mimir's file format is **not** Pi's. Its `models` is an object map and its `api` is
`openai-chat-completions`, with the key kept separately in `auth.json`. So the test entry is
written fresh in Pi's format, not copied:

```json
"omni": { "baseUrl": "https://omni.syek.tech/v1", "api": "openai-completions",
          "apiKey": "<omni key>",
          "models": [{ "id": "dashscope-intl/deepseek-v4.1-flash-1017", "name": "DeepSeek V4.1 Flash (1017)" }] }
```

**Acceptance:**
1. Add "omni" from the phone. Fetch models lists the 439 ids; select only the DeepSeek 4.1 entry.
2. `omni/dashscope-intl/deepseek-v4.1-flash-1017` shows up in the Pi model picker.
3. A Pi chat on that model completes a turn.

Slashes in model ids are safe. Zeron matches the whole `provider/id` string against Pi's list
(`harness/src/pi/mod.rs:577`). It then sends `provider` and `modelId` as separate fields, and
resume does the same (`sessions.rs:120`). Nothing splits on `/`.

## Updating your engine

Install the engine built from this worktree on the device that runs Pi, then restart it.
The phone build needs the new FFI too. Per your usual flow, redeploy staging only if the
relay/worker side changes; this feature shouldn't touch it.

## Watch-outs

- **Pi version.** The globally installed Pi is 0.84.2, which is below the harness minimum
  (0.85.1), and it isn't on PATH. Check which Pi your engine actually runs (`PI_EXECUTABLE`) so
  the model catalog isn't failing for an unrelated reason.
- **Plaintext keys.** `models.json` holds plaintext keys, which is Pi's own design. Zeron
  keeps the file 0600 and never echoes the key.
- **Probe timing.** The model probe takes up to about 65s on a slow endpoint (it calls
  `set_model` for each model). A provider with hundreds of models makes the picker slow, so
  the checklist should default to *none selected* rather than all.
