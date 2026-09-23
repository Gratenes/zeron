import { useCallback, useEffect, useRef, useState } from 'react';
import { ActivityIndicator, FlatList, Pressable, StyleSheet, TextInput, View } from 'react-native';
import { Text } from './AppText';
import { colors, radius, spacing, typography } from './theme';

type Entry = { path: string; name: string; kind: 'file' | 'directory' | 'symlink'; size?: number; ignored: boolean; readOnly: boolean };
type Page = { entries: Entry[]; nextCursor?: string | null };
type FileText = {
  checkoutId: string; path: string; text: string | null; contentHash: string | null;
  encoding: 'utf8' | 'utf8Bom' | 'binary' | 'unsupported';
  lineEnding: 'lf' | 'crlf' | 'mixed' | 'none' | null;
  readOnlyReason: string | null; truncated: boolean;
};
type WorkspaceProps = {
  call: (method: string, params: Record<string, unknown>) => Promise<unknown>;
  chatId: string;
  targetDeviceId?: string | null;
  cwd?: string | null;
};

function record(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('Invalid workspace response');
  return value as Record<string, unknown>;
}

function pageFrom(value: unknown, directory: string): Page {
  const data = record(value);
  if (data.directory !== directory || !Array.isArray(data.entries)) throw new Error('Invalid directory listing');
  const entries = data.entries.map(raw => {
    const entry = record(raw);
    if (typeof entry.path !== 'string' || typeof entry.name !== 'string' ||
        !['file', 'directory', 'symlink'].includes(String(entry.kind))) throw new Error('Invalid directory entry');
    return {
      path: entry.path, name: entry.name, kind: entry.kind as Entry['kind'],
      size: typeof entry.size === 'number' ? entry.size : undefined,
      ignored: entry.ignored === true, readOnly: entry.readOnly === true,
    };
  });
  if (data.nextCursor != null && typeof data.nextCursor !== 'string') throw new Error('Invalid directory cursor');
  return { entries, nextCursor: data.nextCursor as string | null | undefined };
}

function fileFrom(value: unknown, path: string): FileText {
  const data = record(value);
  if (data.path !== path || typeof data.checkoutId !== 'string' ||
      !['utf8', 'utf8Bom', 'binary', 'unsupported'].includes(String(data.encoding)) ||
      (data.text != null && typeof data.text !== 'string')) throw new Error('Invalid file response');
  return {
    checkoutId: data.checkoutId, path, text: typeof data.text === 'string' ? data.text : null,
    contentHash: typeof data.contentHash === 'string' ? data.contentHash : null,
    encoding: data.encoding as FileText['encoding'],
    lineEnding: typeof data.lineEnding === 'string' ? data.lineEnding as FileText['lineEnding'] : null,
    readOnlyReason: typeof data.readOnlyReason === 'string' ? data.readOnlyReason : null,
    truncated: data.truncated === true,
  };
}

function errorText(error: unknown): string { return error instanceof Error ? error.message : 'Workspace request failed'; }
function parentOf(path: string): string { return path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : ''; }
function fileSize(bytes?: number): string { return bytes == null ? '' : bytes < 1024 ? `${bytes} B` : `${(bytes / 1024).toFixed(1)} KB`; }

type UnsavedDraft = { selected: Entry; file: FileText; text: string };
type SaveOutcome = { status: 'written'; file: FileText } | { status: 'conflict' } | { status: 'error'; message: string };
const unsavedDrafts = new Map<string, UnsavedDraft>();
const pendingSaves = new Map<string, Promise<SaveOutcome>>();
const MAX_UNSAVED_DRAFTS = 12;
const MAX_UNSAVED_DRAFT_BYTES = 48 * 1024 * 1024;

function retainDraft(context: string, value: UnsavedDraft): boolean {
  if (!unsavedDrafts.has(context) && unsavedDrafts.size >= MAX_UNSAVED_DRAFTS) return false;
  let characters = (value.file.text?.length ?? 0) + value.text.length;
  for (const [key, draft] of unsavedDrafts) {
    if (key !== context) characters += (draft.file.text?.length ?? 0) + draft.text.length;
  }
  if (characters * 2 > MAX_UNSAVED_DRAFT_BYTES) return false;
  unsavedDrafts.set(context, value);
  return true;
}

