import { Text as NativeText, StyleSheet, type TextProps } from 'react-native';
import { typography } from './theme';

const styles = StyleSheet.create({ text: { fontFamily: typography.family } });

export function Text({ style, ...props }: TextProps) {
  return <NativeText {...props} style={[styles.text, style]} />;
}
