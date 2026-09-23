export type Drafts = Record<string, string>;

export function moveNewDraft(drafts: Drafts, chatId: string): Drafts {
  return { ...drafts, __new__: '', [chatId]: drafts.__new__ ?? '' };
}

export function clearDeliveredDraft(drafts: Drafts, draftKey: string, chatId: string, submitted: string): Drafts {
  const next = { ...drafts };
  if ((next[draftKey] ?? '') === submitted) next[draftKey] = '';
  if ((next[chatId] ?? '') === submitted) next[chatId] = '';
  return next;
}
