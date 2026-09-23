import { useEffect, useState, type ReactNode } from 'react';
import {
  Pressable,
  SafeAreaView,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
  useWindowDimensions,
} from 'react-native';
import { colors, radius, spacing, typography } from './theme';

export type SessionStatus = 'idle' | 'working' | 'awaitingInput' | 'completed' | 'queued' | 'errored';

export type SessionPreview = {
  id: string;
  title: string;
  spaceId?: string;
  spaceName?: string;
  deviceName?: string;
  branch?: string;
  harness?: string;
  timeLabel?: string;
  status?: SessionStatus;
  archived?: boolean;
};

export type SpacePreview = { id: string; name: string; deviceName?: string };
export type ShellSection = 'history' | 'files' | 'terminal' | 'changes' | 'browser';

type Props = {
  sessions: SessionPreview[];
  selectedId?: string;
  onSelectSession: (id: string) => void;
  onNewSession: () => void;
  onOpenSettings: () => void;
  children: ReactNode;
  spaces?: SpacePreview[];
  selectedSpaceId?: string | null;
  onOpenSection?: (section: ShellSection) => void;
  onSelectSpace?: (spaceId: string | null) => void;
  onAddSpace?: () => void;
  availableSections?: ShellSection[];
};

const sectionItems: { id: ShellSection; title: string; icon: string }[] = [
  { id: 'history', title: 'History', icon: '↶' },
  { id: 'files', title: 'Files', icon: '▣' },
  { id: 'terminal', title: 'Terminal', icon: '>_' },
  { id: 'changes', title: 'Changes', icon: '⇄' },
  { id: 'browser', title: 'Browser', icon: '◎' },
];

const statusLabel: Record<Exclude<SessionStatus, 'idle'>, string> = {
  working: 'Working',
  awaitingInput: 'Input',
  completed: 'Done',
  queued: 'Queued',
  errored: 'Failed',
};

const statusColor: Record<Exclude<SessionStatus, 'idle'>, string> = {
  working: colors.accent,
  awaitingInput: colors.warning,
  completed: colors.success,
  queued: colors.warning,
  errored: colors.danger,
};

