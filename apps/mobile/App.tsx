import { StatusBar } from 'expo-status-bar';
import { useEffect, useMemo, useState } from 'react';
import { ActivityIndicator, Pressable, SafeAreaView, StyleSheet, Text, TextInput, View } from 'react-native';
import { Composer, type QueuedMessage } from './src/Composer';
import Shell, { type ShellSection } from './src/Shell';
import { Transcript } from './src/Transcript';
import { Workspace } from './src/Workspace';
import { connect, restore, type Connection } from './src/connection';
import { applyTranscriptUpdate, previews, renderEntries, spacePreviews, type Chat, type LiveSession, type Space, type TranscriptUpdate, type WireEntry } from './src/liveModel';
import { colors, radius, spacing, typography } from './src/theme';

export default function App() {
  const [connection, setConnection] = useState<Connection | null>(null);
  const [starting, setStarting] = useState(true);
  const [connecting, setConnecting] = useState(false);
  const [invitation, setInvitation] = useState('');
  const [error, setError] = useState('');
  const [chats, setChats] = useState<Chat[]>([]);
  const [spaces, setSpaces] = useState<Space[]>([]);
  const [sessions, setSessions] = useState<LiveSession[]>([]);
  const [selectedId, setSelectedId] = useState<string | undefined>();
  const [entries, setEntries] = useState<WireEntry[]>([]);
  const [queue, setQueue] = useState<QueuedMessage[]>([]);
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [section, setSection] = useState<ShellSection | 'chat' | 'settings'>('chat');
  const [syncEpoch, setSyncEpoch] = useState(0);

  useEffect(() => {
    let alive = true;
    restore().then(saved => { if (alive) setConnection(saved); })
      .catch(e => { if (alive) setError(message(e)); })
      .finally(() => { if (alive) setStarting(false); });
    return () => { alive = false; };
  }, []);

  useEffect(() => {
    if (!connection) return;
    let cancelled = false;
    const stops: (() => void)[] = [];
    const watch = async (method: string, onItem: (item: unknown) => void) => {
      try {
        const stop = await connection.subscribe(method, {}, item => { if (!cancelled) onItem(item); }, e => { if (!cancelled) setError(message(e)); });
        if (cancelled) stop(); else stops.push(stop);
      } catch (e) { if (!cancelled) setError(message(e)); }
    };
    void watch('WatchChats', item => setChats(asArray<Chat>(item)));
    void watch('WatchSpaces', item => setSpaces(asArray<Space>(item)));
    void watch('WatchSessions', item => setSessions(asArray<LiveSession>(item)));
    return () => { cancelled = true; stops.forEach(stop => stop()); };
  }, [connection, syncEpoch]);

  useEffect(() => {
    setEntries([]);
    setQueue([]);
    if (!connection || !selectedId) return;
    let cancelled = false;
    const stops: (() => void)[] = [];
    const fail = (e: Error) => { if (!cancelled) setError(message(e)); };
    connection.subscribe('WatchDocMessages', { chatId: selectedId }, item => {
      if (cancelled) return;
      try { setEntries(previous => applyTranscriptUpdate(previous, item as TranscriptUpdate)); }
      catch (e) { fail(e as Error); }
    }, fail).then(stop => { if (cancelled) stop(); else stops.push(stop); }).catch(fail);
    connection.subscribe('WatchQueue', { chatId: selectedId }, item => {
      if (!cancelled) setQueue(asArray<QueuedMessage>((item as { items?: unknown })?.items));
    }, fail).then(stop => { if (cancelled) stop(); else stops.push(stop); }).catch(fail);
    return () => { cancelled = true; stops.forEach(stop => stop()); };
  }, [connection, selectedId, syncEpoch]);

  const selectedChat = chats.find(chat => chat.id === selectedId);
  const running = sessions.some(session => session.chatId === selectedId && session.status === 'working');
  const sessionPreviews = useMemo(() => previews(chats, spaces, sessions), [chats, spaces, sessions]);
  const transcript = useMemo(() => renderEntries(entries), [entries]);
  const draftKey = selectedId ?? '__new__';
  const draft = drafts[draftKey] ?? '';

  const pair = async () => {
    setConnecting(true);
    setError('');
    try { setConnection(await connect(invitation.trim())); setInvitation(''); }
    catch (e) { setError(message(e)); }
    finally { setConnecting(false); }
  };

  const retrySaved = async () => {
    setConnecting(true);
    setError('');
    try {
      const saved = await restore();
      if (!saved) throw new Error('No saved device to reconnect');
      setConnection(saved);
    } catch (e) { setError(message(e)); }
    finally { setConnecting(false); }
  };

  const call = async (method: string, params: Record<string, unknown>) => {
    if (!connection) throw new Error('Connect to a device first');
    return connection.call(method, params);
  };

  const submit = async (text: string, sendToQueue: boolean) => {
    if (!connection) return;
    setBusy(true);
    setError('');
    try {
      let chatId = selectedId;
      const chat = selectedChat;
      if (!chatId) {
        chatId = newId();
        const space = spaces[0];
        await call('Mutate', { op: 'createChat', chatId, spaceId: space?.id ?? null, deviceId: space ? null : connection.hostDeviceId });
        setDrafts(previous => ({ ...previous, [chatId!]: previous.__new__ ?? '' }));
        setSelectedId(chatId);
      }
      if (sendToQueue) await call('QueueMessage', { chatId, text, holdForTurnEnd: true });
      else await call('QueueCommand', { chatId, command: { kind: 'run', messageId: newId(), request: {
        prompt: text, harness: chat?.config?.harness ?? null, model: chat?.config?.model ?? null,
        reasoning: chat?.config?.reasoning ?? null, modelOptions: chat?.config?.modelOptions ?? {},
        cwd: chat?.cwd ?? '', sandbox: chat?.config?.sandbox ?? 'workspace-write', autoApprove: false,
        resume: null, attachments: [],
      } } });
      setDrafts(previous => ({ ...previous, [draftKey]: '', [chatId!]: '' }));
    } catch (e) { setError(message(e)); throw e; }
    finally { setBusy(false); }
  };

  const runAction = (method: string, params: Record<string, unknown>) => {
    call(method, params).catch(e => setError(message(e)));
  };

  if (starting) return <SafeAreaView style={styles.center}><StatusBar style="light" /><ActivityIndicator color={colors.accent} /><Text style={styles.muted}>Connecting to Kratos…</Text></SafeAreaView>;
  if (!connection) return <SafeAreaView style={styles.pairScreen}>
    <StatusBar style="light" />
    <View style={styles.pairCard}>
      <Text style={styles.eyebrow}>KRATOS</Text>
      <Text style={styles.pairTitle}>Connect your device</Text>
      <Text style={styles.muted}>Create a pairing invitation on your Kratos desktop, then paste it here. Your device connects through Tailcat.</Text>
      <TextInput accessibilityLabel="Pairing invitation" autoCapitalize="none" autoCorrect={false} multiline onChangeText={setInvitation}
        placeholder="kratos-pair:…" placeholderTextColor={colors.textFaint} style={styles.pairInput} value={invitation} />
      <Pressable accessibilityRole="button" disabled={!invitation.trim() || connecting} onPress={pair} style={[styles.primaryButton, (!invitation.trim() || connecting) && styles.disabled]}>
        <Text style={styles.primaryText}>{connecting ? 'Connecting…' : 'Connect'}</Text>
      </Pressable>
      <Pressable accessibilityRole="button" disabled={connecting} onPress={retrySaved} style={styles.outlineButton}>
        <Text style={styles.outlineText}>Reconnect paired device</Text>
      </Pressable>
      {!!error && <Text accessibilityLiveRegion="polite" style={styles.error}>{error}</Text>}
    </View>
  </SafeAreaView>;

  return <Shell sessions={sessionPreviews} spaces={spacePreviews(spaces)} selectedId={selectedId}
    onSelectSession={id => { setSelectedId(id); setSection('chat'); setError(''); }}
    onNewSession={() => { setSelectedId(undefined); setSection('chat'); setError(''); }}
    onOpenSettings={() => setSection('settings')} onOpenSection={setSection}>
    <StatusBar style="light" />
    {section === 'settings' ? <View style={styles.panel}>
      <Text style={styles.heading}>Settings</Text>
      <Text style={styles.muted}>Connected device · {connection.hostDeviceId}</Text>
      <Text style={styles.muted}>Profile · {connection.profileId}</Text>
      <Pressable accessibilityRole="button" onPress={() => { connection.disconnect(); setConnection(null); setChats([]); setSpaces([]); setSessions([]); setSelectedId(undefined); }} style={styles.outlineButton}>
        <Text style={styles.outlineText}>Disconnect device</Text>
      </Pressable>
    </View> : section === 'chat' ? <View style={styles.chat}>
      <View style={styles.chatHeader}>
        <Text numberOfLines={1} style={styles.heading}>{selectedChat?.title || (selectedId ? 'Conversation' : 'New conversation')}</Text>
        <Pressable accessibilityRole="button" onPress={() => { setError(''); setSyncEpoch(epoch => epoch + 1); }}><Text style={styles.refresh}>Refresh</Text></Pressable>
      </View>
      <View style={styles.transcript}>
        {selectedId ? <Transcript messages={transcript} isStreaming={running} /> : <View style={styles.empty}>
          <Text style={styles.emptyTitle}>What are we working on?</Text>
          <Text style={styles.muted}>Start a conversation with your connected Kratos device.</Text>
        </View>}
      </View>
      <Composer draft={draft} onChangeDraft={text => setDrafts(previous => ({ ...previous, [draftKey]: text }))}
        onSubmit={submit} running={running} busy={busy} error={error}
        target={selectedChat ? undefined : spaces[0]?.name || 'Your device'} project={selectedChat?.cwd ?? undefined}
        model={selectedChat?.config?.model ?? selectedChat?.config?.harness ?? undefined}
        queue={queue} onInterrupt={selectedId ? () => runAction('QueueCommand', { chatId: selectedId, command: { kind: 'interrupt' } }) : undefined}
        onRemoveQueued={selectedId ? id => runAction('RemoveQueuedMessage', { chatId: selectedId, id }) : undefined}
        onEditQueued={selectedId ? (id, text) => runAction('UpdateQueuedMessage', { chatId: selectedId, id, text }) : undefined} />
    </View> : section === 'files' && selectedChat ? <Workspace call={(method, params) => connection.call(method, params)}
      chatId={selectedChat.id} targetDeviceId={selectedChat.deviceId} cwd={selectedChat.cwd ?? undefined} /> : <View style={styles.panel}>
      <Text style={styles.heading}>{section[0].toUpperCase() + section.slice(1)}</Text>
      <Text style={styles.muted}>Select a conversation to open its {section}.</Text>
    </View>}
  </Shell>;
}

