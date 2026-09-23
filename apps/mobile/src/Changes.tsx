import { useCallback, useEffect, useRef, useState } from 'react';
import { ActivityIndicator, FlatList, Pressable, StyleSheet, View } from 'react-native';
import { Text } from './AppText';
import { colors, radius, spacing, typography } from './theme';

type DiffFile = { path: string; oldPath?: string; status: string; additions: number; deletions: number; binary: boolean };
type CheckoutDiff = {
  checkoutId: string; deviceId: string; cwd: string; patch: string; files: DiffFile[];
  additions: number; deletions: number; truncated: boolean; checksum: string;
};
type FileText = { diffChecksum: string; oldText: string | null; newText: string | null; binary: boolean; truncated: boolean; stale: boolean };
type Props = {
  call: (method: string, params: Record<string, unknown>) => Promise<unknown>;
  subscribe: (method: string, params: Record<string, unknown>, onItem: (item: unknown) => void,
    onError?: (error: Error) => void) => Promise<() => void>;
  chatId: string;
  targetDeviceId?: string | null;
  cwd?: string | null;
};

function record(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('Invalid diff response');
  return value as Record<string, unknown>;
}

function diffFrom(value: unknown): CheckoutDiff {
  const data = record(value);
  if (typeof data.checkoutId !== 'string' || typeof data.deviceId !== 'string' ||
      typeof data.cwd !== 'string' || typeof data.patch !== 'string' ||
      typeof data.checksum !== 'string' || !Array.isArray(data.files)) throw new Error('Invalid checkout diff');
  const files = data.files.map(raw => {
    const file = record(raw);
    if (typeof file.path !== 'string' || typeof file.status !== 'string') throw new Error('Invalid diff file');
    return {
      path: file.path, oldPath: typeof file.oldPath === 'string' ? file.oldPath : undefined,
      status: file.status, additions: Number(file.additions) || 0,
      deletions: Number(file.deletions) || 0, binary: file.binary === true,
    };
  });
  return {
    checkoutId: data.checkoutId, deviceId: data.deviceId, cwd: data.cwd,
    patch: data.patch, files, additions: Number(data.additions) || 0,
    deletions: Number(data.deletions) || 0, truncated: data.truncated === true,
    checksum: data.checksum,
  };
}

function textFrom(value: unknown, checksum: string): FileText {
  const data = record(value);
  if (typeof data.diffChecksum !== 'string') throw new Error('Invalid file diff');
  return {
    diffChecksum: data.diffChecksum,
    oldText: typeof data.oldText === 'string' ? data.oldText : null,
    newText: typeof data.newText === 'string' ? data.newText : null,
    binary: data.binary === true, truncated: data.truncated === true,
    stale: data.stale === true || data.diffChecksum !== checksum,
  };
}

function sameCwd(a: string, b: string): boolean { return a.replace(/\/+$/, '') === b.replace(/\/+$/, ''); }
function errorText(error: unknown): string { return error instanceof Error ? error.message : 'Diff request failed'; }

function filePatch(patch: string, file: DiffFile): string {
  const sections = patch.split(/(?=^diff --git )/m);
  const found = sections.find(section => {
    const newPath = section.match(/^\+\+\+ b\/(.+)$/m)?.[1];
    const oldPath = section.match(/^--- a\/(.+)$/m)?.[1];
    return newPath === file.path || oldPath === (file.oldPath ?? file.path);
  });
  return found ?? '';
}

function DiffLines({ source, mode }: { source: string; mode: 'patch' | 'before' | 'after' }) {
  const lines = source.split('\n');
  if (lines.at(-1) === '') lines.pop();
  return <FlatList
    data={lines}
    keyExtractor={(_, index) => String(index)}
    contentContainerStyle={styles.diffContent}
    renderItem={({ item, index }) => {
      const added = mode === 'patch' && item.startsWith('+') && !item.startsWith('+++');
      const removed = mode === 'patch' && item.startsWith('-') && !item.startsWith('---');
      const header = mode === 'patch' && item.startsWith('@@');
      return <View style={[styles.line, added && styles.addedLine, removed && styles.removedLine, header && styles.hunkLine]}>
        <Text style={styles.lineNumber}>{mode === 'patch' ? '' : index + 1}</Text>
        <Text selectable style={[styles.lineText, added && styles.addedText, removed && styles.removedText, header && styles.hunkText]}>{item || ' '}</Text>
      </View>;
    }}
    ListEmptyComponent={<Text style={styles.empty}>No text in this view.</Text>}
  />;
}

