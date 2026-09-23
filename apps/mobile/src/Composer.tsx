import { useRef, useState } from 'react';
import { Pressable, StyleSheet, TextInput, View } from 'react-native';
import { Text } from './AppText';
import { colors, radius, spacing, typography } from './theme';

export type QueuedMessage = { id: string; text: string };
export type ComposerSuggestion = { label: string; detail?: string };

type Props = {
  draft: string;
  onChangeDraft: (text: string) => void;
  onSubmit: (text: string, queue: boolean) => Promise<void> | void;
  expanded?: boolean;
  running?: boolean;
  busy?: boolean;
  disabled?: boolean;
  onInterrupt?: () => void;
  onAttach?: () => void;
  onRemoveQueued?: (id: string) => void;
  onEditQueued?: (id: string, text: string) => void;
  queue?: QueuedMessage[];
  commands?: ComposerSuggestion[];
  mentions?: ComposerSuggestion[];
  target?: string;
  project?: string;
  model?: string;
  notice?: string;
  error?: string;
};

export function Composer({
  draft, onChangeDraft, onSubmit, expanded = false, running = false, busy = false, disabled = false,
  onInterrupt, onAttach, onRemoveQueued, onEditQueued, queue = [], commands = [], mentions = [],
  target, project, model, notice, error,
}: Props) {
  const [editing, setEditing] = useState<string | null>(null);
  const [editText, setEditText] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [wrappedDraft, setWrappedDraft] = useState<string | null>(null);
  const latestDraft = useRef(draft);
  latestDraft.current = draft;
  const token = draft.match(/(?:^|\s)([\/@])([^\s]*)$/);
  const options = token?.[1] === '/' ? commands : token?.[1] === '@' ? mentions : [];
  const matches = token ? options.filter(option => option.label.toLowerCase().includes(token[2].toLowerCase())).slice(0, 6) : [];
  const canSend = !!draft.trim() && !disabled && !busy && !submitting;
  const expandedMode = expanded || draft.includes('\n') || (!!wrappedDraft && draft.startsWith(wrappedDraft));

  const send = async () => {
    if (!canSend) return;
    setSubmitting(true);
    try {
      await onSubmit(draft.trim(), running);
    } catch {
      // The owner shows the delivery error while preserving this draft.
    } finally {
      setSubmitting(false);
    }
  };

  const choose = (label: string) => {
    if (!token) return;
    onChangeDraft(draft.slice(0, token.index! + token[0].length - token[2].length) + label + ' ');
  };

  return (
    <View style={styles.wrap}>
      {!!error && <Text accessibilityLiveRegion="polite" style={styles.error}>{error}</Text>}
      {!!notice && <Text accessibilityLiveRegion="polite" style={styles.notice}>{notice}</Text>}
      {!!queue.length && <View style={styles.queue}>
        <Text style={styles.queueTitle}>Queued next · {queue.length}</Text>
        {queue.map(item => <View key={item.id} style={styles.queueRow}>
          {editing === item.id ? <>
            <TextInput accessibilityLabel="Edit queued message" multiline onChangeText={setEditText} style={styles.queueInput} value={editText} />
            <Action label="Save" onPress={() => { onEditQueued?.(item.id, editText); setEditing(null); }} />
            <Action label="Cancel" onPress={() => setEditing(null)} />
          </> : <>
            <Text numberOfLines={2} style={styles.queueText}>{item.text}</Text>
            {!!onEditQueued && <Action label="Edit" onPress={() => { setEditing(item.id); setEditText(item.text); }} />}
            {!!onRemoveQueued && <Action label="Remove" onPress={() => onRemoveQueued(item.id)} />}
          </>}
        </View>)}
      </View>}
      {!!matches.length && <View style={styles.suggestions}>
        {matches.map(item => <Pressable accessibilityRole="button" key={item.label} onPress={() => choose(item.label)} style={styles.suggestion}>
          <Text style={styles.suggestionLabel}>{token?.[1]}{item.label}</Text>
          {!!item.detail && <Text numberOfLines={1} style={styles.suggestionDetail}>{item.detail}</Text>}
        </Pressable>)}
      </View>}
      {!!(target || project) && <View style={styles.targetRow}>
        {!!target && <Text style={styles.targetChip}>{target}</Text>}
        {!!project && <Text style={styles.targetChip}>{project}</Text>}
      </View>}
      <View style={[styles.pill, !expandedMode && styles.compactPill]}>
        <TextInput
          accessibilityLabel="Message composer"
          editable={!disabled}
          multiline
          onChangeText={text => { latestDraft.current = text; onChangeDraft(text); }}
          onContentSizeChange={event => {
            if (!expandedMode && latestDraft.current && event.nativeEvent.contentSize.height > 48) setWrappedDraft(latestDraft.current);
          }}
          placeholder="Do anything…"
          placeholderTextColor={colors.textFaint}
          style={[styles.input, !expandedMode && styles.compactInput]}
          textAlignVertical={expandedMode ? 'top' : 'center'}
          value={draft}
        />
        <View style={[styles.actions, !expandedMode && styles.compactActions]}>
          <View style={styles.actionGroup}>
            {!!model && <Text numberOfLines={1} style={styles.model}>{model}</Text>}
            {!!onAttach && <Action label="Attach" onPress={onAttach} />}
          </View>
          {running && !draft.trim() && onInterrupt ? <Action label="Stop" onPress={onInterrupt} prominent /> :
            <Action label={running ? 'Queue' : 'Send'} onPress={send} disabled={!canSend} prominent />}
        </View>
      </View>
    </View>
  );
}