export function Workspace({ call, chatId, targetDeviceId, cwd }: WorkspaceProps) {
  const context = `${chatId}\0${targetDeviceId ?? ''}\0${cwd ?? ''}`;
  const restored = unsavedDrafts.get(context);
  const pendingSave = pendingSaves.get(context);
  const callRef = useRef(call);
  callRef.current = call;
  const currentContext = useRef(context);
  currentContext.current = context;
  const generation = useRef(0);
  const readGeneration = useRef(0);
  const saveGeneration = useRef(0);
  const previousContext = useRef(context);
  const [directory, setDirectory] = useState('');
  const [entries, setEntries] = useState<Entry[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [error, setError] = useState('');
  const [selected, setSelected] = useState<Entry | null>(restored?.selected ?? null);
  const [file, setFile] = useState<FileText | null>(restored?.file ?? null);
  const [fileContext, setFileContext] = useState<string | null>(restored ? context : null);
  const [draft, setDraft] = useState(restored?.text ?? '');
  const [fileError, setFileError] = useState(restored ? 'Unsaved draft restored. Saving will check whether the file changed.' : '');
  const [reading, setReading] = useState(false);
  const [saving, setSaving] = useState(!!pendingSave);
  const [saved, setSaved] = useState(false);

  const target = useCallback(() => ({ chatId, ...(targetDeviceId ? { targetDeviceId } : {}) }), [chatId, targetDeviceId]);
  const load = useCallback(async (path: string, next?: string) => {
    const current = next ? generation.current : ++generation.current;
    if (next) setLoadingMore(true); else { setLoading(true); setError(''); setEntries([]); setCursor(null); }
    try {
      const result = pageFrom(await callRef.current('ListWorkspaceDirectory', {
        ...target(), directory: path, includeIgnored: false, ...(next ? { cursor: next } : {}),
      }), path);
      if (current !== generation.current) return;
      setEntries(previous => next ? [...previous, ...result.entries.filter(entry => !previous.some(old => old.path === entry.path))] : result.entries);
      setCursor(result.nextCursor ?? null);
    } catch (cause) {
      if (current === generation.current) setError(errorText(cause));
    } finally {
      if (current === generation.current) { setLoading(false); setLoadingMore(false); }
    }
  }, [target]);

  useEffect(() => {
    if (previousContext.current === context) return;
    previousContext.current = context;
    readGeneration.current++;
    saveGeneration.current++;
    const cached = unsavedDrafts.get(context);
    setReading(false);
    setSaving(!!pendingSave);
    setSaved(false);
    setDirectory('');
    setSelected(cached?.selected ?? null);
    setFile(cached?.file ?? null);
    setFileContext(cached ? context : null);
    setDraft(cached?.text ?? '');
    setFileError(cached ? 'Unsaved draft restored. Saving will check whether the file changed.' : '');
  }, [context]);
  useEffect(() => {
    if (chatId && cwd) void load(directory);
    return () => { generation.current++; };
  }, [chatId, cwd, directory, load]);

  const openFile = useCallback(async (entry: Entry) => {
    const cached = unsavedDrafts.get(context);
    if (cached) {
      setSelected(cached.selected); setFile(cached.file); setFileContext(context); setDraft(cached.text);
      setFileError('Save or discard your changes before opening another file.');
      return;
    }
    const current = ++readGeneration.current;
    setSelected(entry); setFile(null); setFileContext(context); setDraft(''); setFileError(''); setReading(true); setSaved(false);
    try {
      const result = fileFrom(await callRef.current('ReadWorkspaceFile', { ...target(), path: entry.path }), entry.path);
      if (current !== readGeneration.current) return;
      setFile(result); setDraft(result.text ?? '');
    } catch (cause) {
      if (current === readGeneration.current) setFileError(errorText(cause));
    } finally {
      if (current === readGeneration.current) setReading(false);
    }
  }, [target, context]);

  const dirty = !!file && file.text !== draft;
  const writable = !!file && fileContext === context && !selected?.readOnly && !file.readOnlyReason && !file.truncated &&
    !!file.contentHash && !!file.checkoutId && (file.encoding === 'utf8' || file.encoding === 'utf8Bom') &&
    (file.lineEnding === 'lf' || file.lineEnding === 'crlf' || file.lineEnding === 'none');
  const closeFile = () => {
    if (dirty || saving) { setFileError('Save or discard your changes before leaving this file.'); return; }
    readGeneration.current++; setSelected(null); setFile(null); setFileError('');
  };
  const finishSave = (outcome: SaveOutcome) => {
    if (currentContext.current !== context) return;
    if (outcome.status === 'written') {
      const cached = unsavedDrafts.get(context);
      setFile(cached?.file ?? outcome.file);
      setDraft(cached?.text ?? outcome.file.text ?? '');
      setSaved(!cached);
    } else {
      setFileError(outcome.status === 'conflict'
        ? 'This file changed on the device. Your draft is preserved. Discard and reopen to see the latest version.'
        : outcome.message);
    }
    setSaving(false);
  };
  const save = async () => {
    if (!file || !selected || !writable || !dirty || saving || pendingSaves.has(context)) return;
    const current = ++saveGeneration.current;
    setSaving(true); setFileError(''); setSaved(false);
    const pending = Promise.resolve().then(() => callRef.current('WriteWorkspaceFile', {
        ...target(), expectedCheckoutId: file.checkoutId, path: file.path, text: draft,
        expectedContentHash: file.contentHash, encoding: file.encoding, lineEnding: file.lineEnding === 'none' ? 'lf' : file.lineEnding,
      })).then(value => {
      const result = record(value);
      if (result.status === 'conflict') return { status: 'conflict' } as SaveOutcome;
      if (result.status === 'written') {
        const written = record(result.file);
        if (typeof written.contentHash !== 'string') throw new Error('Invalid save response');
        const savedFile = { ...file, text: draft, contentHash: written.contentHash };
        const cached = unsavedDrafts.get(context);
        if (cached?.file.path === file.path && cached.file.contentHash === file.contentHash && cached.text === draft) {
          unsavedDrafts.delete(context);
        } else if (cached?.file.path === file.path && cached.file.contentHash === file.contentHash) {
          unsavedDrafts.set(context, { ...cached, file: savedFile });
        }
        return { status: 'written', file: savedFile } as SaveOutcome;
      }
      throw new Error('Invalid save response');
    }).catch(cause => ({ status: 'error', message: errorText(cause) } as SaveOutcome));
    pendingSaves.set(context, pending);
    const outcome = await pending;
    if (pendingSaves.get(context) === pending) pendingSaves.delete(context);
    if (current === saveGeneration.current) finishSave(outcome);
  };
  useEffect(() => {
    if (!pendingSave) return;
    let active = true;
    void pendingSave.then(outcome => { if (active) finishSave(outcome); });
    return () => { active = false; };
  }, [context, pendingSave]);

  if (!chatId || !cwd) return <View style={styles.center}><Text style={styles.muted}>No workspace is available for this chat.</Text></View>;

  if (selected) return <View style={styles.root}>
    <View style={styles.toolbar}>
      <Pressable accessibilityRole="button" onPress={closeFile} style={styles.toolbarButton}><Text style={styles.toolbarText}>‹ Files</Text></Pressable>
      <Text numberOfLines={1} style={styles.toolbarTitle}>{selected.name}</Text>
      {writable && <Pressable accessibilityRole="button" disabled={!dirty || saving} onPress={() => void save()} style={[styles.saveButton, (!dirty || saving) && styles.disabled]}>
        <Text style={styles.saveText}>{saving ? 'Saving…' : 'Save'}</Text>
      </Pressable>}
    </View>
    <Text numberOfLines={1} style={styles.path}>{selected.path}</Text>
    {reading ? <ActivityIndicator style={styles.busy} color={colors.accent} /> : file ? <>
      {file.text == null ? <View style={styles.center}><Text style={styles.muted}>Preview unavailable for this file.</Text></View> :
        <TextInput
          accessibilityLabel={`Contents of ${selected.name}`}
          multiline
          editable={writable && !saving}
          onChangeText={text => {
            if (saving || pendingSaves.has(context)) return;
            if (text === file.text) unsavedDrafts.delete(context);
            else if (!retainDraft(context, { selected, file, text })) {
              setFileError('Unsaved draft memory is full. Save or discard another draft before editing.');
              return;
            }
            setDraft(text); setSaved(false);
          }}
          value={draft}
          autoCapitalize="none"
          autoCorrect={false}
          scrollEnabled
          textAlignVertical="top"
          style={styles.editor}
        />}
      {!writable && <Text style={styles.notice}>Read only{fileContext !== context ? ' · workspace changed' : file.readOnlyReason ? ` · ${file.readOnlyReason}` : file.truncated ? ' · file too large' : ''}</Text>}
      {dirty && <Pressable accessibilityRole="button" disabled={saving} onPress={() => { unsavedDrafts.delete(fileContext ?? context); setDraft(file.text ?? ''); setFileError(''); }} style={styles.discard}><Text style={styles.muted}>Discard changes</Text></Pressable>}
      {saved && <Text style={styles.success}>Saved</Text>}
    </> : null}
    {!!fileError && <Text accessibilityLiveRegion="polite" style={styles.error}>{fileError}</Text>}
  </View>;

  return <View style={styles.root}>
    <View style={styles.toolbar}>
      {directory ? <Pressable accessibilityRole="button" onPress={() => setDirectory(parentOf(directory))} style={styles.toolbarButton}><Text style={styles.toolbarText}>‹ Back</Text></Pressable> : null}
      <Text numberOfLines={1} style={styles.toolbarTitle}>{directory ? directory.split('/').at(-1) : 'Files'}</Text>
      <Pressable accessibilityRole="button" accessibilityLabel="Refresh files" onPress={() => void load(directory)} style={styles.toolbarButton}><Text style={styles.toolbarText}>↻</Text></Pressable>
    </View>
    <Text numberOfLines={1} style={styles.path}>{cwd}{directory ? ` / ${directory}` : ''}</Text>
    {loading ? <ActivityIndicator style={styles.busy} color={colors.accent} /> : <FlatList
      data={entries}
      keyExtractor={entry => entry.path}
      contentContainerStyle={styles.list}
      renderItem={({ item }) => <Pressable accessibilityRole="button" onPress={() => item.kind === 'directory' ? setDirectory(item.path) : void openFile(item)} style={styles.entry}>
        <Text style={styles.icon}>{item.kind === 'directory' ? '▸' : item.kind === 'symlink' ? '↗' : '·'}</Text>
        <Text numberOfLines={1} style={[styles.name, item.ignored && styles.muted]}>{item.name}</Text>
        <Text style={styles.meta}>{item.kind === 'directory' ? '›' : fileSize(item.size)}</Text>
      </Pressable>}
      ListEmptyComponent={!error ? <Text style={styles.empty}>This folder is empty.</Text> : null}
      ListFooterComponent={cursor ? <Pressable accessibilityRole="button" disabled={loadingMore} onPress={() => void load(directory, cursor)} style={styles.more}><Text style={styles.toolbarText}>{loadingMore ? 'Loading…' : 'Load more'}</Text></Pressable> : null}
    />}
    {!!error && <Pressable accessibilityRole="button" onPress={() => void load(directory)} style={styles.errorBox}><Text style={styles.error}>{error} · Tap to retry</Text></Pressable>}
  </View>;
}

const styles = StyleSheet.create({
  root: { flex: 1, backgroundColor: colors.bg },
  center: { flex: 1, alignItems: 'center', justifyContent: 'center', padding: spacing.lg },
  toolbar: { minHeight: 48, flexDirection: 'row', alignItems: 'center', paddingHorizontal: spacing.md, borderBottomColor: colors.border, borderBottomWidth: 1, gap: spacing.sm },
  toolbarButton: { minWidth: 44, minHeight: 44, alignItems: 'center', justifyContent: 'center' },
  toolbarText: { color: colors.textMuted, fontSize: typography.body },
  toolbarTitle: { flex: 1, color: colors.text, fontSize: typography.body, fontWeight: '700' },
  saveButton: { backgroundColor: colors.text, borderRadius: radius.control, paddingHorizontal: spacing.md, paddingVertical: spacing.sm },
  disabled: { opacity: 0.4 },
  saveText: { color: colors.bg, fontSize: typography.small, fontWeight: '700' },
  path: { color: colors.textFaint, fontSize: typography.small, paddingHorizontal: spacing.lg, paddingVertical: spacing.sm },
  list: { paddingBottom: 24 },
  entry: { minHeight: 44, flexDirection: 'row', alignItems: 'center', paddingHorizontal: spacing.lg, gap: spacing.md, borderBottomColor: colors.border, borderBottomWidth: 1 },
  icon: { color: colors.textMuted, width: 18, fontSize: typography.body },
  name: { color: colors.text, fontSize: typography.body, flex: 1 },
  meta: { color: colors.textFaint, fontSize: typography.small },
  more: { alignItems: 'center', padding: spacing.lg },
  busy: { marginTop: 32 },
  empty: { color: colors.textFaint, fontSize: typography.body, textAlign: 'center', marginTop: 48 },
  editor: { flex: 1, color: colors.text, backgroundColor: colors.surfaceCard, fontFamily: typography.mono, fontSize: typography.small, lineHeight: 19, padding: spacing.lg },
  notice: { color: colors.textMuted, fontSize: typography.small, paddingHorizontal: spacing.lg, paddingVertical: spacing.sm },
  discard: { alignSelf: 'flex-start', paddingHorizontal: spacing.lg, paddingVertical: spacing.md },
  muted: { color: colors.textMuted, fontSize: typography.small },
  success: { color: colors.success, fontSize: typography.small, paddingHorizontal: spacing.lg, paddingBottom: spacing.sm },
  errorBox: { padding: spacing.lg },
  error: { color: colors.danger, fontSize: typography.small, paddingHorizontal: spacing.lg, paddingBottom: spacing.md },
});
