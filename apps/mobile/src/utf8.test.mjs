import assert from 'node:assert/strict';
import { test } from 'node:test';

const samples = ['plain ASCII', 'Kratos — café', 'emoji 👩🏽‍💻', 'quote " and newline\n'];

for (const fallback of [false, true]) {
  test(`UTF-8 ${fallback ? 'fallback' : 'native'} round trips protocol text`, async () => {
    const nativeEncoder = globalThis.TextEncoder;
    const nativeDecoder = globalThis.TextDecoder;
    if (fallback) {
      globalThis.TextEncoder = undefined;
      globalThis.TextDecoder = undefined;
    }
    try {
      const { utf8Encode, utf8Decode } = await import(`./utf8.ts?fallback=${fallback}`);
      for (const sample of samples) {
        const bytes = utf8Encode(sample);
        assert.deepEqual([...bytes], [...Buffer.from(sample)]);
        assert.equal(utf8Decode(bytes), sample);
      }
    } finally {
      globalThis.TextEncoder = nativeEncoder;
      globalThis.TextDecoder = nativeDecoder;
    }
  });
}
