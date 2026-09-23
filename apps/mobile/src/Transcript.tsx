import { memo, useCallback, useRef, useState } from 'react';
import { FlatList, Linking, Pressable, StyleSheet, Text, View, type NativeScrollEvent, type NativeSyntheticEvent } from 'react-native';
import { colors, radius, spacing, typography } from './theme';
import { Markdown } from './transcript/Markdown';

export type TranscriptPart =
  | { type: 'text'; text: string }
  | { type: 'reasoning'; text: string }
  | { type: 'tool'; id: string; name: string; input?: string; output?: string; status?: 'running' | 'complete' | 'error' }
  | { type: 'artifact'; id: string; title: string; kind?: string; uri?: string };

export type TranscriptItem = {
  id: string;
  role: 'user' | 'assistant' | 'system';
  parts: TranscriptPart[];
  status?: 'streaming' | 'complete' | 'error';
};

type ArtifactPart = Extract<TranscriptPart, { type: 'artifact' }>;

function ArtifactCard({ part, onOpen, attached = false }: { part: ArtifactPart; onOpen?: (artifact: ArtifactPart) => void; attached?: boolean }) {
  const target = part.uri;
  const press = onOpen ? () => onOpen(part) : target && /^https?:\/\//.test(target) ? () => void Linking.openURL(target) : undefined;
  return <Pressable accessibilityRole={press ? 'link' : undefined} disabled={!press} onPress={press} style={attached ? styles.userArtifact : styles.artifact}>
    <Text style={styles.artifactKind}>{part.kind ?? (attached ? 'Attachment' : 'Artifact')}</Text>
    <Text style={styles.artifactTitle}>{part.title}</Text>
  </Pressable>;
}

function Collapsible({ label, text, streaming = false }: { label: string; text: string; streaming?: boolean }) {
  const [openOverride, setOpenOverride] = useState<boolean | null>(null);
  const open = openOverride ?? streaming;
  return <View style={styles.detailGroup}>
    <Pressable accessibilityRole="button" accessibilityState={{ expanded: open }} onPress={() => setOpenOverride(!open)} style={styles.detailHeader}>
      <Text style={styles.chevron}>{open ? '⌄' : '›'}</Text><Text style={styles.detailLabel}>{label}</Text>
    </Pressable>
    {open && <Text selectable style={styles.detailText}>{text}</Text>}
  </View>;
}

function Tool({ part }: { part: Extract<TranscriptPart, { type: 'tool' }> }) {
  const [open, setOpen] = useState(false);
  const hasDetail = !!part.input || !!part.output;
  return <View style={styles.tool}>
    <Pressable accessibilityRole="button" accessibilityState={{ expanded: open }} disabled={!hasDetail} onPress={() => setOpen(!open)} style={styles.toolHeader}>
      <Text style={[styles.toolDot, part.status === 'error' && styles.toolDotError]}>{part.status === 'running' ? '◌' : part.status === 'error' ? '!' : '✓'}</Text>
      <Text numberOfLines={1} style={styles.toolName}>{part.name}</Text>
      {hasDetail && <Text style={styles.chevron}>{open ? '⌄' : '›'}</Text>}
    </Pressable>
    {open && <View style={styles.toolDetails}>
      {!!part.input && <Text selectable style={styles.detailText}><Text style={styles.detailTitle}>Invocation\n</Text>{part.input}</Text>}
      {!!part.output && <Text selectable style={styles.detailText}><Text style={styles.detailTitle}>Output\n</Text>{part.output}</Text>}
    </View>}
  </View>;
}

function ToolGroup({ parts, streaming }: { parts: Extract<TranscriptPart, { type: 'tool' }>[]; streaming: boolean }) {
  const [openOverride, setOpenOverride] = useState<boolean | null>(null);
  const open = openOverride ?? streaming;
  const failures = parts.filter(part => part.status === 'error').length;
  return <View style={styles.activity}>
    <Pressable accessibilityRole="button" accessibilityState={{ expanded: open }} onPress={() => setOpenOverride(!open)} style={styles.activityHeader}>
      <Text style={styles.chevron}>{open ? '⌄' : '›'}</Text>
      <Text style={styles.activitySummary}>Called {parts.length} {parts.length === 1 ? 'tool' : 'tools'}{failures ? ` · ${failures} failed` : ''}</Text>
    </Pressable>
    {open && parts.map(part => <Tool key={part.id} part={part} />)}
  </View>;
}

