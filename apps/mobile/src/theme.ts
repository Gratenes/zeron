// Mirrors the default Kratos dark theme in crates/ui/src/theme.rs.
// React Native uses opaque colors for surfaces and RGBA for GPUI's washes.
export const colors = {
  bg: '#060606',
  surface: '#0d0d0d',
  surfaceRaised: '#1e1e1e',
  surfaceCard: '#0e0e0e',
  surfaceOverlay: '#161616',
  inputBg: 'rgba(255,255,255,0.03)',
  selected: 'rgba(235,235,235,0.11)',
  hover: 'rgba(235,235,235,0.08)',
  border: 'rgba(255,255,255,0.08)',
  borderStrong: 'rgba(255,255,255,0.14)',
  text: '#e5e5e5',
  textMuted: '#a3a3a3',
  textFaint: '#737373',
  textDim: '#989898',
  solid: '#e5e5e5',
  onSolid: '#0e0e0e',
  accent: '#818cf8',
  accentWash: 'rgba(79,70,229,0.45)',
  danger: '#fb7185',
  warning: '#fbbf24',
  success: '#34d399',
  codeText: '#818cf8',
  codeWash: 'rgba(129,140,248,0.12)',
} as const;

export const spacing = { xs: 4, sm: 8, md: 12, lg: 16 } as const;
export const radius = { control: 6, panel: 10, bubble: 16 } as const;
export const typography = {
  caption: 11,
  small: 12,
  body: 14,
  title: 18,
  heading: 24,
  mono: 'monospace',
} as const;
