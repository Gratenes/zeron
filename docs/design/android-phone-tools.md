# Agents on the Android phone

Agents in Zeron can read and drive the user's Android phone (screen, apps, texts) with no PC
tethered and no wireless adb. Personal fork only (sideloaded): it never goes upstream.

## How it works

```
agent ── zeron mcp ── engine ──(device room, PhoneCall)── edge ──(device room)── phone
        phone_* tools   routes    target = the account's Android phone              host
```

- The phone joins **its own** device room as the host (`zeron_rpc::HostRelay`, as an engine
  does) and serves one method, `PhoneCall {tool, args, origin}`, to the account's other devices.
  `zeron-client::phone_host` carries it; Kotlin (`sh.zeron.android.phone`) does the work. There is
  no engine on the phone, and the phone writes its own device row (platform `android`,
  capability `phone-tools-v1`) so `list_devices` and the tools find it.
- The engine's MCP (`crates/mcp/src/tools/phone.rs`) offers `phone_*` tools; each is one
  `PhoneCall` to the account's only phone (or `device`). The engine dials the phone whatever its
  registry presence says (it rests its workspace sockets), waits up to 20 s for a phone that is
  away, and lets its link go 30 s after the last call.
- The host is started by the **AccessibilityService** (Android keeps it bound and restarts it
  after a kill or reboot): no foreground service, no permanent notification. A boot / package
  replaced receiver brings the link back in other cases.

## Tools

`phone_ping`, `phone_screen_read`, `phone_screenshot` (saved as a file on the agent's machine),
`phone_tap`, `phone_long_press`, `phone_type`, `phone_swipe`, `phone_scroll`, `phone_press`
(back/home/recents/shade/…), `phone_launch_app`, `phone_list_apps`, `phone_wait`,
`phone_sms_threads`, `phone_sms_read`, `phone_sms_send`, `phone_contacts_search`.

New texts can be passed to a chat (Settings → Agent phone access → Wake a chat on a new text),
only from numbers the user lists; the text goes in quoted and labelled as the sender's, not an
instruction.

## Safety (all enforced on the phone)

- Off by default. A master switch, six per-tool-group switches, Android permissions the user
  grants one by one.
- While an agent uses the phone: an overlay chip and a notification, both with **Stop**
  (refuses agents for 10 minutes and answers any pending question "no").
- A log of every call (tool, a redacted summary, the chat that asked, ok or refused). Never
  message text or what was typed, only lengths.
- Texts: a notification shows the whole text with Send / Don't send (setting
  `confirmSmsSend`, default on until the user decides); no answer in 60 s is a no.
- Never read or driven: Zeron itself, the package installer. Password fields read as redacted
  and are never typed into. Windows an app marks secure can't be screenshot.

## Battery

Only one socket stays open on an idle phone: the host's, pinging the room every `keepalive`
(default 60 s, Settings: 30 s / 1 min / 2 min). The room is told (`?keepalive=` on the join) and
keeps routing to it for 3 intervals (`hostLivenessMs` in `edge/src/device-room.ts`); the host's
own silence lease is 2.5 intervals. A headless start (accessibility bound, boot) rests the
workspace sockets (registry, previews, chats). The engine's own client link, which pings every
10 s, is closed 30 s after the last call.

### Measuring it on the real phone

`scripts/android/phone-battery.sh start` / `stop` (Android's own batterystats: level drop per
hour, Zeron's estimated mAh, radio-active time, wakeups). Same hours each time, screen off,
unplugged, a quiet network, at least 6 h (overnight is best); connect with wireless adb only to
start and stop.

| Run | Zeron | Question |
| --- | --- | --- |
| A | installed, agent access **off** | the baseline (the app's normal idle cost) |
| B | access on, accessibility on, keep-alive 60 s, no calls | what the link costs idle |
| C | as B, keep-alive 30 s, then 2 min | how much the interval is worth |
| D | as B, battery optimization exempt vs not | whether Doze defers the pings (watch for `hostConnected` flapping: `GET /device/<id>/status`) |
| E | as B on mobile data only | the radio tail cost, which is what a ping really costs |

Judge B − A: under ~1 %/h is invisible, over ~2 %/h means lengthen the keep-alive or add push-to-wake
and drop the socket. Also record, once, how long a call takes after the phone has been idle an hour
(the first `phone_ping` from an agent: Doze or an OEM killer shows up as a slow or failed first call).
