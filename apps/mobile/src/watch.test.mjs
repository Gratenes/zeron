import assert from 'node:assert/strict';
import { test } from 'node:test';
import { watchWithRetry } from './watch.ts';
import { applyTranscriptUpdate } from './liveModel.ts';

test('restarts a failed foreground stream and cancels it on cleanup', async () => {
  const starts = [];
  const failures = [];
  const subscribe = async (_method, _params, onItem, onError) => {
    const stream = { onItem, onError, canceled: false };
    starts.push(stream);
    return () => { stream.canceled = true; };
  };
  const received = [];
  const stop = watchWithRetry(subscribe, 'WatchChats', {}, item => received.push(item), error => failures.push(error.message), 1);
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(starts.length, 1);
  starts[0].onItem(['first']);
  starts[0].onError(new Error('link lost'));
  await new Promise(resolve => setTimeout(resolve, 10));
  assert.equal(starts[0].canceled, true);
  assert.equal(starts.length, 2);
  starts[1].onItem(['recovered']);
  assert.deepEqual(received, [['first'], ['recovered']]);
  assert.deepEqual(failures, ['link lost']);
  stop();
  assert.equal(starts[1].canceled, true);
});

test('invalid transcript delta requests a fresh reset without throwing into React', async () => {
  const starts = [];
  const errors = [];
  let entries = [];
  const stop = watchWithRetry(async (_method, _params, onItem, onError) => {
    const stream = { onItem, onError, canceled: false };
    starts.push(stream);
    return () => { stream.canceled = true; };
  }, 'WatchDocMessages', { chatId: 'chat' }, item => {
    entries = applyTranscriptUpdate(entries, item);
  }, error => errors.push(error.message), 1);
  await new Promise(resolve => setTimeout(resolve, 0));
  starts[0].onItem({ reset: [{ id: 'one', role: 'assistant', parts: [] }] });
  starts[0].onItem({ append: [{ entry: 'missing', part: 'missing', text: 'x', len: 1 }] });
  await new Promise(resolve => setTimeout(resolve, 10));
  assert.equal(starts[0].canceled, true);
  assert.equal(starts.length, 2);
  starts[1].onItem({ reset: [{ id: 'two', role: 'assistant', parts: [] }] });
  assert.deepEqual(entries.map(entry => entry.id), ['two']);
  assert.deepEqual(errors, ['Transcript append target missing']);
  stop();
});
