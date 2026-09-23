const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { test } = require('node:test');
const ts = require('typescript');

test('file drafts survive section and session navigation and keep their original conflict guard', async () => {
  let active;
  const same = (a, b) => a?.length === b?.length && a.every((value, i) => Object.is(value, b[i]));
  const hooks = {
    useState(initial) {
      const runner = active; const index = runner.next++;
      if (!runner.slots[index]) runner.slots[index] = { value: initial };
      return [runner.slots[index].value, value => {
        runner.slots[index].value = typeof value === 'function' ? value(runner.slots[index].value) : value;
        runner.dirty = true;
      }];
    },
    useRef(initial) {
      const runner = active; const index = runner.next++;
      return (runner.slots[index] ??= { current: initial });
    },
    useCallback(fn, deps) {
      const runner = active; const index = runner.next++;
      const slot = runner.slots[index];
      if (slot && same(slot.deps, deps)) return slot.value;
      runner.slots[index] = { value: fn, deps };
      return fn;
    },
    useEffect(fn, deps) {
      const runner = active; const index = runner.next++;
      const slot = runner.slots[index];
      if (!slot || !same(slot.deps, deps)) runner.pending.push({ index, fn, deps });
    },
  };
  const jsx = (type, props) => ({ type, props });
  const source = fs.readFileSync(path.join(__dirname, 'Workspace.tsx'), 'utf8');
  const code = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX, target: ts.ScriptTarget.ES2022 } }).outputText;
  const exports = {};
  vm.runInNewContext(code, {
    exports, Map, require: name => name === 'react' ? hooks : name === 'react/jsx-runtime' ? { jsx, jsxs: jsx }
      : name === 'react-native' ? { ActivityIndicator: 'ActivityIndicator', FlatList: 'FlatList', Pressable: 'Pressable', Text: 'Text', TextInput: 'TextInput', View: 'View', StyleSheet: { create: value => value } }
      : { colors: {}, radius: {}, spacing: {}, typography: {} },
  });

  const calls = [];
  let completeRaceWrite;
  let raceWrites = 0;
  const entry = { path: 'note.txt', name: 'note.txt', kind: 'file', ignored: false, readOnly: false };
  const call = async (method, params) => {
    calls.push({ method, params });
    if (method === 'ListWorkspaceDirectory') return { directory: params.directory, entries: [entry] };
    if (method === 'ReadWorkspaceFile') return { checkoutId: `checkout-${params.chatId}`, path: entry.path, text: 'original', contentHash: `hash-${params.chatId}`, encoding: 'utf8', lineEnding: params.chatId === 'race' ? 'none' : 'lf', readOnlyReason: null, truncated: false };
    if (method === 'WriteWorkspaceFile' && params.chatId === 'race') {
      return ++raceWrites === 1
        ? new Promise(resolve => { completeRaceWrite = resolve; })
        : { status: 'written', file: { contentHash: 'hash-after-B' } };
    }
    if (method === 'WriteWorkspaceFile') return { status: 'conflict' };
    throw new Error(method);
  };
  const mount = chatId => {
    let currentChatId = chatId;
    const runner = { slots: [], pending: [], next: 0, dirty: true, tree: null };
    const render = () => {
      for (let i = 0; runner.dirty && i < 10; i++) {
        runner.dirty = false; runner.next = 0; runner.pending = [];
        active = runner;
        runner.tree = exports.Workspace({ call, chatId: currentChatId, targetDeviceId: 'device', cwd: '/repo' });
        for (const { index, fn, deps } of runner.pending) {
          runner.slots[index]?.cleanup?.();
          runner.slots[index] = { deps, cleanup: fn() };
        }
      }
    };
    return {
      get tree() { render(); return runner.tree; },
      async settle() { for (let i = 0; i < 6; i++) { render(); await Promise.resolve(); } render(); },
      switchChat(id) { currentChatId = id; runner.dirty = true; },
      unmount() { runner.slots.forEach(slot => slot?.cleanup?.()); },
    };
  };
  const find = (node, predicate) => {
    if (!node || typeof node !== 'object') return null;
    if (predicate(node)) return node;
    const children = node.props?.children;
    for (const child of Array.isArray(children) ? children : [children]) {
      const match = find(child, predicate);
      if (match) return match;
    }
    return null;
  };
  const editor = view => find(view.tree, node => node.type === 'TextInput');
  const open = async view => {
    await view.settle();
    find(view.tree, node => node.type === 'FlatList').props.renderItem({ item: entry }).props.onPress();
    await view.settle();
  };

  const first = mount('one');
  await first.settle();
  const staleOpen = find(first.tree, node => node.type === 'FlatList').props.renderItem({
    item: { ...entry, path: 'other.txt', name: 'other.txt' },
  }).props.onPress;
  await open(first);
  editor(first).props.onChangeText('draft one');
  staleOpen();
  await first.settle();
  assert.equal(editor(first).props.value, 'draft one');
  first.unmount(); // Files → Terminal

  const second = mount('one'); // Terminal → Files
  await second.settle();
  assert.equal(editor(second).props.value, 'draft one');
  second.unmount();

  const other = mount('two');
  await open(other);
  editor(other).props.onChangeText('draft two');
  other.unmount();

  const back = mount('one');
  await back.settle();
  assert.equal(editor(back).props.value, 'draft one');
  find(back.tree, node => node.type === 'Pressable' && node.props.children?.props?.children === 'Save').props.onPress();
  await back.settle();
  assert.equal(editor(back).props.value, 'draft one');
  assert.deepEqual(JSON.parse(JSON.stringify(calls.findLast(item => item.method === 'WriteWorkspaceFile').params)), {
    chatId: 'one', targetDeviceId: 'device', expectedCheckoutId: 'checkout-one', path: 'note.txt',
    text: 'draft one', expectedContentHash: 'hash-one', encoding: 'utf8', lineEnding: 'lf',
  });
  back.unmount();

  const again = mount('one');
  await again.settle();
  assert.equal(editor(again).props.value, 'draft one');
  again.unmount();
  const otherAgain = mount('two');
  await otherAgain.settle();
  assert.equal(editor(otherAgain).props.value, 'draft two');
  otherAgain.unmount();

  for (let i = 3; i <= 12; i++) {
    const view = mount(String(i));
    await open(view);
    editor(view).props.onChangeText(`draft ${i}`);
    view.unmount();
  }
  const full = mount('13');
  await open(full);
  editor(full).props.onChangeText('would be lost on navigation');
  assert.equal(editor(full).props.value, 'original');
  assert.match(find(full.tree, node => node.props?.accessibilityLiveRegion === 'polite').props.children, /memory is full/);
  full.unmount();
  const retained = mount('one');
  await retained.settle();
  assert.equal(editor(retained).props.value, 'draft one');
  find(retained.tree, node => node.type === 'Pressable' && node.props.children?.props?.children === 'Discard changes').props.onPress();
  assert.equal(editor(retained).props.value, 'original');
  retained.unmount();
  const freed = mount('13');
  await open(freed);
  editor(freed).props.onChangeText('now retained');
  freed.unmount();
  const resumed = mount('13');
  await resumed.settle();
  assert.equal(editor(resumed).props.value, 'now retained');
  find(resumed.tree, node => node.type === 'Pressable' && node.props.children?.props?.children === 'Discard changes').props.onPress();
  resumed.unmount();
  const remove = mount('two');
  await remove.settle();
  find(remove.tree, node => node.type === 'Pressable' && node.props.children?.props?.children === 'Discard changes').props.onPress();
  remove.unmount();

  const live = mount('one');
  await open(live);
  editor(live).props.onChangeText('draft one again');
  live.switchChat('fresh');
  await live.settle();
  assert.equal(editor(live), null);
  await open(live);
  editor(live).props.onChangeText('fresh draft');
  live.switchChat('one');
  await live.settle();
  assert.equal(editor(live).props.value, 'draft one again');
  live.switchChat('fresh');
  await live.settle();
  assert.equal(editor(live).props.value, 'fresh draft');
  find(live.tree, node => node.type === 'Pressable' && node.props.children?.props?.children === 'Discard changes').props.onPress();
  live.unmount();

  const saving = mount('race');
  await open(saving);
  editor(saving).props.onChangeText('A');
  find(saving.tree, node => node.type === 'Pressable' && node.props.children?.props?.children === 'Save').props.onPress();
  await saving.settle();
  saving.unmount();

  const reentered = mount('race');
  await reentered.settle();
  assert.equal(editor(reentered).props.value, 'A');
  assert.equal(editor(reentered).props.editable, false);
  editor(reentered).props.onChangeText('B before response');
  assert.equal(editor(reentered).props.value, 'A');
  completeRaceWrite({ status: 'written', file: { contentHash: 'hash-after-A' } });
  await reentered.settle();
  assert.equal(editor(reentered).props.editable, true);
  editor(reentered).props.onChangeText('B');
  find(reentered.tree, node => node.type === 'Pressable' && node.props.children?.props?.children === 'Save').props.onPress();
  await reentered.settle();
  assert.equal(calls.findLast(item => item.method === 'WriteWorkspaceFile').params.expectedContentHash, 'hash-after-A');
  assert.equal(calls.findLast(item => item.method === 'WriteWorkspaceFile').params.lineEnding, 'lf');
  reentered.unmount();
});
