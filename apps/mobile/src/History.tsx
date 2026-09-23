import { useCallback, useEffect, useRef, useState } from 'react';
import { ActivityIndicator, FlatList, Pressable, StyleSheet, TextInput, View } from 'react-native';
import { Text } from './AppText';
import { colors, radius, spacing, typography } from './theme';

type Commit = {
  sha: string; subject: string; authorName: string; authorEmail: string; authoredAt: string;
  refs: { kind: string; label: string }[];
};
type Page = {
  commits: Commit[]; nextCursor: number | null; totalCount: number | null;
  headSha: string | null; headCommitCount: number | null;
  comparison: { base: string; ahead: number; behind: number } | null;
};
type Result = Page & { loading: boolean; error: string };
type Props = {
  call: (method: string, params: Record<string, unknown>) => Promise<unknown>;
  chatId: string;
  targetDeviceId?: string | null;
  cwd?: string | null;
};

const PAGE_SIZE = 100;
const emptyResult = (): Result => ({ commits: [], nextCursor: null, totalCount: null, headSha: null, headCommitCount: null, comparison: null, loading: false, error: '' });

function record(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('Invalid history response');
  return value as Record<string, unknown>;
}

function optionalCount(value: unknown): number | null {
  if (value == null) return null;
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < 0) throw new Error('Invalid history count');
  return value;
}

function pageFrom(value: unknown): Page {
  const data = record(value);
  if (!Array.isArray(data.commits)) throw new Error('Invalid history page');
  const commits = data.commits.map(raw => {
    const item = record(raw);
    if (typeof item.sha !== 'string' || typeof item.subject !== 'string' ||
        typeof item.authorName !== 'string' || typeof item.authorEmail !== 'string' ||
        typeof item.authoredAt !== 'string') throw new Error('Invalid history commit');
    const refs = Array.isArray(item.refs) ? item.refs.map(rawRef => {
      const ref = record(rawRef);
      if (typeof ref.kind !== 'string' || typeof ref.label !== 'string') throw new Error('Invalid history ref');
      return { kind: ref.kind, label: ref.label };
    }) : [];
    return { sha: item.sha, subject: item.subject, authorName: item.authorName,
      authorEmail: item.authorEmail, authoredAt: item.authoredAt, refs };
  });
  const comparison = data.comparison == null ? null : record(data.comparison);
  if (comparison && (typeof comparison.base !== 'string' || typeof comparison.ahead !== 'number' || typeof comparison.behind !== 'number')) throw new Error('Invalid branch comparison');
  if (data.headSha != null && typeof data.headSha !== 'string') throw new Error('Invalid HEAD');
  return {
    commits,
    nextCursor: optionalCount(data.nextCursor), totalCount: optionalCount(data.totalCount),
    headSha: data.headSha as string | null,
    headCommitCount: optionalCount(data.headCommitCount),
    comparison: comparison ? { base: comparison.base as string, ahead: comparison.ahead as number, behind: comparison.behind as number } : null,
  };
}

function errorText(error: unknown): string { return error instanceof Error ? error.message : 'History request failed'; }
function dateText(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
}

