import { StatusBar } from 'expo-status-bar';
import { useFonts } from 'expo-font';
import { useEffect, useMemo, useRef, useState } from 'react';
import { ActivityIndicator, AppState, KeyboardAvoidingView, Platform, Pressable, StyleSheet, TextInput, View } from 'react-native';
import { Text } from './src/AppText';
import { SafeAreaProvider, SafeAreaView } from 'react-native-safe-area-context';
import { Composer, type QueuedMessage } from './src/Composer';
import { History } from './src/History';
import { Changes } from './src/Changes';
import Shell, { type ShellSection } from './src/Shell';
import { Spaces } from './src/Spaces';
import { Terminal } from './src/Terminal';
import { Transcript, type UserInputAnswer } from './src/Transcript';
import { Workspace } from './src/Workspace';
import { connect, restore, type Connection } from './src/connection';
import { clearDeliveredDraft, moveNewDraft } from './src/drafts';
import { applyTranscriptUpdate, previews, renderEntries, spacePreviews, type Chat, type LiveSession, type Space, type TranscriptUpdate, type WireEntry } from './src/liveModel';
import { colors, radius, spacing, typography } from './src/theme';
import { watchWithRetry } from './src/watch';

export default function App() {
  const [fontsLoaded, fontError] = useFonts({
    Geist: require('./assets/fonts/Geist.ttf'),
    GeistMono: require('./assets/fonts/GeistMono.ttf'),
  });
  if (!fontsLoaded && !fontError) return <View style={styles.app} />;
  return <SafeAreaProvider>
    <KeyboardAvoidingView behavior={Platform.OS === 'ios' ? 'padding' : Platform.OS === 'android' ? 'height' : undefined} style={styles.app}>
      <AppContent />
    </KeyboardAvoidingView>
  </SafeAreaProvider>;
}