function Action({ label, onPress, disabled, prominent }: { label: string; onPress: () => void; disabled?: boolean; prominent?: boolean }) {
  return <Pressable accessibilityRole="button" accessibilityLabel={label} disabled={disabled} hitSlop={prominent ? 8 : undefined} onPress={onPress} style={[styles.action, prominent && styles.prominent, disabled && styles.disabled]}>
    <Text style={[styles.actionText, prominent && styles.prominentText, label === 'Stop' && styles.stopIcon]}>{prominent ? label === 'Stop' ? '■' : '↑' : label}</Text>
  </Pressable>;
}

const styles = StyleSheet.create({
  wrap: { width: '100%', maxWidth: 768, alignSelf: 'center', paddingHorizontal: spacing.lg, paddingBottom: spacing.lg },
  notice: { color: colors.textMuted, fontSize: typography.caption, marginBottom: spacing.sm, marginHorizontal: spacing.sm },
  error: { color: colors.danger, fontSize: typography.small, marginBottom: spacing.sm, marginHorizontal: spacing.sm },
  queue: { backgroundColor: colors.surfaceRaised, borderColor: colors.border, borderWidth: 1, borderTopLeftRadius: radius.panel, borderTopRightRadius: radius.panel, marginHorizontal: spacing.sm, padding: spacing.sm },
  queueTitle: { color: colors.textMuted, fontSize: typography.caption, fontWeight: '700', marginBottom: spacing.xs },
  queueRow: { alignItems: 'center', flexDirection: 'row', gap: spacing.xs, paddingVertical: spacing.xs },
  queueText: { color: colors.text, flex: 1, fontSize: typography.small },
  queueInput: { borderColor: colors.border, borderWidth: 1, color: colors.text, flex: 1, fontFamily: typography.family, fontSize: typography.small, minHeight: 36, padding: spacing.xs },
  suggestions: { backgroundColor: colors.surfaceRaised, borderColor: colors.border, borderRadius: radius.panel, borderWidth: 1, marginBottom: spacing.sm, overflow: 'hidden' },
  suggestion: { alignItems: 'center', flexDirection: 'row', gap: spacing.sm, paddingHorizontal: spacing.md, paddingVertical: spacing.sm },
  suggestionLabel: { color: colors.text, fontSize: typography.small },
  suggestionDetail: { color: colors.textMuted, flex: 1, fontSize: typography.caption },
  targetRow: { flexDirection: 'row', gap: spacing.xs, marginBottom: spacing.sm },
  targetChip: { borderColor: colors.border, borderRadius: radius.control, borderWidth: 1, color: colors.textMuted, fontSize: typography.small, overflow: 'hidden', paddingHorizontal: spacing.sm, paddingVertical: spacing.xs },
  pill: { backgroundColor: colors.inputBg, borderColor: colors.border, borderRadius: 26, borderWidth: 1, minHeight: 124 },
  compactPill: { alignItems: 'center', flexDirection: 'row', minHeight: 49 },
  input: { color: colors.text, fontFamily: typography.family, fontSize: typography.body, lineHeight: 21, maxHeight: 260, minHeight: 76, paddingHorizontal: spacing.lg, paddingTop: spacing.lg, paddingBottom: spacing.xs },
  compactInput: { flex: 1, maxHeight: 180, minHeight: 47, paddingTop: 10, paddingBottom: 10, paddingRight: spacing.sm },
  actions: { alignItems: 'center', flexDirection: 'row', justifyContent: 'space-between', minHeight: 46, paddingTop: spacing.xs, paddingBottom: 10, paddingHorizontal: spacing.md },
  compactActions: { gap: spacing.sm, minHeight: 47, paddingTop: 0, paddingBottom: 0, paddingLeft: spacing.xs, paddingRight: spacing.sm },
  actionGroup: { alignItems: 'center', flexDirection: 'row', gap: spacing.xs },
  model: { color: colors.textMuted, fontSize: typography.small, maxWidth: 120, paddingHorizontal: spacing.sm },
  action: { alignItems: 'center', borderRadius: radius.control, justifyContent: 'center', minHeight: 34, minWidth: 52, paddingHorizontal: spacing.sm },
  prominent: { backgroundColor: colors.solid, borderRadius: 14, height: 28, minHeight: 28, minWidth: 28, width: 28, paddingHorizontal: 0 },
  disabled: { opacity: 0.4 },
  actionText: { color: colors.textMuted, fontSize: typography.small, fontWeight: '600' },
  prominentText: { color: colors.onSolid, fontSize: 18, lineHeight: 22 },
  stopIcon: { fontSize: 11 },
});
