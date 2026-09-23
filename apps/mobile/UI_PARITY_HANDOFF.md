# React Native GPUI parity handoff

## Branch and scope

- Branch: `feat/mobile-ui-parity`
- App: `apps/mobile`
- Primary target: Android; shared React Native UI also supports iOS and web where the native transport is available.
- Product reference: the existing GPUI desktop/browser UI. The SwiftUI client is a protocol reference only.

This branch contains the actual Expo app. The old Tailcat test screen has been replaced. Pairing and all live app data use Tailcat and authenticated ControlRpc; there is no sample connection route or fixture mode.

## Implemented vertical slice

- Signed peer pairing, saved pairing restoration, host engine discovery, and authenticated Tailcat ControlRpc.
- GPUI-inspired responsive shell, projects, sessions, transcript and composer with live queue controls.
- Streaming document updates, markdown/code and tool rows.
- Live folder/project creation, files, terminal, changes and Git history panels.
- Host selection and disconnect in settings.

See `docs/gpui-parity.md` for the full target inventory. This branch is still an initial implementation; browser preview, richer settings, attachments and other GPUI interactions remain.

## Source of truth

- Shell: `crates/ui/src/shell.rs` and `crates/ui/src/shell/`
- Transcript: `crates/ui/src/transcript.rs` and `crates/ui/src/markdown/`
- Composer: `crates/ui/src/composer.rs`, `queue.rs`, `attachments.rs`
- Design system: `crates/ui/src/theme.rs`, `typography.rs`, `motion.rs`, `icons.rs`
- Transport: `apps/mobile/src/connection.ts`, `apps/mobile/modules/my-module/`, `connectivity/tailcat/`

## Validation and next work

TypeScript, Android JS export, native Android build, and transport tests pass locally. A successful signed pairing and ControlRpc session on an attached Android device still needs verification. The earlier Tailcat proof did not cover the new signed pairing/RPC path.

Continue with device pairing and data round trips, then expand the remaining GPUI surfaces. Keep changes in small reviewable commits and do not modify `/home/vm/code/kratos/zeron` from this worktree.
