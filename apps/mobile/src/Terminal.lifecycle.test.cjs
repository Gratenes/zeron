const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { test } = require('node:test');
const Module = require('node:module');
const ts = require('typescript');

test('terminal navigation detaches and resumes; Close terminates the host shell', async () => {
  let current;
  const react = {
    useRef(value) { return current.slot(() => ({ current: value })); },
    useState(value) {
      const slot = current.slot(() => ({ value }));
      return [slot.value, next => { slot.value = typeof next === 'function' ? next(slot.value) : next; }];
    },
    useEffect(effect) { if (current.first) current.effects.push(effect); },
  };
  const jsx = (type, props) => ({ type, props });
  const source = fs.readFileSync(path.join(__dirname, 'Terminal.tsx'), 'utf8');
  const code = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX },
  }).outputText;
  const file = new Module(path.join(__dirname, 'Terminal.tsx'), module);
  file.filename = path.join(__dirname, 'Terminal.tsx');
  file.paths = module.paths;
  file.require = name => {
    if (name === 'react') return react;
    if (name === 'react/jsx-runtime') return { jsx, jsxs: jsx, Fragment: 'Fragment' };
    if (name === 'react-native') return { StyleSheet: { create: value => value } };
    if (name === './connection') return { utf8Decode: bytes => new TextDecoder('utf-8', { fatal: true }).decode(bytes), utf8Encode: text => new TextEncoder().encode(text) };
    if (name === './theme') return { colors: {}, radius: {}, spacing: {}, typography: {} };
    throw new Error(`Unexpected import: ${name}`);
  };
  file._compile(code, file.filename);

  const calls = [];
  const streams = [];
  const owner = {};
  const props = {
    owner, chatId: 'lifecycle-test-chat', targetDeviceId: 'host',
    call: async (method, params) => {
      calls.push({ method, params });
      if (method === 'OpenTerminal') return { id: `shell-${calls.filter(call => call.method === 'OpenTerminal').length}`, cwd: '/', shell: 'sh' };
      return { ok: true };
    },
    subscribe: async (method, params, onItem) => {
      const stream = { method, params, onItem, cancelled: false };
      streams.push(stream);
      return () => { stream.cancelled = true; };
    },
  };
  const mount = () => {
    const instance = {
      slots: [], effects: [], first: true, index: 0,
      slot(create) { return this.slots[this.index++] ??= create(); },
      render() { current = this; this.index = 0; const tree = file.exports.Terminal(props); this.first = false; return tree; },
    };
    instance.render();
    const cleanup = instance.effects[0]();
    return { render: () => instance.render(), unmount: cleanup };
  };
  const tick = () => new Promise(resolve => setImmediate(resolve));
  const find = (node, label) => {
    if (!node || typeof node !== 'object') return undefined;
    if (Array.isArray(node)) return node.map(child => find(child, label)).find(Boolean);
    if (node.props?.accessibilityLabel === label) return node;
    return find(node.props?.children, label);
  };

  const first = mount();
  await tick();
  assert.equal(calls.filter(call => call.method === 'OpenTerminal').length, 1);
  streams[0].onItem({ type: 'data', seq: 1, data: Buffer.from('hello').toString('base64') });
  first.unmount();
  assert.equal(streams[0].cancelled, true);
  assert.equal(calls.filter(call => call.method === 'CloseTerminal').length, 0);

  const second = mount();
  await tick();
  assert.equal(calls.filter(call => call.method === 'OpenTerminal').length, 1);
  assert.deepEqual(streams[1].params, { terminalId: 'shell-1', afterSeq: 1, targetDeviceId: 'host' });
  assert.equal(find(second.render(), 'Close terminal')?.props.onPress instanceof Function, true);
  find(second.render(), 'Close terminal').props.onPress();
  await tick();
  assert.deepEqual(calls.filter(call => call.method === 'CloseTerminal').map(call => call.params), [{ terminalId: 'shell-1', targetDeviceId: 'host' }]);
  second.unmount();

  const third = mount();
  await tick();
  assert.equal(calls.filter(call => call.method === 'OpenTerminal').length, 2);
  third.unmount();

  const subscribe = props.subscribe;
  props.subscribe = async () => { throw new Error('Terminal not found'); };
  const stale = mount();
  await tick();
  stale.unmount();
  props.subscribe = subscribe;
  const recovered = mount();
  await tick();
  assert.equal(calls.filter(call => call.method === 'OpenTerminal').length, 3);
  recovered.unmount();

  props.owner = {};
  const newConnection = mount();
  await tick();
  assert.equal(calls.filter(call => call.method === 'OpenTerminal').length, 4);
  newConnection.unmount();

  let finishOpen;
  props.chatId = 'pending-open-chat';
  props.call = async (method, params) => {
    calls.push({ method, params });
    if (method === 'OpenTerminal') return new Promise(resolve => { finishOpen = resolve; });
    return { ok: true };
  };
  const pending = mount();
  await tick();
  pending.unmount();
  const resumed = mount();
  finishOpen({ id: 'shell-pending', cwd: '/', shell: 'sh' });
  await tick();
  assert.equal(calls.filter(call => call.method === 'OpenTerminal').length, 5);
  assert.deepEqual(streams.at(-1).params, { terminalId: 'shell-pending', afterSeq: 0, targetDeviceId: 'host' });
  assert.equal(calls.filter(call => call.method === 'CloseTerminal').length, 1);
  resumed.unmount();
});