export function Changes({ call, subscribe, chatId, targetDeviceId, cwd }: Props) {
  const callRef = useRef(call); callRef.current = call;
  const subscribeRef = useRef(subscribe); subscribeRef.current = subscribe;
  const [diff, setDiff] = useState<CheckoutDiff | null>(null);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [text, setText] = useState<FileText | null>(null);
  const [mode, setMode] = useState<'patch' | 'before' | 'after'>('patch');
  const [loading, setLoading] = useState(false);
  const [reading, setReading] = useState(false);
  const [error, setError] = useState('');
  const [watchError, setWatchError] = useState('');
  const [fileError, setFileError] = useState('');
  const requestId = useRef(0);
  const fileRequestId = useRef(0);
  const target = useCallback(() => targetDeviceId ? { targetDeviceId } : {}, [targetDeviceId]);

  const refresh = useCallback(async () => {
    if (!chatId || !cwd) return;
    const current = ++requestId.current;
    setLoading(true); setError('');
    try {
      const next = diffFrom(await callRef.current('GetCheckoutDiff', {
        cwd, mode: 'workingTree', chatId, ...target(),
      }));
      if (current === requestId.current) setDiff(next);
    } catch (cause) {
      if (current === requestId.current) setError(errorText(cause));
    } finally {
      if (current === requestId.current) setLoading(false);
    }
  }, [chatId, cwd, target]);

  useEffect(() => {
    setDiff(null); setSelectedPath(null); setText(null); setError(''); setWatchError('');
    void refresh();
    return () => { requestId.current++; fileRequestId.current++; };
  }, [refresh]);

  useEffect(() => {
    if (!chatId || !cwd) return;
    let disposed = false;
    let stop: (() => void) | undefined;
    let retry: ReturnType<typeof setTimeout> | undefined;
    const schedule = (message: string) => {
      if (disposed) return;
      setWatchError(message);
      stop?.(); stop = undefined;
      if (retry) clearTimeout(retry);
      retry = setTimeout(start, 2000);
    };
    const onItem = (value: unknown) => {
      if (disposed) return;
      try {
        if (Array.isArray(value)) {
          const all = value.map(diffFrom);
          setDiff(current => all.find(item => item.checkoutId === current?.checkoutId) ?? all.find(item => sameCwd(item.cwd, cwd)) ?? null);
        } else {
          const next = diffFrom(value);
          setDiff(current => current?.checkoutId === next.checkoutId || sameCwd(next.cwd, cwd) ? next : current);
        }
        setError('');
        setWatchError('');
      } catch (cause) { setWatchError(errorText(cause)); }
    };
    const start = async () => {
      if (disposed) return;
      try {
        const unsubscribe = await subscribeRef.current('WatchCheckoutDiffs', target(), onItem,
          cause => schedule(`Diff watch interrupted: ${errorText(cause)}`));
        if (disposed) unsubscribe(); else stop = unsubscribe;
      } catch (cause) { schedule(`Diff watch unavailable: ${errorText(cause)}`); }
    };
    void start();
    return () => { disposed = true; stop?.(); if (retry) clearTimeout(retry); };
  }, [chatId, cwd, target]);

  const selected = diff?.files.find(file => file.path === selectedPath) ?? null;
  useEffect(() => {
    if (!selected || !diff || !chatId) { setText(null); return; }
    const current = ++fileRequestId.current;
    setReading(true); setText(null); setFileError('');
    void (async () => {
      try {
        const response = textFrom(await callRef.current('GetCheckoutFileDiffText', {
          checkoutId: diff.checkoutId, cwd: diff.cwd, path: selected.path,
          mode: 'workingTree', chatId, diffChecksum: diff.checksum, ...target(),
        }), diff.checksum);
        if (current !== fileRequestId.current) return;
        if (response.stale) setFileError('Diff changed while loading. Refresh to view the latest file.');
        else setText(response);
      } catch (cause) {
        if (current === fileRequestId.current) setFileError(errorText(cause));
      } finally {
        if (current === fileRequestId.current) setReading(false);
      }
    })();
    return () => { fileRequestId.current++; };
  }, [selected?.path, diff?.checkoutId, diff?.checksum, chatId, target]);

  if (!chatId || !cwd) return <View style={styles.center}><Text style={styles.muted}>No checkout is available for this chat.</Text></View>;

  const selectedPatch = selected && diff ? filePatch(diff.patch, selected) : '';
  const source = mode === 'patch' ? selectedPatch : mode === 'before' ? text?.oldText : text?.newText;
  return <View style={styles.root}>
    <View style={styles.toolbar}>
      {selectedPath && <Pressable accessibilityRole="button" onPress={() => { setSelectedPath(null); setMode('patch'); }} style={styles.toolbarButton}><Text style={styles.toolbarText}>‹ Changes</Text></Pressable>}
      <Text numberOfLines={1} style={styles.title}>{selected?.path.split('/').at(-1) ?? 'Changes'}</Text>
      <Pressable accessibilityRole="button" accessibilityLabel="Refresh changes" onPress={() => void refresh()} style={styles.toolbarButton}><Text style={styles.toolbarText}>↻</Text></Pressable>
    </View>
    {selected ? <>
      <Text numberOfLines={1} style={styles.path}>{selected.path}{selected.oldPath ? ` ← ${selected.oldPath}` : ''}</Text>
      <View style={styles.tabs}>{(['patch', 'before', 'after'] as const).map(tab => <Pressable key={tab} accessibilityRole="tab" accessibilityState={{ selected: mode === tab }} onPress={() => setMode(tab)} style={[styles.tab, mode === tab && styles.activeTab]}><Text style={[styles.tabText, mode === tab && styles.activeTabText]}>{tab === 'patch' ? 'Diff' : tab === 'before' ? 'Before' : 'After'}</Text></Pressable>)}</View>
      {reading && mode !== 'patch' ? <ActivityIndicator style={styles.busy} color={colors.accent} /> :
        source != null && source !== '' ? <DiffLines source={source} mode={mode} /> :
        <View style={styles.center}><Text style={styles.muted}>{selected.binary || text?.binary ? 'Binary file: text diff unavailable.' : reading ? 'Loading file text…' : mode === 'patch' && diff?.truncated ? 'Patch is partial. Use Before or After for the full text.' : 'No text available for this view.'}</Text></View>}
      {text?.truncated && <Text style={styles.notice}>File text is partial.</Text>}
      {!!fileError && <Text accessibilityLiveRegion="polite" style={styles.error}>{fileError}</Text>}
    </> : <>
      <Text numberOfLines={1} style={styles.path}>{cwd}</Text>
      {diff && <Text style={styles.summary}>{diff.files.length} {diff.files.length === 1 ? 'file' : 'files'}  <Text style={styles.addedText}>+{diff.additions}</Text>  <Text style={styles.removedText}>−{diff.deletions}</Text>{diff.truncated ? '  · Partial snapshot' : ''}</Text>}
      {loading && !diff ? <ActivityIndicator style={styles.busy} color={colors.accent} /> :
        <FlatList
          data={diff?.files ?? []}
          keyExtractor={file => file.path}
          renderItem={({ item }) => <Pressable accessibilityRole="button" onPress={() => { setSelectedPath(item.path); setMode('patch'); }} style={styles.fileRow}>
            <Text numberOfLines={1} style={styles.status}>{item.status}</Text>
            <View style={styles.fileInfo}><Text numberOfLines={1} style={styles.fileName}>{item.path}</Text>{item.oldPath && <Text numberOfLines={1} style={styles.oldPath}>from {item.oldPath}</Text>}</View>
            <Text style={styles.counts}><Text style={styles.addedText}>+{item.additions}</Text> <Text style={styles.removedText}>−{item.deletions}</Text></Text>
          </Pressable>}
          ListEmptyComponent={!error ? <Text style={styles.empty}>{diff ? 'No changes in this checkout.' : 'Waiting for checkout changes…'}</Text> : null}
        />}
      {!!error && <Pressable accessibilityRole="button" onPress={() => void refresh()} style={styles.errorBox}><Text style={styles.error}>{error} · Tap to retry</Text></Pressable>}
      {!!watchError && <Text accessibilityLiveRegion="polite" style={styles.notice}>{watchError}</Text>}
    </>}
  </View>;
}

