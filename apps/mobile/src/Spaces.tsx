import { useEffect, useRef, useState } from 'react';
import { ActivityIndicator, FlatList, Pressable, StyleSheet, Text, TextInput, View } from 'react-native';
import { colors, radius, spacing, typography } from './theme';

type Folder = { name: string; isDir: boolean; isRepo: boolean };
type Listing = { path: string; entries: Folder[]; truncated: boolean };
type Props = {
  call: (method: string, params: Record<string, unknown>) => Promise<unknown>;
  hostDeviceId: string;
  onCreated: (spaceId: string, path: string) => void;
  onClose: () => void;
  existingSpaces?: { id: string; deviceId: string; path: string }[];
};

function listingFrom(value: unknown): Listing {
  if (!value || typeof value !== 'object') throw new Error('Invalid folder listing');
  const data = value as Record<string, unknown>;
  if (typeof data.path !== 'string' || !Array.isArray(data.entries)) throw new Error('Invalid folder listing');
  return {
    path: data.path,
    entries: data.entries.map(raw => {
      if (!raw || typeof raw !== 'object') throw new Error('Invalid folder entry');
      const entry = raw as Record<string, unknown>;
      if (typeof entry.name !== 'string' || !entry.name || typeof entry.isDir !== 'boolean' || typeof entry.isRepo !== 'boolean') throw new Error('Invalid folder entry');
      return { name: entry.name, isDir: entry.isDir, isRepo: entry.isRepo };
    }),
    truncated: data.truncated === true,
  };
}

function childPath(parent: string, child: string): string {
  const slash = parent.includes('\\') && !parent.includes('/') ? '\\' : '/';
  return parent.endsWith('/') || parent.endsWith('\\') ? parent + child : parent + slash + child;
}

function parentPath(path: string): string | null {
  const cut = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
  if (cut < 0) return null;
  if (cut === 0) return path.slice(0, 1);
  if (path[cut - 1] === ':') return path.slice(0, cut + 1);
  return path.slice(0, cut);
}

