const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { test } = require('node:test');
const ts = require('typescript');

const source = fs.readFileSync(path.join(__dirname, 'liveModel.ts'), 'utf8');
const code = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS } }).outputText;
const model = {};
new Function('exports', code)(model);

test('host input and error parts remain actionable and readable', () => {
  const questions = [{ id: 'q1', header: 'Choice', question: 'Which path?', options: ['A', 'B'], multiSelect: false }];
  const [entry] = model.renderEntries([{ id: 'entry', role: 'assistant', parts: [
    { id: 'request-1', kind: 'input', questions, resolved: false },
    { id: 'error-1', kind: 'error', message: 'The host stopped this run' },
  ] }]);
  assert.deepEqual(entry.parts, [
    { type: 'input', id: 'request-1', requestId: 'request-1', questions, resolved: false },
    { type: 'error', id: 'error-1', message: 'The host stopped this run' },
  ]);
  assert.equal(model.renderEntries([{ id: 'entry', role: 'assistant', parts: [
    { id: 'request-1', kind: 'input', questions, resolved: true },
  ] }])[0].parts[0].resolved, true);
});
