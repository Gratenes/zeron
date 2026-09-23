const encoder = typeof TextEncoder === 'undefined' ? null : new TextEncoder();
const decoder = typeof TextDecoder === 'undefined' ? null : new TextDecoder();

export function utf8Encode(text: string): Uint8Array {
  if (encoder) return encoder.encode(text);
  const escaped = encodeURIComponent(text);
  const bytes = new Uint8Array(escaped.length);
  let count = 0;
  for (let index = 0; index < escaped.length; index++) {
    if (escaped[index] === '%') {
      bytes[count++] = Number.parseInt(escaped.slice(index + 1, index + 3), 16);
      index += 2;
    } else bytes[count++] = escaped.charCodeAt(index);
  }
  return bytes.subarray(0, count);
}

export function utf8Decode(bytes: Uint8Array): string {
  if (decoder) return decoder.decode(bytes);
  let escaped = '';
  for (const byte of bytes) escaped += `%${byte.toString(16).padStart(2, '0')}`;
  return decodeURIComponent(escaped);
}

