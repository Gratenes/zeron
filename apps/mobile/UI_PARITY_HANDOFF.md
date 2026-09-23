# React Native GPUI-parity handoff

## Branch and scope

- Branch: `feat/mobile-ui-parity`
- Parent: `feat/mobile-expo` at `1cd343a0`
- App: `apps/mobile`
- Primary target: Android; iOS and web share the React Native UI where useful.

Build the product UI from the GPUI desktop/browser experience. The existing
SwiftUI iOS client is a protocol reference only; it is not a feature or visual
baseline.

Keep Tailcat transport work on the parent branch unless UI work exposes a
specific transport requirement. The current `App.tsx` remains a working native
transport probe and should be retained behind a temporary development route or
screen while the new app shell is built.

## Source of truth

- Scope inventory: `apps/mobile/docs/gpui-parity.md`
- GPUI shell: `crates/ui/src/shell.rs` and `crates/ui/src/shell/`
- Transcript: `crates/ui/src/transcript.rs` and `crates/ui/src/markdown/`
- Composer: `crates/ui/src/composer.rs`, `queue.rs`, `attachments.rs`
- Design system: `crates/ui/src/theme.rs`, `typography.rs`, `motion.rs`, `icons.rs`

Use the currently working browser/GPUI surface as the visual reference. Match
information architecture and interactions first; a narrow phone layout may
reflow panels, but must not silently remove GPUI features.

## Recommended first slice

1. Add fixture data and a development-only fixture mode; do not block UI work
   on live RPC.
2. Build tokens (color, spacing, type, radius, borders, icons) from GPUI.
3. Build the app shell: rail/navigation, session list, transcript, composer,
   responsive panel/drawer behavior.
4. Make transcript rows, markdown/code blocks, streaming state, and composer
   interactions feel correct with fixtures.
5. Replace fixtures incrementally after Tailcat/RPC proves pairing, document
   reads, unary calls, and streams.

Start with one coherent session screen. Do not make a collection of disconnected
mock screens or copy the reduced iOS layout.

## Current transport baseline

- Android bridge: `modules/my-module/android/src/main/java/ai/kratos/tailcat/KratosTailcatModule.kt`
- Go binding: `connectivity/tailcat/adapter.go`
- Android-safe interface discovery: `connectivity/tailcat/netmon_android.go`
- Existing transport handoff: `HANDOFF.md`

Android Tailcat still needs a successful device end-to-end retry. The UI branch
may proceed with fixtures, but live client integration is not proven until that
gate passes.

## Local loop

```sh
cd apps/mobile
npm install
npx tsc --noEmit
npx expo start --web       # fast layout iteration
npx expo start --dev-client --tunnel  # native JavaScript iteration
```

Kotlin, Go, Gradle, or Expo-module changes require rebuilding the Android APK.
Expo Go cannot load the native Tailcat module. See `HANDOFF.md` for the full
native build and release commands.

## Guardrails

- Do not modify `/home/vm/code/kratos/zeron`; it is the active web worktree.
- Do not delete the Tailcat probe until the real pairing/startup path replaces it.
- Avoid new UI libraries until built-in React Native primitives cannot meet a
  concrete parity requirement.
- Keep the UI domain model and transport adapter separate so fixtures and live
  RPC feed the same screens.
