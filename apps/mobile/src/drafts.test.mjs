import assert from 'node:assert/strict';
import { test } from 'node:test';
import { clearDeliveredDraft, moveNewDraft } from './drafts.ts';

test('a delayed send preserves text typed afterward in an existing chat', () => {
  const submitted = 'first message';
  const whilePending = { chat: 'next message' };
  assert.equal(clearDeliveredDraft(whilePending, 'chat', 'chat', submitted).chat, 'next message');
  assert.equal(clearDeliveredDraft({ chat: submitted }, 'chat', 'chat', submitted).chat, '');
});

test('a delayed first send migrates and preserves the next chat draft', () => {
  const submitted = 'first message';
  const migrated = moveNewDraft({ __new__: submitted }, 'new-chat');
  assert.deepEqual(migrated, { __new__: '', 'new-chat': submitted });
  assert.deepEqual(clearDeliveredDraft({ ...migrated, 'new-chat': 'next message' }, '__new__', 'new-chat', submitted),
    { __new__: '', 'new-chat': 'next message' });
});
