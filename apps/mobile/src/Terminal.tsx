import { useEffect, useRef, useState } from 'react';
import { ActivityIndicator, Pressable, ScrollView, StyleSheet, TextInput, View, type LayoutChangeEvent } from 'react-native';
import { Text } from './AppText';
import { utf8Decode, utf8Encode } from './connection';
import { colors, radius, spacing, typography } from './theme';

type Props = {
  owner: object;
  call: (method: string, params: Record<string, unknown>) => Promise<unknown>;
  subscribe: (method: string, params: Record<string, unknown>, onItem: (item: unknown) => void, onError?: (error: Error) => void) => Promise<() => void>;
  chatId: string;
  targetDeviceId?: string | null;
  cwd?: string | null;
};

type Session = { id: string; cwd: string; shell: string };
type Phase = 'opening' | 'ready' | 'reconnecting' | 'exited' | 'closed' | 'error';
type Active = { id: string; cancelled: boolean; cancelStream?: () => void; retry?: ReturnType<typeof setTimeout> };
type StoredTerminal = { session: Promise<Session>; output: Output; carry: Uint8Array; lastSeq: number; exited: boolean };

const terminals = new WeakMap<object, Map<string, StoredTerminal>>();

const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';

function encodeBase64(text: string): string {
  const bytes = utf8Encode(text);
  let result = '';
  for (let i = 0; i < bytes.length; i += 3) {
    const value = (bytes[i] << 16) | ((bytes[i + 1] ?? 0) << 8) | (bytes[i + 2] ?? 0);
    result += alphabet[(value >>> 18) & 63] + alphabet[(value >>> 12) & 63] +
      (i + 1 < bytes.length ? alphabet[(value >>> 6) & 63] : '=') +
      (i + 2 < bytes.length ? alphabet[value & 63] : '=');
  }
  return result;
}

function decodeBase64(value: string): Uint8Array {
  const data = value.replace(/\s/g, '');
  if (data.length % 4 === 1) throw new Error('Invalid terminal output');
  const bytes: number[] = [];
  for (let i = 0; i < data.length; i += 4) {
    const block = data.slice(i, i + 4);
    const digits = [...block].map(char => char === '=' ? 0 : alphabet.indexOf(char));
    if (digits.some(digit => digit < 0)) throw new Error('Invalid terminal output');
    const value = ((digits[0] ?? 0) << 18) | ((digits[1] ?? 0) << 12) | ((digits[2] ?? 0) << 6) | (digits[3] ?? 0);
    bytes.push((value >>> 16) & 255);
    if (block.length > 2 && block[2] !== '=') bytes.push((value >>> 8) & 255);
    if (block.length > 3 && block[3] !== '=') bytes.push(value & 255);
  }
  return new Uint8Array(bytes);
}

function decodeChunk(bytes: Uint8Array, carry: Uint8Array): { text: string; carry: Uint8Array } {
  const joined = new Uint8Array(carry.length + bytes.length);
  joined.set(carry);
  joined.set(bytes, carry.length);
  let end = joined.length;
  let lead = end - 1;
  while (lead >= 0 && (joined[lead] & 0xc0) === 0x80) lead--;
  if (lead >= 0) {
    const first = joined[lead];
    const needed = first < 0x80 ? 1 : first < 0xe0 ? 2 : first < 0xf0 ? 3 : 4;
    if (end - lead < needed) end = lead;
  }
  let text = '';
  try { text = utf8Decode(joined.subarray(0, end)); }
  catch { text = Array.from(joined.subarray(0, end), byte => byte < 0x80 ? String.fromCharCode(byte) : '�').join(''); }
  return { text, carry: joined.slice(end) };
}

// A line-oriented VT output view: handles shell prompts and line editing while
// ignoring color and title sequences. Full-screen TUIs need a cell-grid renderer.
function nextCharacter(text: string, index: number, end = text.length): number {
  if (index >= end) return end;
  return Math.min(end, index + ((text.codePointAt(index) ?? 0) > 0xffff ? 2 : 1));
}

function previousCharacter(text: string, index: number, start = 0): number {
  if (index <= start) return start;
  const last = text.charCodeAt(index - 1);
  const before = text.charCodeAt(index - 2);
  return last >= 0xdc00 && last <= 0xdfff && before >= 0xd800 && before <= 0xdbff && index - 2 >= start ? index - 2 : index - 1;
}