export function History({ call, chatId, targetDeviceId, cwd }: Props) {
  const callRef = useRef(call); callRef.current = call;
  const listRequest = useRef(0);
  const searchRequest = useRef(0);
  const [query, setQuery] = useState('');
  const [list, setList] = useState<Result>(emptyResult);
  const [search, setSearch] = useState<Result>(emptyResult);
  const searched = query.trim();
  const active = searched ? search : list;
  const target = useCallback(() => targetDeviceId ? { targetDeviceId } : {}, [targetDeviceId]);

  const load = useCallback(async (searchQuery: string, cursor = 0) => {
    if (!chatId || !cwd) return;
    const searching = !!searchQuery;
    const request = searching ? searchRequest : listRequest;
    const update = searching ? setSearch : setList;
    const current = cursor === 0 ? ++request.current : request.current;
    update(previous => ({ ...(cursor === 0 ? emptyResult() : previous), loading: true, error: '' }));
    try {
      const page = pageFrom(await callRef.current(searching ? 'SearchGitHistory' : 'ListGitHistory', {
        cwd, cursor, limit: PAGE_SIZE, ...target(), ...(searching ? { query: searchQuery } : {}),
      }));
      if (current !== request.current) return;
      update(previous => {
        const seen = new Set(previous.commits.map(commit => commit.sha));
        const commits = cursor === 0 ? page.commits : [...previous.commits, ...page.commits.filter(commit => !seen.has(commit.sha))];
        return {
          ...page, commits,
          headSha: page.headSha ?? previous.headSha,
          totalCount: page.totalCount ?? previous.totalCount,
          headCommitCount: page.headCommitCount ?? previous.headCommitCount,
          comparison: page.comparison ?? previous.comparison,
          loading: false, error: '',
        };
      });
    } catch (cause) {
      if (current === request.current) update(previous => ({ ...previous, loading: false, error: errorText(cause) }));
    }
  }, [chatId, cwd, target]);

  useEffect(() => {
    setQuery(''); setList(emptyResult()); setSearch(emptyResult());
    void load('', 0);
    return () => { listRequest.current++; searchRequest.current++; };
  }, [load]);

  useEffect(() => {
    searchRequest.current++;
    setSearch({ ...emptyResult(), loading: !!searched });
    if (!searched) return;
    const timer = setTimeout(() => void load(searched, 0), 120);
    return () => clearTimeout(timer);
  }, [searched, load]);

  if (!chatId || !cwd) return <View style={styles.center}><Text style={styles.muted}>No Git history is available for this chat.</Text></View>;

  const refresh = () => void load(searched, 0);
  const loadMore = () => { if (active.nextCursor != null && !active.loading) void load(searched, active.nextCursor); };
  return <View style={styles.root}>
    <View style={styles.toolbar}>
      <Text style={styles.title}>History</Text>
      <Pressable accessibilityRole="button" accessibilityLabel="Refresh history" onPress={refresh} style={styles.toolbarButton}><Text style={styles.toolbarText}>↻</Text></Pressable>
    </View>
    <Text numberOfLines={1} style={styles.path}>{cwd}</Text>
    <View style={styles.searchBox}>
      <TextInput
        accessibilityLabel="Search Git history"
        autoCapitalize="none"
        autoCorrect={false}
        onChangeText={setQuery}
        placeholder="Search commits or SHA"
        placeholderTextColor={colors.textFaint}
        returnKeyType="search"
        style={styles.searchInput}
        value={query}
      />
      {!!query && <Pressable accessibilityRole="button" accessibilityLabel="Clear history search" onPress={() => setQuery('')} style={styles.clearButton}><Text style={styles.toolbarText}>×</Text></Pressable>}
    </View>
    {!searched && <View style={styles.summary}>
      {list.headCommitCount != null && <Text style={styles.summaryText}>{list.headCommitCount} on HEAD</Text>}
      {list.comparison && <Text style={styles.summaryText}>vs {list.comparison.base} · ↑{list.comparison.ahead} ↓{list.comparison.behind}</Text>}
    </View>}
    {searched && search.totalCount != null && <Text style={styles.resultCount}>{search.totalCount} {search.totalCount === 1 ? 'result' : 'results'}</Text>}
    {active.loading && active.commits.length === 0 ? <ActivityIndicator style={styles.busy} color={colors.accent} /> : <FlatList
      data={active.commits}
      keyExtractor={commit => commit.sha}
      contentContainerStyle={styles.listContent}
      renderItem={({ item }) => <View style={styles.commit}>
        <View style={styles.graph}><View style={[styles.node, item.sha === list.headSha && styles.headNode]} /></View>
        <View style={styles.commitBody}>
          <Text selectable style={styles.subject}>{item.subject || '(no subject)'}</Text>
          <View style={styles.metadata}>
            <Text selectable style={styles.sha}>{item.sha.slice(0, 8)}</Text>
            <Text numberOfLines={1} style={styles.author}>{item.authorName || item.authorEmail}</Text>
            <Text style={styles.date}>{dateText(item.authoredAt)}</Text>
          </View>
          {!!item.refs.length && <View style={styles.refs}>{item.refs.map(ref => <Text key={`${ref.kind}:${ref.label}`} numberOfLines={1} style={styles.ref}>{ref.label}</Text>)}</View>}
        </View>
      </View>}
      ListEmptyComponent={!active.error ? <Text style={styles.empty}>{searched ? 'No matching commits.' : 'No commits in this checkout.'}</Text> : null}
      ListFooterComponent={active.nextCursor != null ? <Pressable accessibilityRole="button" disabled={active.loading} onPress={loadMore} style={styles.more}><Text style={styles.moreText}>{active.loading ? 'Loading…' : 'Load more commits'}</Text></Pressable> : null}
    />}
    {!!active.error && <Pressable accessibilityRole="button" onPress={() => active.nextCursor != null && active.commits.length ? loadMore() : refresh()} style={styles.errorBox}><Text style={styles.error}>{active.error} · Tap to retry</Text></Pressable>}
  </View>;
}

