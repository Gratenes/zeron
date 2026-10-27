# Android Auto

Drive your agents from the car: start work by voice, hear when an agent
finishes or needs you, and answer it — while your music keeps playing.

## Shape

A Car App Library app (`androidx.car.app`, messaging category) in the
Android app, under `sh.zeron.android.car`:

| Piece | Role |
| --- | --- |
| `CarAgents` | Watches the workspace for agents that finish, fail or wait for input; fetches the reply (`SessionHandle.lastReply`); owns the car orchestrator chat; sends replies. Car-independent. |
| `CarVoice` | Phone-local speech: listens through the car microphone (`CarAudioRecord`) into the phone's on-device recognizer, speaks with the phone's text-to-speech. Owns audio focus. |
| `ZeronCarAppService` | The car screens: agents list with Talk, an agent's latest report with Reply / Read / Stop, alerts when an agent reports. |

## Audio

Music is never stopped for the length of a session; Zeron only takes audio
focus for the moment it needs it:

| Moment | Focus | Usage | Music |
| --- | --- | --- | --- |
| Listening | `GAIN_TRANSIENT_EXCLUSIVE` | `ASSISTANCE_NAVIGATION_GUIDANCE` | paused, resumes after |
| Speaking | `GAIN_TRANSIENT_MAY_DUCK` | `ASSISTANCE_NAVIGATION_GUIDANCE` | ducked |

Navigation guidance is the stream Android Auto mixes over media. Car mode
never uses `MODE_IN_COMMUNICATION` (the call path pauses media and switches
Bluetooth to HFP). Players of spoken content (podcasts) pause rather than
duck; that's the player's choice.

## Voice modes

- **Orchestrator (default).** Speech becomes text on the phone and goes to a
  car orchestrator chat: an ordinary chat on the host (the new-session
  page's host and harness) told to use the Zeron MCP to create, message and
  check on chats, and to answer in one or two spoken sentences. Its reply is
  read aloud. No voice model, no extra cost.
- **Conversation.** The existing Codex realtime call (`VoiceController`),
  for a back-and-forth. It is a call: media pauses while it runs.

Replying to one agent skips the orchestrator: the words go straight into that
chat.

## Reports

An agent's report is its last reply, cleaned for speech (markdown, code and
links dropped) and cut to a couple of sentences; the agent screen shows more
and Read reads the whole first paragraph. Orchestrator-created chats report
like any other.

## Questions

A request's questions are asked one at a time (`QuestionFlow`), and all
the answers go in one `respondInput` at the end. That covers a report, Answer
on the agent screen, and Choices. By voice, each question is read with its
options ("Question 1 of 2. …") and the microphone opens. A spoken answer
picks the option it names ("the second one", "staging"); otherwise it's
sent as free text. A multi-select question takes several options ("staging
and production", "the first and the third", "all of them"). With more
than one question, the answers are read back ("Sending: …") before they
go. Silence or Cancel part-way sends nothing, and the question stays
open.

Choices shows one question at a time. A tap picks a single option.
Toggles plus Next pick several. The microphone icon answers the current
question by voice. Each next question replaces the screen rather than
being pushed on top of it, because the host allows only about five
template steps per task, and an in-place update with new rows counts as
a step too.

## Distribution

Android Auto only runs templated apps installed from Google Play in a real
car (an internal test track is enough); "Unknown sources" covers media and
notification apps only. Development uses the Desktop Head Unit
(`extras;google;auto`) against an emulator with Play services and Android
Auto, where adb-installed builds run.

## Testing

```sh
# emulator (Play image) with Android Auto installed, developer mode on and
# "Start head unit server" chosen in its overflow menu
adb forward tcp:5277 tcp:5277
~/Android/Sdk/extras/google/auto/desktop-head-unit
```

`mic begin` in the DHU console speaks through the computer's microphone.
Without one, point ALSA capture at a raw file (`pcm.micfile { type file;
slave.pcm "null"; file "/dev/null"; infile "<16 kHz s16 mono raw>" }`)
and run the DHU with that `.asoundrc`; `ffmpeg -f lavfi -i
"flite=text='…'"` makes the speech.

## Findings

- The phone's on-device recognizer (Android System Intelligence) accepts
  the car microphone through `EXTRA_AUDIO_SOURCE`, but doesn't endpoint
  it: `CarVoice` ends the stream after 1.2 s of quiet (`Endpointer`). Its
  final text arrives as a partial marked `final_result`, with an empty
  `onResults`.
- Without its language pack the recognizer fails with error 13;
  `triggerModelDownload` offers the download, and the other recognizer is
  tried meanwhile.
- Android Auto maps listening to `AUDIO_FOCUS_GAIN_TRANSIENT` and speaking
  to `AUDIO_FOCUS_GAIN_TRANSIENT_MAY_DUCK` on the head unit (`CAR.AUDIO.FOCUS`
  in logcat).
- Header action strips take one titled action and no primary one.

## Next

- Messaging notifications (reports on the phone and in cars without the app).
- Conversation mode (Codex realtime call) from the car is wired but untested.
