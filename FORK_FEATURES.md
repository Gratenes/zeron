# Fork features

Generated from `scripts/fork-features/registry.toml` by
`scripts/fork-features/check.py --md`. Edit the registry, not this file.

pc-combined is upstream `main` plus the features below. Upstream merges
(#1499 deleting `crates/ui`, the 0.3.8/0.3.9 composer refactor) have dropped or
broken fork features without a conflict, so the fork now guards them:

- **The registry** (this table): every feature, who knows it, the branch that
  carries it alone, its commits, and what fails if it goes.
- **`scripts/fork-features/check.py`**: markers (a call site, an RPC method, a
  setting key, a test's own name that must still be in the tree), the zgx pin,
  the Luau/cargo tests named here, and *survival* (how many of each feature's
  commit lines are still in the tree, against a recorded floor).
- **A patch queue, in the registry**: each feature keeps its own branch
  (`branch` below), and `check.py --risk` merges each one onto the new main with
  `git merge-tree` (nothing touched), so a conflict is named per feature before
  `pc-combined` is merged.

## The procedure

Merging main into pc-combined (whoever does it):

1. `git fetch origin && scripts/fork-features/check.py --risk`: the features
   whose files main changed or deleted, and the ones that would conflict.
2. `git merge origin/main`; resolve; for each conflicted or "DELETED upstream"
   feature, read its commits (`git log --oneline -- <file>`) and keep or port it.
3. `scripts/fork-features/check.py --survival --tests`: markers, pin, the Luau
   tests, and the survival floors. A failure is a feature the merge dropped or
   broke: restore it (or ask its owner chat), don't edit the registry to pass.
   Changing a marker, status or floor is a decision: say why in the commit and
   tell the owner.
4. Commit the merge. Then, before every build:

   `scripts/fork-features/check.py --gate --tests` (add `--rust` when the build
   is for the engine or the zgx app and there's time: it compiles)

   It fetches, fails if HEAD is behind `origin/main` (the standing rule: don't
   build), and fails on any feature gone. Don't stage a build that fails it.

Adding a feature (new fork commit, new fork branch): add a `[[feature]]` to the
registry in the same commit, with markers that only exist while it's wired in
and at least one test; run `check.py --bless` for its survival floor; run
`check.py --md`.

When upstream ships its own version of a fork feature: set `status =
"superseded"` with `why`, keep a marker on the upstream behaviour, and drop the
fork commits' survival floor (`--bless`).

## Features

| id | feature | owner chat | branch | commits | tests |
|---|---|---|---|---|---|
| `taskbar-badge` | The app icon's badge counts the chats waiting for you; Settings → Notifications has the switch and what it counts | Zeron Management (8a6897a4), built by 1f88cdc9 | `zgx/taskbar-badge-setting` | `f2c168ab0`, `acf9d27fc`, `be434f5bf` | cargo `zeron-zgx taskbar_badge_tests` |
| `taskbar-badge-linux` | Linux: the app names its desktop file (zeron.desktop), so a dock finds the entry that shows the badge | Zeron Management (8a6897a4) | `zgx/taskbar-badge` | `f19f5468e` | markers only |
| `zgx-pin` | The zgx pin is the Gratenes/zgx fork rev with the Windows badge DPI fix, hidden-window focus, terminal block elements and the self-clearing panic banner | Zeron Management (8a6897a4), built by 1f88cdc9 |  | `f60801034`, `995785670`, `8c68553fa` | markers only |
| `chat-account-picker` | A chat chooses its own Claude account in the composer's Account row ('This chat: <account>'); PinChatAccount over RPC | 553be3e3 | `claude-zgx-chat-account-picker` | `8e126da92`, `3c79e0dc1` | luau `a chat's pin is asked after`<br>luau `choosing an account pins this chat`<br>luau `choosing Automatic takes the pin back`<br>luau `a chat on automatic reads`<br>luau `a chat on automatic still shows the usage`<br>luau `a login that needs signing in again is shown signed out`<br>luau `no pin said for the chat`<br>luau `a chat on a Cloud machine chooses among`<br>cargo `zeron-zgx a_chats_account_pin_reads`<br>cargo `zeron-engine a_persons_choice_pins_the_chat` |
| `pin-fallback` | A chat pinned to a login that needs signing in moves to a working account instead of failing; the login folders keep each pin | 553be3e3 | `claude-pin-fallback` | `046abf8d5`, `f0d5d981e`, `e943975c8`, `ee44a6547` … | cargo `zeron-engine needs_signin`<br>cargo `zeron-engine a_chat_pinned_to_a_spent_login`<br>cargo `zeron-engine a_run_refused_its_login` |
| `claude-identity-guard` | A chat's CLI stamping its account over ~/.claude.json is not a login switch: the identity stands with its tokens, a login two saved accounts hold is saved into neither, and the live login's folder goes only when the token is its own | 553be3e3 |  | `4af52cdf9` | cargo `zeron-engine identity::tests` |
| `remote-catalog-relook` | A chat on another device keeps its Claude tab: an empty model list is no answer, the last good models stay, and a catalog short of an agent's models is looked at again (20 s to 5 min, bounded) while the composer stays there | 553be3e3 |  |  | cargo `zeron-zgx an_empty_model_list_is_no_answer`<br>cargo `zeron-zgx a_catalog_short_of_an_agent_the_device_offers`<br>cargo `zeron-zgx a_short_catalog_is_looked_at_again` |
| `providers-signed-out` | Settings → Providers marks a login that needs signing in as Signed out, dates its meters, and offers Sign in again (no switch) | 6d0212ed | `zgx-signed-out-dead-logins` | `ae2208beb`, `c1dc7b4db` | luau `a dead login is marked signed out`<br>luau `its Sign in again starts the agent's sign-in`<br>luau `its menu signs in again and offers no switch`<br>luau `a healthy saved login still offers its switch`<br>luau `login per provider it signs in`<br>cargo `zeron-zgx a_login_that_needs_signing_in_says_so_and_dates_its_meters` |
| `usage-meters-ratelimit` | Account usage meters: a rate-limited usage check says so (not 'out of quota'), kept meters are dated 'Usage as of …', the probe stops hammering a rate limit | 6d0212ed | `account-usage-rate-limit-pr` | `33c2cad2b`, `5653efb08` | luau `meters a failed re-check left are dated`<br>luau `says how old its figures are`<br>cargo `zeron-engine claude_rate_limit`<br>cargo `zeron-engine healthy_claude`<br>cargo `zeron-engine a_healthy_claude_reading`<br>cargo `zeron-zgx kept_meters_are_dated` |
| `send-run-signin` | A sign-in failure at run start offers the Log in button; a refused login relaunches the run once | 553be3e3 |  | `c1dc7b4db`, `13bf085b4`, `34a715042` | cargo `zeron-engine a_refused_` |
| `claude-rotation` | claude-rotation plugin: Claude chats rotate over the saved accounts with headroom (plugins-dev) | 553be3e3 (owns ~/.zeron-v2/plugins/claude-rotation: never touch it) |  | `3bd8e6cfd`, `ac8c29576` | markers only |
| `claude-200k` | The 200K Claude context window really limits the window (CLAUDE_CODE_DISABLE_1M_CONTEXT) | Zeron Management (8a6897a4) |  | `1936db2d6`, `64b7d1d74` | markers only |
| `composer-press` | Composer: a press on the draft keeps its own caret and selection (double-click selects a word); a press on the pill's hairline gives the draft the keys | Zeron Management (8a6897a4) | `fix/composer-double-click` | `4ef07a790`, `d4655b115` | luau `a click in the draft puts the caret there`<br>luau `a press anywhere under the draft's line` |
| `same-title-location` | Sidebar: chats that share a title say where they run | Zeron Management (8a6897a4) | `zgx/same-title-location` | `2d453fa93`, `44e2c17a9` | luau `same title:` |
| `activity-working` | Activity: a chat that is working stays in the list (the Working shelf; the default is on) | Zeron Management (8a6897a4) | `zgx/activity-working-visible` | `92fec5472`, `44e2c17a9` | luau `Activity: a chat that starts working folds into Working`<br>luau `Activity: Fold working chats into Working` |
| `sidebar-unreachable` | Sidebar: a chat whose device is offline wears a wifi-off glyph; a row's corner has one fixed width; side chats nest and fold | Zeron Management (8a6897a4) |  | `7f2ce9d6c`, `b747292c7`, `7edec9cd6`, `ed3e6c2ff` … | luau `every chat of an offline device says so`<br>luau `a row's unreachable glyph is left of its side chats' chevron`<br>luau `past its first five` |
| `terminal-gui-bridge` | zgx on Windows: a GUI exe's terminal tabs get a console for their bridge | Zeron Management (8a6897a4) | `zgx-terminal-gui-console` | `5b991f902` | cargo `zeron-zgx a_gui_exes_bridge_runs_under_cmd` |
| `terminal-warn-logs` | Terminal: stream and link failures are logged at warn, so a blank remote terminal leaves a trace **(SUPERSEDED: The panel.rs half (the SubscribeTerminal retry log) went with crates/ui (#1499): zgx's bridge (apps/zgx/src/terminal.rs) has no retry loop to log. The term-link half, where the failures are, stays.)** | Zeron Management (8a6897a4) | `terminal-failures-warn` | `3cf46a8ab` | markers only |
| `windows-zgx` | zgx on Windows: caption buttons in Zeron's ink, rounded corner kept maximized, mouse side buttons step the route history, microphones by friendly name, the engine's app-binary spawn, dev port override | Zeron Management (8a6897a4), built by 1f88cdc9 |  | `d507ed162`, `bd757db62`, `8823f2bf8`, `0ec695c77` … | markers only |
| `zeron-v2-template` | The zeron-v2 template (Zeron's v2 look) and its blessed base hashes: a v3 ds/screens change must be re-blessed there | Zeron Management (8a6897a4) |  | `0d52d32b6`, `a0f7d5a91`, `c43819b23` | markers only |
| `plugin-device-review` | Plugins: another of your verified devices may review and allow a plugin here (bound to that device, once); a declined plugin can be reviewed again | Zeron Management (8a6897a4) |  | `db04e8e91`, `238826b48`, `f43003adb`, `cb1a3ab60` … | markers only |
| `zgx-small-fixes` | zgx: a reply's table shares its width by its columns; the mention launcher says when its list couldn't be had; a plugin window reload that timed out is retried | Zeron Management (8a6897a4) |  | `abeb3b9bd`, `535fdeb43`, `2fa8e0c45` | markers only |
| `stall-watchdog` | Engine: stall watchdog, run logging, and the ListRuns status RPC | bd5f93ab | `engine-stall-watchdog` | `9db9af77e` | markers only |
| `stall-watchdog-credential` | Stall watchdog: local runs are judged too (not only Cloud), one agent per chat, automatic updates never hold new runs, staged run starts | bd5f93ab | `stall-watchdog-credential` | `391264fd5` | cargo `zeron-engine sessions::watchdog` |
| `agent-login-logs` | Agent sign-in is logged at INFO: start (with whether a URL and callback came back), a poll's changes, a failure to start or poll, and cancel | Zeron Management (8a6897a4) | `engine-agent-login-logs` | `84cbc22cc` | cargo `zeron-engine sign_in_poll_logging` |
| `zgx-onboarding-no-username` | Setup finishes on an edge without usernames (a self-hosted one answering /username with 404 skips the username and card steps), and a local engine follows a sign-in | Zeron Management (8a6897a4) | `zgx-onboarding-no-username` | `983a071c9` | luau `setup goes through its steps in order` |
| `registry-poison` | Registry sync: a refused batch is isolated, not replayed for ever; session rows are bounded and queued ops the room would refuse are shed | Zeron Management (8a6897a4) | `registry-poison-batch` | `9c9c8efb6`, `a8b11e76b` | markers only |
| `phone-tools` | Agents can use a phone: PhoneCall, phone_* MCP tools (SMS/RCS/MMS read+send+react+attachments+queue, unlock, files, install/grant/force-stop/settings, Shizuku probe, voicemails) | 895d268e | `phone-tools-engine` | `e4fe968f6`, `047fd198a`, `249155d78`, `f536689bc` … | cargo `zeron-mcp phone` |
| `device-files` | Device files: list, stat and copy between the account's devices over the relay, a phone's folders included (agents get device_files_list, device_file_stat, device_file_copy, device_file_copy_status) | 895d268e | `device-files-phone` | `e6a1a0215`, `fc0faab7e`, `5a38a6a77`, `dbe5955d3` … | markers only |
| `scheduled-messages` | schedule_message, list_schedules, cancel_schedule; repeats (cron with a timezone, every_seconds, until, max_runs); a one-minute floor, no daily cap | Zeron Management (8a6897a4) | `zeron/scheduled-messages` | `2abe55103`, `a46745c5c`, `c80d0ff5e` | markers only |
| `mcp-model-options` | MCP: list_models returns each model's options; create_chat and create_chats accept them | Zeron Management (8a6897a4) | `mcp-model-options` | `ed8639927`, `e9e97f85f` | markers only |
| `mcp-ambiguous-title` | MCP: a chat key that matches several chats says which chat is which | Zeron Management (8a6897a4) | `mcp/ambiguous-chat-title` | `744e7dd6a` | markers only |