const styles = StyleSheet.create({
  root: { flex: 1, backgroundColor: colors.bg },
  center: { flex: 1, alignItems: 'center', justifyContent: 'center', padding: spacing.lg },
  toolbar: { minHeight: 48, flexDirection: 'row', alignItems: 'center', paddingHorizontal: spacing.lg, borderBottomColor: colors.border, borderBottomWidth: 1 },
  title: { flex: 1, color: colors.text, fontSize: typography.title, fontWeight: '700' },
  toolbarButton: { minHeight: 44, minWidth: 44, alignItems: 'center', justifyContent: 'center' },
  toolbarText: { color: colors.textMuted, fontSize: 22 },
  path: { color: colors.textFaint, fontSize: typography.small, paddingHorizontal: spacing.lg, paddingVertical: spacing.sm },
  searchBox: { flexDirection: 'row', alignItems: 'center', marginHorizontal: spacing.lg, marginBottom: spacing.sm, backgroundColor: colors.inputBg, borderWidth: 1, borderColor: colors.border, borderRadius: radius.control },
  searchInput: { flex: 1, color: colors.text, fontSize: typography.body, minHeight: 40, paddingHorizontal: spacing.md },
  clearButton: { minWidth: 40, minHeight: 40, alignItems: 'center', justifyContent: 'center' },
  summary: { flexDirection: 'row', flexWrap: 'wrap', gap: spacing.md, paddingHorizontal: spacing.lg, paddingBottom: spacing.sm },
  summaryText: { color: colors.textMuted, fontSize: typography.small },
  resultCount: { color: colors.textMuted, fontSize: typography.small, paddingHorizontal: spacing.lg, paddingBottom: spacing.sm },
  listContent: { paddingBottom: 24 },
  commit: { flexDirection: 'row', paddingHorizontal: spacing.lg, minHeight: 74 },
  graph: { width: 24, alignItems: 'center', paddingTop: spacing.lg },
  node: { width: 9, height: 9, borderRadius: 5, backgroundColor: colors.textFaint },
  headNode: { backgroundColor: colors.accent, borderWidth: 2, borderColor: colors.text },
  commitBody: { flex: 1, borderBottomWidth: 1, borderBottomColor: colors.border, paddingVertical: spacing.md, paddingLeft: spacing.sm, gap: spacing.xs },
  subject: { color: colors.text, fontSize: typography.body, lineHeight: 20 },
  metadata: { flexDirection: 'row', alignItems: 'center', gap: spacing.sm },
  sha: { color: colors.accent, fontFamily: typography.mono, fontSize: typography.small },
  author: { color: colors.textMuted, fontSize: typography.small, flex: 1 },
  date: { color: colors.textFaint, fontSize: typography.small },
  refs: { flexDirection: 'row', flexWrap: 'wrap', gap: spacing.xs },
  ref: { color: colors.textMuted, backgroundColor: colors.surfaceRaised, fontSize: typography.caption, paddingHorizontal: spacing.sm, paddingVertical: 2, borderRadius: radius.control, maxWidth: 190 },
  more: { alignItems: 'center', padding: spacing.lg },
  moreText: { color: colors.textMuted, fontSize: typography.body },
  busy: { marginTop: 32 },
  muted: { color: colors.textMuted, fontSize: typography.body, textAlign: 'center' },
  empty: { color: colors.textFaint, fontSize: typography.body, textAlign: 'center', marginTop: 48 },
  errorBox: { padding: spacing.lg },
  error: { color: colors.danger, fontSize: typography.small },
});
