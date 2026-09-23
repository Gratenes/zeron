import { Fragment } from 'react';
import { Linking, ScrollView, StyleSheet, View } from 'react-native';
import { Text } from '../AppText';
import { colors, radius, spacing, typography } from '../theme';

type Block = { type: 'text' | 'heading' | 'list' | 'quote' | 'code' | 'rule'; text: string; level?: number; language?: string };

function blocks(source: string): Block[] {
  const lines = source.split('\n');
  const result: Block[] = [];
  let paragraph: string[] = [];
  const flush = () => {
    if (paragraph.length) result.push({ type: 'text', text: paragraph.join('\n') });
    paragraph = [];
  };
  for (let index = 0; index < lines.length; index++) {
    const line = lines[index];
    const fence = /^\s*```([^`]*)$/.exec(line);
    if (fence) {
      flush();
      const code: string[] = [];
      while (++index < lines.length && !/^\s*```\s*$/.test(lines[index])) code.push(lines[index]);
      result.push({ type: 'code', text: code.join('\n'), language: fence[1].trim() });
    } else if (!line.trim()) {
      flush();
    } else if (/^\s*([-*+] |\d+\. )/.test(line)) {
      flush();
      result.push({ type: 'list', text: line.replace(/^\s*([-*+] |\d+\. )/, ''), language: /^\s*\d+\./.test(line) ? line.trim().split('.')[0] + '.' : '•' });
    } else if (/^\s*> ?/.test(line)) {
      flush();
      result.push({ type: 'quote', text: line.replace(/^\s*> ?/, '') });
    } else if (/^#{1,6} /.test(line)) {
      flush();
      const level = line.match(/^#+/)?.[0].length ?? 1;
      result.push({ type: 'heading', text: line.slice(level + 1), level });
    } else if (/^\s*(---+|\*\*\*+)\s*$/.test(line)) {
      flush();
      result.push({ type: 'rule', text: '' });
    } else {
      paragraph.push(line);
    }
  }
  flush();
  return result;
}

const inlinePattern = /\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)|`([^`]+)`|\*\*([^*]+)\*\*|\*([^*]+)\*|(?<!\()https?:\/\/[^\s<>]+/g;

function Inline({ text }: { text: string }) {
  const runs: React.ReactNode[] = [];
  let last = 0;
  for (const match of text.matchAll(inlinePattern)) {
    const start = match.index;
    if (start > last) runs.push(text.slice(last, start));
    const url = match[2] ?? (match[0].startsWith('http') ? match[0] : undefined);
    if (url) {
      runs.push(<Text key={start} style={styles.link} onPress={() => void Linking.openURL(url)}>{match[1] ?? match[0]}</Text>);
    } else if (match[3]) {
      runs.push(<Text key={start} style={styles.inlineCode}>{match[3]}</Text>);
    } else {
      runs.push(<Text key={start} style={match[4] ? styles.bold : styles.italic}>{match[4] ?? match[5]}</Text>);
    }
    last = start + match[0].length;
  }
  if (last < text.length) runs.push(text.slice(last));
  return <Fragment>{runs}</Fragment>;
}

export function Markdown({ source }: { source: string }) {
  return <View style={styles.blocks}>{blocks(source).map((block, index) => {
    if (block.type === 'code') return <View key={index} style={styles.codeFrame}>
      {!!block.language && <Text style={styles.codeLanguage}>{block.language}</Text>}
      <ScrollView horizontal showsHorizontalScrollIndicator={false} contentContainerStyle={styles.codeScroll}>
        <Text selectable style={styles.codeText}>{block.text}</Text>
      </ScrollView>
    </View>;
    if (block.type === 'rule') return <View key={index} style={styles.rule} />;
    if (block.type === 'list') return <View key={index} style={styles.listRow}>
      <Text style={styles.marker}>{block.language}</Text><Text selectable style={styles.body}><Inline text={block.text} /></Text>
    </View>;
    return <Text key={index} selectable style={[styles.body, block.type === 'heading' && styles.heading, block.type === 'quote' && styles.quote]}>
      <Inline text={block.text} />
    </Text>;
  })}</View>;
}

const styles = StyleSheet.create({
  blocks: { gap: spacing.md },
  body: { color: colors.text, fontSize: typography.body, lineHeight: 22, flexShrink: 1 },
  heading: { fontWeight: '700', fontSize: typography.title, lineHeight: 26 },
  quote: { color: colors.textMuted, borderLeftColor: colors.borderStrong, borderLeftWidth: 2, paddingLeft: spacing.md },
  listRow: { flexDirection: 'row', alignItems: 'flex-start', gap: spacing.sm },
  marker: { color: colors.textMuted, fontSize: typography.body, lineHeight: 22, minWidth: 18 },
  rule: { height: 1, backgroundColor: colors.border },
  codeFrame: { borderColor: colors.border, borderWidth: 1, borderRadius: radius.panel, backgroundColor: colors.inputBg, overflow: 'hidden' },
  codeLanguage: { color: colors.textMuted, fontSize: typography.small, borderBottomColor: colors.border, borderBottomWidth: 1, paddingHorizontal: spacing.md, paddingVertical: spacing.sm },
  codeScroll: { padding: spacing.md },
  codeText: { color: colors.codeText, fontFamily: typography.mono, fontSize: typography.small, lineHeight: 19 },
  inlineCode: { color: colors.codeText, backgroundColor: colors.inputBg, fontFamily: typography.mono },
  bold: { fontWeight: '700' },
  italic: { fontStyle: 'italic' },
  link: { color: colors.accent, textDecorationLine: 'underline' },
});