class Output {
  text = '';
  cursor = 0;
  mode: 'normal' | 'escape' | 'csi' | 'osc' = 'normal';
  sequence = '';
  oscEscape = false;

  feed(chunk: string): string {
    for (const char of chunk) {
      if (this.mode === 'osc') {
        if (char === '\x07' || (this.oscEscape && char === '\\')) this.mode = 'normal';
        this.oscEscape = char === '\x1b';
        continue;
      }
      if (this.mode === 'escape') {
        this.mode = char === '[' ? 'csi' : char === ']' ? 'osc' : 'normal';
        this.sequence = '';
        continue;
      }
      if (this.mode === 'csi') {
        if (char >= '@' && char <= '~') {
          this.control(char);
          this.mode = 'normal';
        } else if (this.sequence.length < 32) this.sequence += char;
        else this.mode = 'normal';
        continue;
      }
      if (char === '\x1b') { this.mode = 'escape'; continue; }
      if (char === '\r') { this.cursor = this.text.lastIndexOf('\n', this.cursor - 1) + 1; continue; }
      if (char === '\b') { this.cursor = previousCharacter(this.text, this.cursor, this.text.lastIndexOf('\n', this.cursor - 1) + 1); continue; }
      if (char === '\n') {
        const end = this.text.indexOf('\n', this.cursor);
        this.cursor = end < 0 ? this.text.length : end;
        this.text = this.text.slice(0, this.cursor) + '\n' + this.text.slice(this.cursor + (end < 0 ? 0 : 1));
        this.cursor++;
        continue;
      }
      if (char < ' ') continue;
      const atEnd = this.cursor >= this.text.length || this.text[this.cursor] === '\n';
      this.text = this.text.slice(0, this.cursor) + char + this.text.slice(atEnd ? this.cursor : nextCharacter(this.text, this.cursor));
      this.cursor += char.length;
    }
    if (this.text.length > 80_000) {
      const cut = nextCharacter(this.text, this.text.length - 80_001);
      this.text = this.text.slice(cut);
      this.cursor = Math.max(0, this.cursor - cut);
    }
    return this.text;
  }

  private control(code: string) {
    const amount = Math.max(1, Number.parseInt(this.sequence, 10) || 1);
    const steps = Math.min(amount, this.text.length + 1);
    const lineStart = this.text.lastIndexOf('\n', this.cursor - 1) + 1;
    const lineEnd = this.text.indexOf('\n', this.cursor);
    const end = lineEnd < 0 ? this.text.length : lineEnd;
    if (code === 'K') {
      if (this.sequence === '2') {
        this.text = this.text.slice(0, lineStart) + this.text.slice(end);
        this.cursor = lineStart;
      } else this.text = this.text.slice(0, this.cursor) + this.text.slice(end);
    } else if (code === 'D') {
      for (let i = 0; i < steps; i++) this.cursor = previousCharacter(this.text, this.cursor, lineStart);
    } else if (code === 'C') {
      for (let i = 0; i < steps; i++) this.cursor = nextCharacter(this.text, this.cursor, end);
    } else if (code === 'G') {
      this.cursor = lineStart;
      for (let i = 1; i < steps; i++) this.cursor = nextCharacter(this.text, this.cursor, end);
    }
  }
}

function sessionFrom(value: unknown): Session {
  if (!value || typeof value !== 'object') throw new Error('Invalid terminal session');
  const session = value as Record<string, unknown>;
  if (typeof session.id !== 'string' || !session.id || typeof session.cwd !== 'string' || typeof session.shell !== 'string') {
    throw new Error('Invalid terminal session');
  }
  return session as Session;
}

function message(error: unknown): string { return error instanceof Error ? error.message : 'Terminal request failed'; }