const MessageRow = memo(function MessageRow({ item, onOpenArtifact }: { item: TranscriptItem; onOpenArtifact?: (artifact: ArtifactPart) => void }) {
  const [expanded, setExpanded] = useState(false);
  if (item.role === 'user') {
    const text = item.parts.filter((part): part is Extract<TranscriptPart, { type: 'text' }> => part.type === 'text').map(part => part.text).join('\n');
    const artifacts = item.parts.filter((part): part is Extract<TranscriptPart, { type: 'artifact' }> => part.type === 'artifact');
    const collapsible = text.length > 400 || text.split('\n').length > 5;
    return <View style={styles.userRow}>
      <View style={styles.userBubble}>
        <Text selectable numberOfLines={collapsible && !expanded ? 5 : undefined} style={styles.userText}>{text}</Text>
        {collapsible && <Pressable accessibilityRole="button" accessibilityState={{ expanded }} onPress={() => setExpanded(!expanded)}>
          <Text style={styles.expand}>{expanded ? 'Show less  ⌃' : 'Show more  ⌄'}</Text>
        </Pressable>}
        {artifacts.map(part => <ArtifactCard key={part.id} part={part} onOpen={onOpenArtifact} attached />)}
      </View>
    </View>;
  }

  const parts = item.parts;
  const elements: React.ReactNode[] = [];
  for (let i = 0; i < parts.length; i++) {
    const part = parts[i];
    if (part.type === 'tool') {
      const group: Extract<TranscriptPart, { type: 'tool' }>[] = [part];
      while (parts[i + 1]?.type === 'tool') group.push(parts[++i] as Extract<TranscriptPart, { type: 'tool' }>);
      elements.push(<ToolGroup key={part.id} parts={group} streaming={item.status === 'streaming'} />);
    } else if (part.type === 'text') {
      elements.push(<Markdown key={i} source={part.text} />);
    } else if (part.type === 'reasoning') {
      elements.push(<Collapsible key={i} label="Thinking" text={part.text} streaming={item.status === 'streaming'} />);
    } else {
      elements.push(<ArtifactCard key={part.id} part={part} onOpen={onOpenArtifact} />);
    }
  }
  return <View style={styles.assistantRow}>
    {item.role === 'system' && <Text style={styles.systemLabel}>SYSTEM</Text>}
    {elements}
    {item.status === 'error' && <Text style={styles.error}>Response failed</Text>}
    {item.status === 'streaming' && <Text accessibilityLiveRegion="polite" style={styles.streaming}>●  Working</Text>}
  </View>;
});

export function Transcript({ messages, isStreaming = false, onOpenArtifact }: { messages: TranscriptItem[]; isStreaming?: boolean; onOpenArtifact?: (artifact: ArtifactPart) => void }) {
  const list = useRef<FlatList<TranscriptItem>>(null);
  const pinned = useRef(true);
  const [showEnd, setShowEnd] = useState(false);
  const onScroll = useCallback((event: NativeSyntheticEvent<NativeScrollEvent>) => {
    const { contentOffset, contentSize, layoutMeasurement } = event.nativeEvent;
    const distance = contentSize.height - layoutMeasurement.height - contentOffset.y;
    pinned.current = distance <= 70;
    setShowEnd(distance > 320);
  }, []);
  const follow = useCallback(() => {
    if (pinned.current) requestAnimationFrame(() => list.current?.scrollToEnd({ animated: false }));
  }, []);
  const toEnd = useCallback(() => {
    pinned.current = true;
    list.current?.scrollToEnd({ animated: true });
    setShowEnd(false);
  }, []);
  return <View style={styles.root}>
    <FlatList
      ref={list}
      data={messages}
      keyExtractor={item => item.id}
      renderItem={({ item }) => <View style={styles.rowWidth}><MessageRow item={item} onOpenArtifact={onOpenArtifact} /></View>}
      onScroll={onScroll}
      onContentSizeChange={follow}
      scrollEventThrottle={32}
      keyboardShouldPersistTaps="handled"
      contentContainerStyle={styles.content}
      ListFooterComponent={isStreaming && messages.at(-1)?.status !== 'streaming' ? <Text style={styles.streaming}>●  Working</Text> : null}
      ListEmptyComponent={<Text style={styles.empty}>Start a conversation</Text>}
    />
    {showEnd && <Pressable accessibilityRole="button" accessibilityLabel="Scroll to latest message" onPress={toEnd} style={styles.endButton}><Text style={styles.endText}>↓</Text></Pressable>}
  </View>;
}

