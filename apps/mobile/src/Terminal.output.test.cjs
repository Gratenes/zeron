const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { test } = require('node:test');
const vm = require('node:vm');
const ts = require('typescript');

test('terminal output keeps whole Unicode characters during write and cursor edits', () => {
  const source = fs.readFileSync(path.join(__dirname, 'Terminal.tsx'), 'utf8');
  const outputSource = source.slice(source.indexOf('function nextCharacter('), source.indexOf('function sessionFrom('));
  const code = ts.transpileModule(`${outputSource}\nexports.Output = Output;`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  }).outputText;
  const exports = {};
  vm.runInNewContext(code, { exports });
  const Output = exports.Output;

  const plain = new Output();
  assert.equal(plain.feed('hello 😀 world'), 'hello 😀 world');

  const overwrite = new Output();
  overwrite.feed('A😀B');
  assert.equal(overwrite.feed('\x1b[2DX'), 'AXB');

  const backspace = new Output();
  backspace.feed('A😀B');
  assert.equal(backspace.feed('\bZ'), 'A😀Z');

  const column = new Output();
  column.feed('A😀B');
  assert.equal(column.feed('\r\x1b[3GZ'), 'A😀Z');
});