export function Terminal({ owner, call, subscribe, chatId, targetDeviceId, cwd }: Props) {
  const callRef = useRef(call);
  const subscribeRef = useRef(subscribe);
  callRef.current = call;
  subscribeRef.current = subscribe;
  const active = useRef<Active | null>(null);
  const generation = useRef(0);
  const size = useRef({ cols: 80, rows: 24 });
  const scroll = useRef<ScrollView>(null);
  const follow = useRef(true);
  const [session, setSession] = useState<Session | null>(null);
  const [phase, setPhase] = useState<Phase>('opening');
  const [display, setDisplay] = useState('');
  const [draft, setDraft] = useState('');
  const [error, setError] = useState('');
  const [sending, setSending] = useState(false);
  const [restart, setRestart] = useState(0);

  const target = targetDeviceId ? { targetDeviceId } : {};
  const close = async (id: string) => {
    try { await callRef.current('CloseTerminal', { terminalId: id, ...target }); }
    catch (cause) { setError(message(cause)); }
  };

  useEffect(() => {
    const current = ++generation.current;
    let disposed = false;
    setSession(null); setPhase('opening'); setDisplay(''); setDraft(''); setError(''); setSending(false);
    const key = JSON.stringify([chatId, targetDeviceId ?? null]);
    let terminalMap = terminals.get(owner);
    if (!terminalMap) { terminalMap = new Map(); terminals.set(owner, terminalMap); }
    let stored = terminalMap.get(key);
    if (chatId && !stored) {
      const requestedSize = { ...size.current };
      stored = {
        session: (async () => sessionFrom(await call('OpenTerminal', { chatId, ...requestedSize, ...target })))(),
        output: new Output(), carry: new Uint8Array(), lastSeq: 0, exited: false,
      };
      terminalMap.set(key, stored);
      const opening = stored;
      void opening.session.catch(() => { if (terminalMap.get(key) === opening) terminalMap.delete(key); });
    }
    const terminalState = stored;

    const attach = async (id: string) => {
      const terminal = active.current;
      if (!terminalState || !terminal || terminal.id !== id || terminal.cancelled || disposed) return;
      const failed = (cause: Error) => {
        if (disposed || terminal.cancelled || current !== generation.current) return;
        const detail = message(cause);
        if (detail.includes('Terminal not found')) {
          terminal.cancelled = true;
          terminal.cancelStream?.();
          if (terminalMap.get(key) === terminalState) terminalMap.delete(key);
          active.current = null;
          setPhase('error'); setError(detail);
          return;
        }
        setPhase('reconnecting'); setError(detail);
        terminal.retry = setTimeout(() => { void attach(id); }, 1200);
      };
      try {
        const cancel = await subscribeRef.current('SubscribeTerminal', { terminalId: id, afterSeq: terminalState.lastSeq, ...target }, item => {
          if (disposed || terminal.cancelled || current !== generation.current || !item || typeof item !== 'object') return;
          const event = item as Record<string, unknown>;
          if (typeof event.seq !== 'number' || event.seq <= terminalState.lastSeq) return;
          terminalState.lastSeq = event.seq;
          if (event.type === 'data' && typeof event.data === 'string') {
            try {
              const chunk = decodeChunk(decodeBase64(event.data), terminalState.carry);
              terminalState.carry = chunk.carry;
              setDisplay(terminalState.output.feed(chunk.text));
            }
            catch (cause) { setError(message(cause)); }
          } else if (event.type === 'exit') {
            terminalState.exited = true;
            terminal.cancelled = true;
            terminal.cancelStream?.();
            setPhase('exited');
            setDisplay(terminalState.output.feed(`\n[process exited ${typeof event.exitCode === 'number' ? event.exitCode : '?'}]\n`));
          }
        }, failed);
        if (disposed || terminal.cancelled || current !== generation.current) cancel();
        else { terminal.cancelStream = cancel; setPhase('ready'); setError(''); }
      } catch (cause) {
        failed(cause as Error);
      }
    };

    if (terminalState) void (async () => {
      try {
        const opened = await terminalState.session;
        if (disposed || current !== generation.current) return;
        setSession(opened); setDisplay(terminalState.output.text);
        if (terminalState.exited) { setPhase('exited'); return; }
        active.current = { id: opened.id, cancelled: false };
        setPhase('ready');
        if (size.current.cols !== 80 || size.current.rows !== 24) {
          void callRef.current('ResizeTerminal', { terminalId: opened.id, ...size.current, ...target }).catch(cause => setError(message(cause)));
        }
        void attach(opened.id);
      } catch (cause) {
        if (!disposed && current === generation.current) { setPhase('error'); setError(message(cause)); }
      }
    })();
    else { setPhase('error'); setError('Select a session to open a terminal.'); }

    return () => {
      disposed = true;
      generation.current++;
      const terminal = active.current;
      if (terminal) {
        terminal.cancelled = true;
        terminal.cancelStream?.();
        if (terminal.retry) clearTimeout(terminal.retry);
        active.current = null;
      }
    };
  }, [owner, chatId, targetDeviceId, restart]);

  const send = async (text: string, clearDraft = false) => {
    const terminal = active.current;
    if (!terminal || terminal.cancelled || sending || !text) return;
    const current = generation.current;
    const submittedDraft = draft;
    setSending(true); setError('');
    try {
      await callRef.current('WriteTerminal', { terminalId: terminal.id, data: encodeBase64(text), ...target });
      if (clearDraft && current === generation.current) setDraft(previous => previous === submittedDraft ? '' : previous);
    } catch (cause) { if (current === generation.current) setError(message(cause)); }
    finally { if (current === generation.current) setSending(false); }
  };

  const stop = async () => {
    const terminal = active.current;
    if (!terminal || terminal.cancelled) return;
    terminal.cancelled = true;
    terminal.cancelStream?.();
    if (terminal.retry) clearTimeout(terminal.retry);
    active.current = null;
    terminals.get(owner)?.delete(JSON.stringify([chatId, targetDeviceId ?? null]));
    setPhase('closed');
    await close(terminal.id);
  };

  const resize = (event: LayoutChangeEvent) => {
    const { width, height } = event.nativeEvent.layout;
    const next = { cols: Math.max(20, Math.min(240, Math.floor((width - 24) / 8))), rows: Math.max(6, Math.min(100, Math.floor((height - 24) / 18))) };
    if (next.cols === size.current.cols && next.rows === size.current.rows) return;
    size.current = next;
    const terminal = active.current;
    if (terminal && !terminal.cancelled) void callRef.current('ResizeTerminal', { terminalId: terminal.id, ...next, ...target }).catch(cause => setError(message(cause)));
  };

  return <View style={styles.root}>
    <View style={styles.toolbar}>
      <Text numberOfLines={1} style={styles.title}>{session?.shell.split('/').pop() ?? 'Terminal'}</Text>
      <Text numberOfLines={1} style={styles.path}>{session?.cwd ?? cwd ?? ''}</Text>
      {(phase === 'ready' || phase === 'reconnecting') && <Pressable accessibilityRole="button" accessibilityLabel="Close terminal" onPress={() => void stop()} style={styles.close}><Text style={styles.closeText}>Close</Text></Pressable>}
    </View>
    <View style={styles.outputArea} onLayout={resize}>
      <ScrollView ref={scroll} onContentSizeChange={() => { if (follow.current) scroll.current?.scrollToEnd({ animated: false }); }} onScroll={event => {
        const { contentOffset, contentSize, layoutMeasurement } = event.nativeEvent;
        follow.current = contentOffset.y + layoutMeasurement.height >= contentSize.height - 48;
      }} scrollEventThrottle={100} contentContainerStyle={styles.outputContent}>
        <Text selectable style={styles.output}>{display}</Text>
        {phase === 'opening' && <ActivityIndicator color={colors.accent} style={styles.spinner} />}
      </ScrollView>
      {!follow.current && <Pressable accessibilityRole="button" accessibilityLabel="Jump to latest terminal output" onPress={() => { follow.current = true; scroll.current?.scrollToEnd(); }} style={styles.jump}><Text style={styles.jumpText}>↓ Latest</Text></Pressable>}
    </View>
    {!!error && <Text accessibilityLiveRegion="polite" style={styles.error}>{error}</Text>}
    {phase === 'reconnecting' && <Text style={styles.notice}>Reconnecting terminal output…</Text>}
    {(phase === 'exited' || phase === 'closed') && <Text style={styles.notice}>{phase === 'exited' ? 'Terminal process exited.' : 'Terminal closed.'}</Text>}
    {chatId && (phase === 'exited' || phase === 'closed' || phase === 'error') && <Pressable accessibilityRole="button" accessibilityLabel="Open terminal" onPress={() => { terminals.get(owner)?.delete(JSON.stringify([chatId, targetDeviceId ?? null])); setRestart(value => value + 1); }} style={styles.reopen}><Text style={styles.reopenText}>Open terminal</Text></Pressable>}
    {(phase === 'ready' || phase === 'reconnecting') && <>
      <View style={styles.controls}>
        {([['Ctrl-C', '\x03'], ['Tab', '\t'], ['Esc', '\x1b'], ['↑', '\x1b[A'], ['↓', '\x1b[B']] as const).map(([label, data]) =>
          <Pressable key={label} accessibilityRole="button" accessibilityLabel={`Send ${label}`} disabled={sending} onPress={() => void send(data)} style={styles.control}><Text style={styles.controlText}>{label}</Text></Pressable>)}
      </View>
      <View style={styles.inputRow}>
        <TextInput accessibilityLabel="Terminal command" autoCapitalize="none" autoCorrect={false} value={draft} onChangeText={setDraft} onSubmitEditing={() => void send(`${draft}\r`, true)} placeholder="Command" placeholderTextColor={colors.textFaint} style={styles.input} />
        <Pressable accessibilityRole="button" accessibilityLabel="Run command" disabled={sending || !draft} onPress={() => void send(`${draft}\r`, true)} style={[styles.run, (sending || !draft) && styles.disabled]}><Text style={styles.runText}>↵</Text></Pressable>
      </View>
    </>}
  </View>;
}