function asArray<T>(value: unknown): T[] { return Array.isArray(value) ? value as T[] : []; }
function message(error: unknown): string { return error instanceof Error ? error.message : 'Could not reach the device'; }
function newId(): string { return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}-${Math.random().toString(36).slice(2)}`; }

const styles = StyleSheet.create({
  center: { alignItems: 'center', backgroundColor: colors.bg, flex: 1, gap: spacing.md, justifyContent: 'center' },
  pairScreen: { alignItems: 'center', backgroundColor: colors.bg, flex: 1, justifyContent: 'center', padding: spacing.lg },
  pairCard: { backgroundColor: colors.surfaceCard, borderColor: colors.border, borderRadius: radius.panel, borderWidth: 1, maxWidth: 440, padding: spacing.lg, width: '100%' },
  eyebrow: { color: colors.accent, fontSize: typography.caption, fontWeight: '700', letterSpacing: 2 },
  pairTitle: { color: colors.text, fontSize: typography.heading, fontWeight: '700', marginBottom: spacing.sm, marginTop: spacing.sm },
  muted: { color: colors.textMuted, fontSize: typography.body, lineHeight: 21, marginTop: spacing.sm },
  pairInput: { borderColor: colors.borderStrong, borderRadius: radius.control, borderWidth: 1, color: colors.text, fontSize: typography.body, marginTop: spacing.lg, minHeight: 88, padding: spacing.md, textAlignVertical: 'top' },
  primaryButton: { alignItems: 'center', backgroundColor: colors.solid, borderRadius: radius.control, marginTop: spacing.md, padding: spacing.md },
  primaryText: { color: colors.onSolid, fontSize: typography.body, fontWeight: '700' },
  disabled: { opacity: 0.45 },
  error: { color: colors.danger, fontSize: typography.small, marginTop: spacing.md },
  chat: { backgroundColor: colors.bg, flex: 1 },
  chatHeader: { alignItems: 'center', borderBottomColor: colors.border, borderBottomWidth: 1, flexDirection: 'row', justifyContent: 'space-between', minHeight: 54, paddingHorizontal: spacing.lg },
  heading: { color: colors.text, flexShrink: 1, fontSize: typography.title, fontWeight: '700' },
  refresh: { color: colors.textMuted, fontSize: typography.small, padding: spacing.sm },
  transcript: { flex: 1 },
  empty: { alignItems: 'center', flex: 1, justifyContent: 'center', padding: spacing.lg },
  emptyTitle: { color: colors.text, fontSize: typography.heading, fontWeight: '600' },
  panel: { backgroundColor: colors.bg, flex: 1, padding: spacing.lg },
  outlineButton: { alignSelf: 'flex-start', borderColor: colors.borderStrong, borderRadius: radius.control, borderWidth: 1, marginTop: spacing.lg, padding: spacing.md },
  outlineText: { color: colors.text, fontSize: typography.body },
});
