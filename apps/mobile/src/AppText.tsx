import { createContext, useContext } from 'react';
import { Text as NativeText, StyleSheet, type TextProps, type TextStyle } from 'react-native';
import { typography } from './theme';

type Font = { family: string; weight: number; italic: boolean };
const FontContext = createContext<Font>({ family: typography.family, weight: 0, italic: false });
const weights = ['', 'Medium', 'SemiBold', 'Bold'] as const;

function weightIndex(weight: TextStyle['fontWeight']): number {
  if (weight === 'bold') return 3;
  const value = Number(weight);
  return value >= 700 ? 3 : value >= 600 ? 2 : value >= 500 ? 1 : 0;
}

export function Text({ style, ...props }: TextProps) {
  const inherited = useContext(FontContext);
  const flattened = StyleSheet.flatten(style);
  const font: Font = {
    family: flattened?.fontFamily ?? inherited.family,
    weight: flattened?.fontWeight === undefined ? inherited.weight : weightIndex(flattened.fontWeight),
    italic: flattened?.fontStyle === undefined ? inherited.italic : flattened.fontStyle === 'italic',
  };
  const isGeist = font.family === typography.family || font.family === typography.mono;
  const family = isGeist ? `${font.family}${weights[font.weight]}${font.italic ? 'Italic' : ''}` : font.family;
  return <FontContext.Provider value={font}>
    <NativeText {...props} style={[style, { fontFamily: family, fontWeight: 'normal', fontStyle: 'normal' }]} />
  </FontContext.Provider>;
}