const styles = StyleSheet.create({
  root: { flex: 1, backgroundColor: colors.bg },
  toolbar: { minHeight: 48, flexDirection: 'row', alignItems: 'center', gap: spacing.sm, paddingHorizontal: spacing.md, borderBottomWidth: 1, borderBottomColor: colors.border },
  title: { color: colors.text, fontSize: typography.body, fontWeight: '700' },
  path: { flex: 1, color: colors.textFaint, fontSize: typography.small },
  close: { minWidth: 44, minHeight: 44, justifyContent: 'center', alignItems: 'center' },
  closeText: { color: colors.textMuted, fontSize: typography.small },
  outputArea: { flex: 1 },
  outputContent: { flexGrow: 1, padding: spacing.md },
  output: { color: colors.text, fontFamily: typography.mono, fontSize: 12, lineHeight: 18 },
  spinner: { alignSelf: 'flex-start', marginTop: spacing.lg },
  jump: { position: 'absolute', right: spacing.md, bottom: spacing.md, paddingHorizontal: spacing.md, paddingVertical: spacing.sm, borderRadius: radius.control, backgroundColor: colors.surfaceRaised },
  jumpText: { color: colors.text, fontSize: typography.small },
  error: { color: colors.danger, fontSize: typography.small, paddingHorizontal: spacing.md, paddingVertical: spacing.xs },
  notice: { color: colors.textFaint, fontSize: typography.small, paddingHorizontal: spacing.md, paddingVertical: spacing.xs },
  controls: { flexDirection: 'row', gap: spacing.xs, paddingHorizontal: spacing.md, paddingVertical: spacing.sm, borderTopWidth: 1, borderTopColor: colors.border },
  control: { minHeight: 44, flex: 1, alignItems: 'center', justifyContent: 'center', borderRadius: radius.control, backgroundColor: colors.surfaceCard },
  controlText: { color: colors.textMuted, fontSize: typography.small },
  inputRow: { flexDirection: 'row', alignItems: 'center', gap: spacing.sm, paddingHorizontal: spacing.md, paddingBottom: spacing.md },
  input: { flex: 1, minHeight: 44, paddingHorizontal: spacing.md, borderWidth: 1, borderColor: colors.borderStrong, borderRadius: radius.control, backgroundColor: colors.inputBg, color: colors.text, fontFamily: typography.mono, fontSize: 13 },
  run: { minWidth: 44, minHeight: 44, alignItems: 'center', justifyContent: 'center', borderRadius: radius.control, backgroundColor: colors.solid },
  runText: { color: colors.onSolid, fontSize: 22 },
  disabled: { opacity: 0.4 },
  reopen: { alignSelf: 'flex-start', minHeight: 44, justifyContent: 'center', margin: spacing.md, paddingHorizontal: spacing.md, borderRadius: radius.control, backgroundColor: colors.surfaceRaised },
  reopenText: { color: colors.text, fontSize: typography.body },
});