function AppContent() {
  const [connection, setConnection] = useState<Connection | null>(null);
  const [starting, setStarting] = useState(true);
  const [connecting, setConnecting] = useState(false);
  const [invitation, setInvitation] = useState('');
  const [error, setError] = useState('');
  const [chats, setChats] = useState<Chat[]>([]);
  const [spaces, setSpaces] = useState<Space[]>([]);
  const [sessions, setSessions] = useState<LiveSession[]>([]);
  const [selectedId, setSelectedId] = useState<string | undefined>();
  const [selectedTargetDeviceId, setSelectedTargetDeviceId] = useState<string | null>(null);
  const [selectedSpaceId, setSelectedSpaceId] = useState<string | null>(null);
  const [entries, setEntries] = useState<WireEntry[]>([]);
  const entriesRef = useRef<WireEntry[]>([]);
  const [queue, setQueue] = useState<QueuedMessage[]>([]);
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [section, setSection] = useState<ShellSection | 'chat' | 'settings' | 'spaces'>('chat');
  const [syncEpoch, setSyncEpoch] = useState(0);
  const selectedChat = chats.find(chat => chat.id === selectedId);

  useEffect(() => {
    let alive = true;
    restore().then(saved => { if (alive) setConnection(saved); })
      .catch(e => { if (alive) setError(message(e)); })
      .finally(() => { if (alive) setStarting(false); });
    return () => { alive = false; };
  }, []);

  useEffect(() => {
    if (!connection) return;
    const listener = AppState.addEventListener('change', state => {
      if (state === 'active') setSyncEpoch(epoch => epoch + 1);
    });
    return () => listener.remove();
  }, [connection]);

  useEffect(() => {
    if (!connection) return;
    const subscribe = connection.subscribe.bind(connection);
    const fail = (e: Error) => setError(message(e));
    const stops = [
      watchWithRetry(subscribe, 'WatchChats', {}, item => setChats(asArray<Chat>(item)), fail),
      watchWithRetry(subscribe, 'WatchSpaces', {}, item => setSpaces(asArray<Space>(item)), fail),
      watchWithRetry(subscribe, 'WatchSessions', {}, item => setSessions(asArray<LiveSession>(item)), fail),
    ];
    return () => stops.forEach(stop => stop());
  }, [connection, syncEpoch]);

  useEffect(() => {
    entriesRef.current = [];
    setEntries([]);
    setQueue([]);
    if (!connection || !selectedId) return;
    const subscribe = connection.subscribe.bind(connection);
    const fail = (e: Error) => setError(message(e));
    const targetDeviceId = selectedChat?.deviceId ?? selectedTargetDeviceId ?? connection.hostDeviceId;
    const stops = [
      watchWithRetry(subscribe, 'WatchDocMessages', { chatId: selectedId, targetDeviceId }, item => {
        const next = applyTranscriptUpdate(entriesRef.current, item as TranscriptUpdate);
        entriesRef.current = next;
        setEntries(next);
      }, fail),
      watchWithRetry(subscribe, 'WatchQueue', { chatId: selectedId, targetDeviceId }, item => {
        setQueue(asArray<QueuedMessage>((item as { items?: unknown })?.items));
      }, fail),
    ];
    return () => stops.forEach(stop => stop());
  }, [connection, selectedId, selectedChat?.deviceId, selectedTargetDeviceId, syncEpoch]);

  const running = sessions.some(session => session.chatId === selectedId && session.status === 'working');
  const awaitingInput = sessions.some(session => session.chatId === selectedId && session.status === 'awaitingInput');
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
    const submittedDraft = draft;
    setBusy(true);
    setError('');
    try {
      let chatId = selectedId;
      const chat = selectedChat;
      const newSpace = selectedId ? undefined : spaces.find(space => space.id === selectedSpaceId);
      if (!chatId) {
        chatId = newId();
        await call('Mutate', { op: 'createChat', chatId, spaceId: newSpace?.id ?? null, deviceId: newSpace ? null : connection.hostDeviceId });
        setDrafts(previous => moveNewDraft(previous, chatId!));
        setSelectedTargetDeviceId(newSpace?.deviceId ?? connection.hostDeviceId);
        setSelectedId(chatId);
      }
      const targetDeviceId = chat?.deviceId ?? newSpace?.deviceId ?? connection.hostDeviceId;
      if (sendToQueue) await call('QueueMessage', { chatId, targetDeviceId, text, holdForTurnEnd: true });
      else await call('QueueCommand', { chatId, targetDeviceId, command: { kind: 'run', messageId: newId(), request: {
        prompt: text, harness: chat?.config?.harness ?? null, model: chat?.config?.model ?? null,
        reasoning: chat?.config?.reasoning ?? null, modelOptions: chat?.config?.modelOptions ?? {},
          cwd: chat?.cwd ?? newSpace?.path ?? '~', sandbox: chat?.config?.sandbox ?? 'workspace-write', autoApprove: false,
        resume: null, attachments: [],
      } } });
      setDrafts(previous => clearDeliveredDraft(previous, draftKey, chatId!, submittedDraft));
    } catch (e) { setError(message(e)); throw e; }
    finally { setBusy(false); }
  };

  const runAction = (method: string, params: Record<string, unknown>) => {
    call(method, params).catch(e => setError(message(e)));
  };

  const respondInput = async (requestId: string, answers: UserInputAnswer[]) => {
    if (!selectedId) throw new Error('No conversation is selected');
    try {
      await call('QueueCommand', {
        chatId: selectedId,
        targetDeviceId: selectedChat?.deviceId ?? selectedTargetDeviceId ?? connection?.hostDeviceId,
        command: { kind: 'respondInput', requestId, answers },
      });
      setError('');
    } catch (e) { setError(message(e)); throw e; }
  };

  const switchHost = async (deviceId: string) => {
    if (!connection || connection.hostDeviceId === deviceId) return;
    const previous = connection.hostDeviceId;
    try {
      connection.selectHostDevice(deviceId);
      await connection.call('EngineInfo', {});
      setSelectedId(undefined);
      setSelectedTargetDeviceId(null);
      setSelectedSpaceId(null);
      setChats([]);
      setSpaces([]);
      setSessions([]);
      setSection('chat');
      setError('');
      setSyncEpoch(epoch => epoch + 1);
    } catch (e) {
      if (previous) connection.selectHostDevice(previous);
      setError(message(e));
      setSyncEpoch(epoch => epoch + 1);
    }
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

  return <Shell sessions={sessionPreviews} spaces={spacePreviews(spaces)} selectedId={selectedId} selectedSpaceId={selectedSpaceId}
    onSelectSpace={setSelectedSpaceId} onAddSpace={() => setSection('spaces')} availableSections={['history', 'files', 'terminal', 'changes']}
    onSelectSession={id => { setSelectedTargetDeviceId(chats.find(chat => chat.id === id)?.deviceId ?? null); setSelectedId(id); setSection('chat'); setError(''); }}
    onNewSession={() => { setSelectedTargetDeviceId(null); setSelectedId(undefined); setSection('chat'); setError(''); }}
    onOpenSettings={() => setSection('settings')} onOpenSection={setSection}>
    <StatusBar style="light" />
    {section === 'spaces' && connection.hostDeviceId ? <Spaces call={(method, params) => connection.call(method, params)}
      hostDeviceId={connection.hostDeviceId} existingSpaces={spaces}
      onCreated={spaceId => { setSelectedSpaceId(spaceId); setSelectedId(undefined); setSection('chat'); }}
      onClose={() => setSection('chat')} /> : section === 'settings' ? <View style={styles.panel}>
      <Text style={styles.heading}>Settings</Text>
      <Text style={styles.muted}>Connected device · {connection.hostDeviceId}</Text>
      <Text style={styles.muted}>Profile · {connection.profileId}</Text>
      <Text style={styles.subheading}>Host engines</Text>
      {connection.engines.map(engine => <Pressable key={engine.deviceId} accessibilityRole="button"
        accessibilityState={{ selected: connection.hostDeviceId === engine.deviceId }} onPress={() => void switchHost(engine.deviceId)} style={styles.engineRow}>
        <Text style={styles.engineText}>{engine.displayName || engine.deviceId}</Text>
        {connection.hostDeviceId === engine.deviceId && <Text style={styles.active}>Connected</Text>}
      </Pressable>)}
      {!!error && <Text accessibilityLiveRegion="polite" style={styles.error}>{error}</Text>}
      <Pressable accessibilityRole="button" onPress={() => { connection.disconnect(); setConnection(null); setChats([]); setSpaces([]); setSessions([]); setSelectedId(undefined); }} style={styles.outlineButton}>
        <Text style={styles.outlineText}>Disconnect device</Text>
      </Pressable>
    </View> : section === 'chat' ? <View style={styles.chat}>
      <View style={styles.transcript}>
        {selectedId && <Transcript messages={transcript} isStreaming={running} onRespondInput={respondInput} />}
      </View>
      <Composer draft={draft} onChangeDraft={text => setDrafts(previous => ({ ...previous, [draftKey]: text }))}
        onSubmit={submit} expanded={!selectedId} running={running} busy={busy} disabled={awaitingInput} error={error}
        notice={awaitingInput ? 'Answer the agent’s question above to continue.' : undefined}
        target={selectedId ? undefined : spaces.find(space => space.id === selectedSpaceId)?.name || 'Your device'}
        model={selectedChat?.config?.model ?? selectedChat?.config?.harness ?? undefined}
        queue={queue} onInterrupt={selectedId ? () => runAction('QueueCommand', { chatId: selectedId, targetDeviceId: selectedChat?.deviceId ?? selectedTargetDeviceId ?? connection.hostDeviceId, command: { kind: 'interrupt' } }) : undefined}
        onRemoveQueued={selectedId ? id => runAction('RemoveQueuedMessage', { chatId: selectedId, targetDeviceId: selectedChat?.deviceId ?? selectedTargetDeviceId ?? connection.hostDeviceId, id }) : undefined}
        onEditQueued={selectedId ? (id, text) => runAction('UpdateQueuedMessage', { chatId: selectedId, targetDeviceId: selectedChat?.deviceId ?? selectedTargetDeviceId ?? connection.hostDeviceId, id, text }) : undefined} />
    </View> : section === 'history' && selectedChat ? <History call={(method, params) => connection.call(method, params)}
      chatId={selectedChat.id} targetDeviceId={selectedChat.deviceId} cwd={selectedChat.cwd} />
      : section === 'changes' && selectedChat ? <Changes call={(method, params) => connection.call(method, params)}
      subscribe={(method, params, onItem, onError) => connection.subscribe(method, params, onItem, onError)}
      chatId={selectedChat.id} targetDeviceId={selectedChat.deviceId} cwd={selectedChat.cwd} />
      : section === 'terminal' && selectedChat ? <Terminal call={(method, params) => connection.call(method, params)}
      owner={connection}
      subscribe={(method, params, onItem, onError) => connection.subscribe(method, params, onItem, onError)}
      chatId={selectedChat.id} targetDeviceId={selectedChat.deviceId} cwd={selectedChat.cwd} />
      : section === 'files' && selectedChat ? <Workspace call={(method, params) => connection.call(method, params)}
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
  app: { backgroundColor: colors.bg, flex: 1 },
  center: { alignItems: 'center', backgroundColor: colors.bg, flex: 1, gap: spacing.md, justifyContent: 'center' },
  pairScreen: { alignItems: 'center', backgroundColor: colors.bg, flex: 1, justifyContent: 'center', padding: spacing.lg },
  pairCard: { backgroundColor: colors.surfaceCard, borderColor: colors.border, borderRadius: radius.panel, borderWidth: 1, maxWidth: 440, padding: spacing.lg, width: '100%' },
  eyebrow: { color: colors.accent, fontSize: typography.caption, fontWeight: '700', letterSpacing: 2 },
  pairTitle: { color: colors.text, fontSize: typography.heading, fontWeight: '700', marginBottom: spacing.sm, marginTop: spacing.sm },
  muted: { color: colors.textMuted, fontSize: typography.body, lineHeight: 21, marginTop: spacing.sm },
  pairInput: { borderColor: colors.borderStrong, borderRadius: radius.control, borderWidth: 1, color: colors.text, fontFamily: typography.family, fontSize: typography.body, marginTop: spacing.lg, minHeight: 88, padding: spacing.md, textAlignVertical: 'top' },
  primaryButton: { alignItems: 'center', backgroundColor: colors.solid, borderRadius: radius.control, marginTop: spacing.md, padding: spacing.md },
  primaryText: { color: colors.onSolid, fontSize: typography.body, fontWeight: '700' },
  disabled: { opacity: 0.45 },
  error: { color: colors.danger, fontSize: typography.small, marginTop: spacing.md },
  chat: { backgroundColor: colors.bg, flex: 1, minHeight: 0 },
  heading: { color: colors.text, flexShrink: 1, fontSize: typography.title, fontWeight: '700' },
  subheading: { color: colors.text, fontSize: typography.body, fontWeight: '600', marginTop: spacing.lg },
  engineRow: { alignItems: 'center', borderBottomColor: colors.border, borderBottomWidth: 1, flexDirection: 'row', justifyContent: 'space-between', minHeight: 44 },
  engineText: { color: colors.text, flex: 1, fontSize: typography.body },
  active: { color: colors.success, fontSize: typography.small },
  transcript: { flex: 1, minHeight: 0 },
  panel: { backgroundColor: colors.bg, flex: 1, padding: spacing.lg },
  outlineButton: { alignSelf: 'flex-start', borderColor: colors.borderStrong, borderRadius: radius.control, borderWidth: 1, marginTop: spacing.lg, padding: spacing.md },
  outlineText: { color: colors.text, fontSize: typography.body },
});