function newId(): string {
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}-${Math.random().toString(36).slice(2)}`;
}

function message(error: unknown): string { return error instanceof Error ? error.message : 'Could not browse folders'; }

export function Spaces({ call, hostDeviceId, onCreated, onClose, existingSpaces = [] }: Props) {
  const callRef = useRef(call);
  callRef.current = call;
  const generation = useRef(0);
  const [listing, setListing] = useState<Listing | null>(null);
  const [home, setHome] = useState<string | null>(null);
  const [query, setQuery] = useState('');
  const [pathInput, setPathInput] = useState('');
  const [currentRepo, setCurrentRepo] = useState(false);
  const [loading, setLoading] = useState(false);
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState('');

  const browse = async (path: string | null, repo = false) => {
    const current = ++generation.current;
    setLoading(true); setError(''); setQuery('');
    try {
      const result = listingFrom(await callRef.current('ListFolders', { ...(path ? { path } : {}), targetDeviceId: hostDeviceId }));
      if (current !== generation.current) return;
      setListing(result); setPathInput(result.path); setCurrentRepo(repo);
      if (path === null) setHome(result.path);
    } catch (cause) {
      if (current === generation.current) setError(message(cause));
    } finally {
      if (current === generation.current) setLoading(false);
    }
  };

  useEffect(() => {
    setListing(null); setHome(null); setCurrentRepo(false); setPathInput('');
    void browse(null);
    return () => { generation.current++; };
  }, [hostDeviceId]);

  const create = async () => {
    if (!listing || creating || loading) return;
    const existing = existingSpaces.find(space => space.deviceId === hostDeviceId && space.path === listing.path);
    if (existing) { onCreated(existing.id, listing.path); return; }
    setCreating(true); setError('');
    const spaceId = newId();
    try {
      await callRef.current('Mutate', {
        op: 'createSpace', spaceId, deviceId: hostDeviceId, path: listing.path, gitDetected: currentRepo,
      });
      onCreated(spaceId, listing.path);
    } catch (cause) { setError(message(cause)); }
    finally { setCreating(false); }
  };

  const folders = listing?.entries.filter(entry => entry.isDir && entry.name.toLowerCase().includes(query.trim().toLowerCase())) ?? [];
  const parent = listing ? parentPath(listing.path) : null;

  return <View style={styles.root}>
    <View style={styles.toolbar}>
      <Text style={styles.title}>Add project</Text>
      <Pressable accessibilityRole="button" accessibilityLabel="Close add project" onPress={onClose} style={styles.close}><Text style={styles.closeText}>×</Text></Pressable>
    </View>
    <Text style={styles.description}>Choose a folder on your Kratos device.</Text>
    <View style={styles.navigation}>
      <Pressable accessibilityRole="button" accessibilityLabel="Browse home folder" disabled={loading || !home || listing?.path === home} onPress={() => void browse(null)} style={[styles.navButton, (loading || !home || listing?.path === home) && styles.disabled]}><Text style={styles.navText}>Home</Text></Pressable>
      <Pressable accessibilityRole="button" accessibilityLabel="Browse parent folder" disabled={loading || !parent || parent === listing?.path} onPress={() => parent && void browse(parent)} style={[styles.navButton, (loading || !parent || parent === listing?.path) && styles.disabled]}><Text style={styles.navText}>↑ Parent</Text></Pressable>
      <Pressable accessibilityRole="button" accessibilityLabel="Refresh folders" disabled={loading} onPress={() => void browse(listing?.path ?? null, currentRepo)} style={[styles.navButton, loading && styles.disabled]}><Text style={styles.navText}>↻</Text></Pressable>
    </View>
    <View style={styles.pathRow}>
      <TextInput accessibilityLabel="Folder path on host device" autoCapitalize="none" autoCorrect={false} value={pathInput} onChangeText={setPathInput} onSubmitEditing={() => void browse(pathInput.trim() || null)} placeholder="Folder path" placeholderTextColor={colors.textFaint} style={styles.pathInput} />
      <Pressable accessibilityRole="button" accessibilityLabel="Browse entered path" disabled={loading || !pathInput.trim()} onPress={() => void browse(pathInput.trim())} style={[styles.goButton, (loading || !pathInput.trim()) && styles.disabled]}><Text style={styles.goText}>Go</Text></Pressable>
    </View>
    <TextInput accessibilityLabel="Filter folders" autoCapitalize="none" autoCorrect={false} value={query} onChangeText={setQuery} placeholder="Filter folders" placeholderTextColor={colors.textFaint} style={styles.search} />
    {loading ? <ActivityIndicator color={colors.accent} style={styles.spinner} /> :
      <FlatList
        data={folders}
        keyExtractor={entry => entry.name}
        keyboardShouldPersistTaps="handled"
        renderItem={({ item }) => <Pressable accessibilityRole="button" accessibilityLabel={`Open folder ${item.name}`} onPress={() => listing && void browse(childPath(listing.path, item.name), item.isRepo)} style={({ pressed }) => [styles.folder, pressed && styles.pressed]}>
          <Text style={styles.folderIcon}>▱</Text>
          <Text numberOfLines={1} style={styles.folderName}>{item.name}</Text>
          {item.isRepo && <Text style={styles.repo}>Git</Text>}
          <Text style={styles.arrow}>›</Text>
        </Pressable>}
        ListEmptyComponent={listing && !error ? <Text style={styles.empty}>{query ? 'No matching folders.' : 'No folders here.'}</Text> : null}
        ListFooterComponent={listing?.truncated ? <Text style={styles.notice}>Showing the first folders returned by this device. Enter a path above to browse elsewhere.</Text> : null}
        contentContainerStyle={styles.list}
      />}
    {!!error && <Text accessibilityLiveRegion="polite" style={styles.error}>{error}</Text>}
    <View style={styles.footer}>
      <Text numberOfLines={2} style={styles.selectedPath}>{listing?.path ?? 'Choose a folder'}</Text>
      <Pressable accessibilityRole="button" accessibilityLabel="Add current folder as project" disabled={!listing || loading || creating} onPress={() => void create()} style={[styles.addButton, (!listing || loading || creating) && styles.disabled]}>
        <Text style={styles.addText}>{creating ? 'Adding…' : 'Add this folder'}</Text>
      </Pressable>
    </View>
  </View>;
}

const styles = StyleSheet.create({
  root: { flex: 1, backgroundColor: colors.bg },
  toolbar: { minHeight: 52, flexDirection: 'row', alignItems: 'center', paddingHorizontal: spacing.lg, borderBottomColor: colors.border, borderBottomWidth: 1 },
  title: { flex: 1, color: colors.text, fontSize: typography.title, fontWeight: '700' },
  close: { width: 44, height: 44, alignItems: 'center', justifyContent: 'center' },
  closeText: { color: colors.textMuted, fontSize: 25 },
  description: { color: colors.textMuted, fontSize: typography.body, paddingHorizontal: spacing.lg, paddingTop: spacing.md },
  navigation: { flexDirection: 'row', gap: spacing.sm, paddingHorizontal: spacing.lg, paddingTop: spacing.md },
  navButton: { minWidth: 64, minHeight: 44, alignItems: 'center', justifyContent: 'center', paddingHorizontal: spacing.sm, borderColor: colors.border, borderWidth: 1, borderRadius: radius.control },
  navText: { color: colors.textMuted, fontSize: typography.body },
  disabled: { opacity: 0.4 },
  pathRow: { flexDirection: 'row', gap: spacing.sm, paddingHorizontal: spacing.lg, paddingTop: spacing.md },
  pathInput: { flex: 1, minWidth: 0, height: 44, paddingHorizontal: spacing.md, borderColor: colors.borderStrong, borderWidth: 1, borderRadius: radius.control, backgroundColor: colors.inputBg, color: colors.text, fontFamily: typography.mono, fontSize: typography.small },
  goButton: { minWidth: 44, height: 44, alignItems: 'center', justifyContent: 'center', borderRadius: radius.control, backgroundColor: colors.surfaceRaised },
  goText: { color: colors.text, fontSize: typography.body, fontWeight: '600' },
  search: { height: 44, margin: spacing.lg, marginBottom: spacing.sm, paddingHorizontal: spacing.md, borderColor: colors.border, borderWidth: 1, borderRadius: radius.control, color: colors.text, fontSize: typography.body },
  spinner: { marginTop: 32 },
  list: { paddingBottom: spacing.lg },
  folder: { minHeight: 52, flexDirection: 'row', alignItems: 'center', gap: spacing.sm, paddingHorizontal: spacing.lg, borderBottomColor: colors.border, borderBottomWidth: 1 },
  pressed: { backgroundColor: colors.hover },
  folderIcon: { color: colors.textMuted, fontSize: 22, width: 24 },
  folderName: { flex: 1, color: colors.text, fontSize: typography.body },
  repo: { color: colors.textMuted, fontSize: typography.caption, paddingHorizontal: spacing.sm, paddingVertical: spacing.xs, borderRadius: radius.control, backgroundColor: colors.surfaceRaised },
  arrow: { color: colors.textFaint, fontSize: 23 },
  empty: { color: colors.textFaint, fontSize: typography.body, textAlign: 'center', marginTop: spacing.lg },
  notice: { color: colors.textFaint, fontSize: typography.small, padding: spacing.lg },
  error: { color: colors.danger, fontSize: typography.small, paddingHorizontal: spacing.lg, paddingBottom: spacing.sm },
  footer: { gap: spacing.sm, padding: spacing.lg, borderTopColor: colors.border, borderTopWidth: 1 },
  selectedPath: { color: colors.textMuted, fontSize: typography.small },
  addButton: { minHeight: 44, alignItems: 'center', justifyContent: 'center', borderRadius: radius.control, backgroundColor: colors.solid },
  addText: { color: colors.onSolid, fontSize: typography.body, fontWeight: '700' },
});