const styles = StyleSheet.create({
  root: { flex: 1, backgroundColor: colors.bg },
  content: { paddingHorizontal: spacing.lg, paddingTop: spacing.lg, paddingBottom: 24, alignItems: 'center' },
  rowWidth: { width: '100%', maxWidth: 736 },
  userRow: { alignItems: 'flex-end', marginBottom: 24 },
  userBubble: { maxWidth: '92%', backgroundColor: colors.surfaceRaised, borderColor: colors.border, borderWidth: 1, borderRadius: radius.bubble, paddingHorizontal: spacing.lg, paddingVertical: spacing.md },
  userText: { color: colors.text, fontSize: typography.body, lineHeight: 22 },
  expand: { color: colors.textMuted, fontSize: typography.small, paddingTop: spacing.sm },
  assistantRow: { gap: spacing.md, marginBottom: 24, paddingRight: spacing.sm },
  systemLabel: { color: colors.textFaint, fontSize: typography.caption, letterSpacing: 1 },
  streaming: { color: colors.textMuted, fontSize: typography.small, marginTop: spacing.sm },
  error: { color: colors.danger, fontSize: typography.small },
  activity: { gap: 1 },
  activityHeader: { flexDirection: 'row', alignItems: 'center', minHeight: 28, gap: spacing.sm },
  activitySummary: { color: colors.textMuted, fontSize: typography.small },
  chevron: { color: colors.textMuted, fontSize: 18, width: 20, textAlign: 'center' },
  tool: { marginLeft: 8, borderLeftWidth: 1, borderLeftColor: colors.border, paddingLeft: spacing.md },
  toolHeader: { flexDirection: 'row', alignItems: 'center', minHeight: 36, gap: spacing.sm },
  toolDot: { color: colors.textFaint, width: 16, textAlign: 'center', fontSize: typography.small },
  toolDotError: { color: colors.danger },
  toolName: { color: colors.textMuted, fontSize: typography.small, flex: 1 },
  toolDetails: { gap: spacing.md, paddingBottom: spacing.md },
  detailGroup: { borderLeftWidth: 1, borderLeftColor: colors.border, paddingLeft: spacing.md },
  detailHeader: { flexDirection: 'row', alignItems: 'center', minHeight: 30, gap: spacing.sm },
  detailLabel: { color: colors.textMuted, fontSize: typography.small },
  detailText: { color: colors.textMuted, fontFamily: 'monospace', fontSize: typography.small, lineHeight: 18 },
  detailTitle: { color: colors.textFaint, fontWeight: '700' },
  artifact: { borderWidth: 1, borderColor: colors.border, borderRadius: radius.panel, backgroundColor: colors.surfaceCard, padding: spacing.md },
  userArtifact: { borderWidth: 1, borderColor: colors.borderStrong, borderRadius: radius.control, backgroundColor: colors.surfaceCard, padding: spacing.sm, marginTop: spacing.sm },
  artifactKind: { color: colors.textFaint, fontSize: typography.caption, textTransform: 'uppercase' },
  artifactTitle: { color: colors.text, fontSize: typography.body, marginTop: spacing.xs },
  endButton: { position: 'absolute', right: spacing.lg, bottom: spacing.lg, width: 38, height: 38, borderRadius: 19, alignItems: 'center', justifyContent: 'center', backgroundColor: colors.surfaceRaised, borderColor: colors.borderStrong, borderWidth: 1 },
  endText: { color: colors.text, fontSize: 21 },
  empty: { color: colors.textFaint, fontSize: typography.body, marginTop: 80 },
});