const styles = StyleSheet.create({
  root: { flex: 1, backgroundColor: colors.bg },
  center: { flex: 1, alignItems: 'center', justifyContent: 'center', padding: spacing.lg },
  toolbar: { minHeight: 48, flexDirection: 'row', alignItems: 'center', gap: spacing.sm, paddingHorizontal: spacing.md, borderBottomColor: colors.border, borderBottomWidth: 1 },
  toolbarButton: { minWidth: 44, minHeight: 44, alignItems: 'center', justifyContent: 'center' },
  toolbarText: { color: colors.textMuted, fontSize: typography.body },
  title: { flex: 1, color: colors.text, fontSize: typography.body, fontWeight: '700' },
  path: { color: colors.textFaint, fontSize: typography.small, paddingHorizontal: spacing.lg, paddingVertical: spacing.sm },
  summary: { color: colors.textMuted, fontSize: typography.small, paddingHorizontal: spacing.lg, paddingBottom: spacing.md },
  fileRow: { minHeight: 52, flexDirection: 'row', alignItems: 'center', paddingHorizontal: spacing.lg, gap: spacing.md, borderBottomColor: colors.border, borderBottomWidth: 1 },
  status: { color: colors.textMuted, fontFamily: typography.mono, fontSize: typography.small, width: 24 },
  fileInfo: { flex: 1 },
  fileName: { color: colors.text, fontSize: typography.body },
  oldPath: { color: colors.textFaint, fontSize: typography.small, marginTop: 2 },
  counts: { fontFamily: typography.mono, fontSize: typography.small },
  addedText: { color: colors.success },
  removedText: { color: colors.danger },
  tabs: { flexDirection: 'row', borderBottomColor: colors.border, borderBottomWidth: 1 },
  tab: { paddingHorizontal: spacing.lg, paddingVertical: spacing.md },
  activeTab: { borderBottomColor: colors.accent, borderBottomWidth: 2 },
  tabText: { color: colors.textMuted, fontSize: typography.small },
  activeTabText: { color: colors.text },
  diffContent: { paddingBottom: 24 },
  line: { flexDirection: 'row', paddingHorizontal: spacing.sm, minHeight: 20 },
  lineNumber: { color: colors.textFaint, fontFamily: typography.mono, fontSize: typography.small, width: 32, textAlign: 'right', marginRight: spacing.md },
  lineText: { color: colors.textMuted, fontFamily: typography.mono, fontSize: typography.small, lineHeight: 20, flex: 1 },
  addedLine: { backgroundColor: 'rgba(52,211,153,0.08)' },
  removedLine: { backgroundColor: 'rgba(251,113,133,0.08)' },
  hunkLine: { backgroundColor: colors.surfaceRaised },
  hunkText: { color: colors.accent },
  busy: { marginTop: 32 },
  muted: { color: colors.textMuted, fontSize: typography.body, textAlign: 'center' },
  empty: { color: colors.textFaint, fontSize: typography.body, textAlign: 'center', marginTop: 48 },
  notice: { color: colors.textMuted, fontSize: typography.small, paddingHorizontal: spacing.lg, paddingVertical: spacing.sm },
  errorBox: { padding: spacing.md },
  error: { color: colors.danger, fontSize: typography.small, paddingHorizontal: spacing.lg, paddingVertical: spacing.sm },
});