export function Shell({
  sessions,
  selectedId,
  onSelectSession,
  onNewSession,
  onOpenSettings,
  children,
  spaces,
  selectedSpaceId,
  onOpenSection,
  onSelectSpace,
  onAddSpace,
  availableSections = ['history', 'files', 'terminal', 'changes', 'browser'],
}: Props) {
  const { width } = useWindowDimensions();
  const narrow = width < 720;
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [spacesOpen, setSpacesOpen] = useState(false);
  const [archivedOpen, setArchivedOpen] = useState(false);
  const [spaceId, setSpaceId] = useState<string | null>(null);
  useEffect(() => { if (selectedSpaceId !== undefined) setSpaceId(selectedSpaceId); }, [selectedSpaceId]);
  const [query, setQuery] = useState('');
  const availableSpaces = spaces ?? Array.from(
    new Map(sessions.filter(session => session.spaceId).map(session => [
      session.spaceId!,
      { id: session.spaceId!, name: session.spaceName ?? session.spaceId!, deviceName: session.deviceName },
    ])).values(),
  );
  const selectedSpace = availableSpaces.find(space => space.id === spaceId);
  const selectedSession = sessions.find(session => session.id === selectedId);
  const visible = sessions.filter(session =>
    (!spaceId || session.spaceId === spaceId) &&
    (!query || `${session.title} ${session.spaceName ?? ''} ${session.deviceName ?? ''}`.toLowerCase().includes(query.toLowerCase())),
  );
  const current = visible.filter(session => !session.archived);
  const archived = visible.filter(session => session.archived);

  const chooseSession = (id: string) => {
    onSelectSession(id);
    setDrawerOpen(false);
  };

  const sessionRow = (session: SessionPreview) => {
    const status = session.status ?? 'idle';
    const context = [session.spaceName ?? '~', session.deviceName].filter(Boolean).join(' @ ');
    return (
      <Pressable
        key={session.id}
        accessibilityRole="button"
        accessibilityLabel={`Open session ${session.title}`}
        accessibilityState={{ selected: selectedId === session.id }}
        onPress={() => chooseSession(session.id)}
        style={({ pressed }) => [styles.sessionRow, selectedId === session.id && styles.selectedRow, pressed && styles.pressedRow]}
      >
        <View style={styles.rowTop}>
          <Text numberOfLines={1} style={styles.context}>{context}</Text>
          {status === 'idle' ? (
            <Text style={styles.time}>{session.timeLabel ?? ''}</Text>
          ) : (
            <View style={styles.statusWrap}>
              <View style={[styles.statusDot, { backgroundColor: statusColor[status] }]} />
              <Text style={[styles.status, { color: statusColor[status] }]}>{statusLabel[status]}</Text>
            </View>
          )}
        </View>
        <Text numberOfLines={1} style={[styles.sessionTitle, selectedId === session.id && styles.selectedTitle]}>{session.title || 'New session'}</Text>
        {!!(session.branch || session.harness) && (
          <Text numberOfLines={1} style={styles.metadata}>{[session.harness, session.branch && `⌁ ${session.branch}`].filter(Boolean).join('  ·  ')}</Text>
        )}
      </Pressable>
    );
  };

  const sidebar = (
    <View style={styles.sidebar}>
      <View style={styles.brandRow}>
        <View style={styles.mark} accessibilityElementsHidden>
          {[colors.accent, '#6366f1', '#4f46e5'].map(color => (
            <View key={color} style={styles.markRow}>
              <View style={[styles.markCell, { backgroundColor: color }]} />
              <View style={[styles.markCell, { backgroundColor: color }]} />
            </View>
          ))}
        </View>
        <Text style={styles.brand}>Kratos</Text>
        {narrow && <Pressable accessibilityRole="button" accessibilityLabel="Close navigation" onPress={() => setDrawerOpen(false)} style={styles.iconButton}><Text style={styles.icon}>×</Text></Pressable>}
      </View>

      <View style={styles.sidebarContent}>
        <Pressable accessibilityRole="button" accessibilityLabel="New session" onPress={() => { onNewSession(); setDrawerOpen(false); }} style={styles.newButton}>
          <Text style={styles.newIcon}>＋</Text><Text style={styles.newLabel}>New session</Text>
        </Pressable>

        <Pressable accessibilityRole="button" accessibilityLabel="Choose project" accessibilityState={{ expanded: spacesOpen }} onPress={() => setSpacesOpen(!spacesOpen)} style={styles.spacePicker}>
          <Text style={styles.spaceIcon}>▱</Text>
          <Text numberOfLines={1} style={styles.spaceName}>{selectedSpace?.name ?? 'All projects'}</Text>
          <Text style={styles.chevron}>{spacesOpen ? '⌃' : '⌄'}</Text>
        </Pressable>
        {spacesOpen && (
          <View style={styles.spaceMenu}>
            <Pressable accessibilityRole="button" accessibilityState={{ selected: !spaceId }} onPress={() => { setSpaceId(null); onSelectSpace?.(null); setSpacesOpen(false); }} style={styles.spaceOption}><Text style={styles.spaceOptionText}>All projects</Text></Pressable>
            {availableSpaces.map(space => (
              <Pressable key={space.id} accessibilityRole="button" accessibilityState={{ selected: spaceId === space.id }} onPress={() => { setSpaceId(space.id); onSelectSpace?.(space.id); setSpacesOpen(false); }} style={styles.spaceOption}>
                <Text numberOfLines={1} style={styles.spaceOptionText}>{space.name}</Text>
                {!!space.deviceName && <Text numberOfLines={1} style={styles.spaceDevice}>@ {space.deviceName}</Text>}
              </Pressable>
            ))}
            {!!onAddSpace && <Pressable accessibilityRole="button" onPress={() => { onAddSpace(); setSpacesOpen(false); setDrawerOpen(false); }} style={styles.spaceOption}>
              <Text style={styles.spaceOptionText}>＋ Add project</Text>
            </Pressable>}
          </View>
        )}

        <TextInput accessibilityLabel="Search sessions" autoCapitalize="none" placeholder="Search sessions" placeholderTextColor={colors.textFaint} value={query} onChangeText={setQuery} style={styles.search} />
        <View style={styles.sectionHeading}><Text style={styles.sectionLabel}>Sessions</Text><View style={styles.sectionLine} /></View>
        <ScrollView style={styles.sessionList} contentContainerStyle={styles.sessionListContent} keyboardShouldPersistTaps="handled">
          {current.length ? current.map(sessionRow) : <Text style={styles.emptyList}>No sessions</Text>}
          {!!archived.length && (
            <>
              <Pressable accessibilityRole="button" accessibilityState={{ expanded: archivedOpen }} onPress={() => setArchivedOpen(!archivedOpen)} style={styles.archiveHeading}>
                <Text style={styles.sectionLabel}>Archived ({archived.length})</Text><Text style={styles.chevron}>{archivedOpen ? '⌃' : '⌄'}</Text>
              </Pressable>
              {archivedOpen && archived.map(sessionRow)}
            </>
          )}
        </ScrollView>

        {!!onOpenSection && (
          <View style={styles.sectionLinks}>
            {sectionItems.filter(item => availableSections.includes(item.id)).map(item => (
              <Pressable key={item.id} accessibilityRole="button" accessibilityLabel={`Open ${item.title}`} onPress={() => { onOpenSection(item.id); setDrawerOpen(false); }} style={styles.navRow}>
                <Text style={styles.navIcon}>{item.icon}</Text><Text style={styles.navText}>{item.title}</Text>
              </Pressable>
            ))}
          </View>
        )}
      </View>

      <Pressable accessibilityRole="button" accessibilityLabel="Open settings" onPress={() => { onOpenSettings(); setDrawerOpen(false); }} style={styles.settingsRow}>
        <Text style={styles.navIcon}>⚙</Text><Text style={styles.navText}>Settings</Text>
      </Pressable>
    </View>
  );

  return (
    <SafeAreaView style={styles.screen}>
      <View style={styles.layout}>
        {!narrow && <View style={styles.sidebarRail}>{sidebar}</View>}
        <View style={[styles.main, !narrow && styles.mainWide]}>
          <View style={styles.toolbar}>
            {narrow && <Pressable accessibilityRole="button" accessibilityLabel="Open navigation" accessibilityState={{ expanded: drawerOpen }} onPress={() => setDrawerOpen(true)} style={styles.iconButton}><Text style={styles.icon}>☰</Text></Pressable>}
            <View style={styles.toolbarTitleWrap}>
              <Text numberOfLines={1} style={styles.toolbarTitle}>{selectedSession?.title || 'New session'}</Text>
              {!!selectedSession?.spaceName && <Text numberOfLines={1} style={styles.toolbarSubtitle}>{selectedSession.spaceName}{selectedSession.deviceName ? ` @ ${selectedSession.deviceName}` : ''}</Text>}
            </View>
            <Pressable accessibilityRole="button" accessibilityLabel="New session" onPress={onNewSession} style={styles.iconButton}><Text style={styles.icon}>＋</Text></Pressable>
          </View>
          <View style={styles.content}>{children}</View>
        </View>
        {narrow && drawerOpen && (
          <View style={styles.drawerLayer}>
            <Pressable accessibilityRole="button" accessibilityLabel="Close navigation" onPress={() => setDrawerOpen(false)} style={styles.scrim} />
            <View style={[styles.drawer, { width: Math.min(width - 40, 320) }]}>{sidebar}</View>
          </View>
        )}
      </View>
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  screen: { flex: 1, backgroundColor: colors.surface },
  layout: { flex: 1, flexDirection: 'row' },
  sidebarRail: { width: 288 },
  sidebar: { flex: 1, backgroundColor: colors.surface },
  brandRow: { height: 48, flexDirection: 'row', alignItems: 'center', paddingHorizontal: spacing.lg, gap: spacing.sm },
  mark: { gap: 2 },
  markRow: { flexDirection: 'row', gap: 2 },
  markCell: { width: 4, height: 4, borderRadius: 1 },
  brand: { flex: 1, color: colors.text, fontSize: 15, fontWeight: '700', letterSpacing: -0.3 },
  sidebarContent: { flex: 1 },
  newButton: { height: 44, flexDirection: 'row', alignItems: 'center', gap: spacing.sm, marginHorizontal: spacing.sm, marginBottom: spacing.md, paddingHorizontal: spacing.sm, borderRadius: radius.control, backgroundColor: colors.selected },
  newIcon: { color: colors.text, fontSize: 19, lineHeight: 22 },
  newLabel: { color: colors.text, fontSize: typography.body, fontWeight: '500' },
  spacePicker: { height: 44, flexDirection: 'row', alignItems: 'center', gap: spacing.sm, marginHorizontal: spacing.sm, paddingHorizontal: spacing.sm, borderRadius: radius.control, borderWidth: 1, borderColor: colors.border },
  spaceIcon: { color: colors.textMuted, fontSize: 19 },
  spaceName: { flex: 1, color: colors.text, fontSize: typography.body },
  chevron: { color: colors.textMuted, fontSize: 15 },
  spaceMenu: { marginHorizontal: spacing.sm, marginTop: spacing.xs, paddingVertical: spacing.xs, borderWidth: 1, borderColor: colors.borderStrong, borderRadius: radius.panel, backgroundColor: colors.surfaceOverlay },
  spaceOption: { minHeight: 44, flexDirection: 'row', alignItems: 'center', gap: spacing.xs, paddingHorizontal: spacing.md },
  spaceOptionText: { flex: 1, color: colors.text, fontSize: typography.body },
  spaceDevice: { color: colors.textFaint, fontSize: typography.caption },
  search: { height: 36, marginHorizontal: spacing.sm, marginTop: spacing.md, paddingHorizontal: spacing.sm, borderWidth: 1, borderColor: colors.border, borderRadius: radius.control, color: colors.text, fontSize: typography.body },
  sectionHeading: { height: 36, flexDirection: 'row', alignItems: 'center', gap: spacing.sm, paddingHorizontal: spacing.lg },
  sectionLabel: { color: colors.textFaint, fontSize: typography.small, fontWeight: '600' },
  sectionLine: { flex: 1, height: 1, backgroundColor: colors.border },
  sessionList: { flex: 1 },
  sessionListContent: { paddingHorizontal: spacing.sm, paddingBottom: spacing.md, gap: 2 },
  emptyList: { color: colors.textFaint, fontSize: typography.small, padding: spacing.sm },
  sessionRow: { minHeight: 50, paddingHorizontal: spacing.sm, paddingVertical: 6, borderRadius: 8, gap: 3 },
  selectedRow: { backgroundColor: colors.selected },
  pressedRow: { backgroundColor: colors.hover },
  rowTop: { flexDirection: 'row', alignItems: 'center', gap: spacing.xs },
  context: { flex: 1, color: colors.textFaint, fontSize: typography.caption, lineHeight: 14 },
  time: { color: colors.textFaint, fontSize: 10, fontWeight: '500' },
  statusWrap: { flexDirection: 'row', alignItems: 'center', gap: spacing.xs },
  statusDot: { width: 6, height: 6, borderRadius: 3 },
  status: { fontSize: 10, fontWeight: '500' },
  sessionTitle: { color: colors.textMuted, fontSize: 13, lineHeight: 17 },
  selectedTitle: { color: colors.text, fontWeight: '500' },
  metadata: { color: colors.textFaint, fontSize: typography.caption, lineHeight: 14 },
  archiveHeading: { height: 36, flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between', marginTop: spacing.sm, paddingHorizontal: spacing.sm },
  sectionLinks: { borderTopWidth: 1, borderColor: colors.border, marginHorizontal: spacing.sm, paddingVertical: spacing.xs },
  navRow: { height: 44, flexDirection: 'row', alignItems: 'center', gap: spacing.sm, paddingHorizontal: spacing.sm, borderRadius: radius.control },
  navIcon: { width: 20, color: colors.textMuted, fontSize: 16, textAlign: 'center' },
  navText: { color: colors.textMuted, fontSize: 13 },
  settingsRow: { height: 44, flexDirection: 'row', alignItems: 'center', gap: spacing.sm, paddingHorizontal: spacing.lg, borderTopWidth: 1, borderColor: colors.border },
  main: { flex: 1, backgroundColor: colors.bg },
  mainWide: { margin: spacing.sm, marginLeft: 0, borderWidth: 1, borderColor: colors.border, borderRadius: radius.panel, overflow: 'hidden' },
  toolbar: { minHeight: 52, flexDirection: 'row', alignItems: 'center', gap: spacing.sm, paddingHorizontal: spacing.sm, borderBottomWidth: 1, borderColor: colors.border },
  toolbarTitleWrap: { flex: 1, minWidth: 0 },
  toolbarTitle: { color: colors.text, fontSize: typography.body, fontWeight: '600' },
  toolbarSubtitle: { color: colors.textFaint, fontSize: typography.caption, marginTop: 2 },
  iconButton: { width: 44, height: 44, alignItems: 'center', justifyContent: 'center', borderRadius: radius.control },
  icon: { color: colors.textMuted, fontSize: 21, lineHeight: 25 },
  content: { flex: 1 },
  drawerLayer: { ...StyleSheet.absoluteFill, flexDirection: 'row', zIndex: 10 },
  scrim: { ...StyleSheet.absoluteFill, backgroundColor: 'rgba(0,0,0,0.65)' },
  drawer: { height: '100%', borderRightWidth: 1, borderColor: colors.borderStrong },
});

export default Shell;
