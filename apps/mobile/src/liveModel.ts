import type { SessionPreview, SpacePreview } from './Shell';
import type { TranscriptItem, TranscriptPart } from './Transcript';

export type Chat = {
  id: string;
  deviceId: string;
  title: string | null;
  archived: boolean;
  cwd: string | null;
  branch: string | null;
  spaceId: string | null;
  config: { harness: string; model: string | null; reasoning: string | null; sandbox: string; modelOptions: Record<string, unknown> } | null;
  lastMessagePreview: string | null;
  lastMessageAt: string | null;
  lastSeenAt: string | null;
  createdAt: string;
};

export type Space = { id: string; name: string | null; deviceId: string; path: string };
export type LiveSession = { chatId: string; status: 'idle' | 'working' | 'awaitingInput' | 'errored'; updatedAt: string };

type WirePart = {
  id: string;
  kind: string;
  text?: string;
  call?: Record<string, unknown>;
  output?: string;
  isError?: boolean;
  resolved?: boolean;
  uri?: string;
  title?: string;
};

export type WireEntry = {
  id: string;
  role: 'user' | 'assistant' | 'system';
  parts: WirePart[];
  status?: 'streaming' | 'complete' | 'aborted';
};

export type TranscriptUpdate = {
  reset?: WireEntry[];
  upsert?: { after: string | null; entry: WireEntry }[];
  append?: { entry: string; part: string; text: string; len: number }[];
  remove?: string[];
  count?: number;
};

export function applyTranscriptUpdate(current: WireEntry[], update: TranscriptUpdate): WireEntry[] {
  if (update.reset) return update.reset;
  const next = current.filter(entry => !update.remove?.includes(entry.id)).map(entry => ({ ...entry, parts: entry.parts.map(part => ({ ...part })) }));
  for (const change of update.upsert ?? []) {
    const previous = next.findIndex(entry => entry.id === change.entry.id);
    if (previous >= 0) next.splice(previous, 1);
    const after = change.after === null ? -1 : next.findIndex(entry => entry.id === change.after);
    if (change.after !== null && after < 0) throw new Error('Transcript anchor missing');
    next.splice(after + 1, 0, change.entry);
  }
  for (const change of update.append ?? []) {
    const entry = next.find(item => item.id === change.entry);
    const part = entry?.parts.find(item => item.id === change.part);
    if (!part || (part.kind !== 'text' && part.kind !== 'reasoning')) throw new Error('Transcript append target missing');
    const body = (part.text ?? '') + change.text;
    if (utf8Length(body) !== change.len) throw new Error('Transcript append length mismatch');
    part.text = body;
  }
  if (update.count !== undefined && next.length !== update.count) throw new Error('Transcript count mismatch');
  return [...next];
}

export function renderEntries(entries: WireEntry[]): TranscriptItem[] {
  return entries.map(entry => ({
    id: entry.id,
    role: entry.role,
    status: entry.status === 'aborted' ? 'error' : entry.status,
    parts: entry.parts.flatMap((part): TranscriptPart[] => {
      if (part.kind === 'text') return [{ type: 'text', text: part.text ?? '' }];
      if (part.kind === 'reasoning') return [{ type: 'reasoning', text: part.text ?? '' }];
      if (part.kind === 'tool') {
        const call = part.call ?? {};
        return [{ type: 'tool', id: part.id, name: String(call.kind ?? call.tag ?? 'Tool'),
          input: typeof call.command === 'string' ? call.command : typeof call.path === 'string' ? call.path : undefined,
          output: part.output, status: part.isError ? 'error' : part.resolved ? 'complete' : 'running' }];
      }
      if (part.kind === 'error') return [{ type: 'text', text: part.text ?? '' }];
      if (part.kind === 'artifact') return [{ type: 'artifact', id: part.id, title: part.title ?? 'Artifact', uri: part.uri }];
      return [];
    }),
  }));
}

export function previews(chats: Chat[], spaces: Space[], sessions: LiveSession[]): SessionPreview[] {
  const spaceById = new Map(spaces.map(space => [space.id, space]));
  const sessionByChat = new Map(sessions.map(session => [session.chatId, session]));
  return [...chats].sort((a, b) => (b.lastMessageAt ?? b.createdAt).localeCompare(a.lastMessageAt ?? a.createdAt)).map(chat => {
    const space = chat.spaceId ? spaceById.get(chat.spaceId) : undefined;
    const live = sessionByChat.get(chat.id);
    const completed = chat.lastMessageAt && (!chat.lastSeenAt || chat.lastSeenAt < chat.lastMessageAt);
    return { id: chat.id, title: chat.title || chat.lastMessagePreview || 'New conversation',
      spaceId: chat.spaceId ?? undefined,
      spaceName: space ? space.name || space.path.split('/').pop() || space.path : undefined,
      branch: chat.branch ?? undefined,
      harness: chat.config?.harness, archived: chat.archived,
      status: live?.status === 'idle' && completed ? 'completed' : live?.status ?? 'idle' };
  });
}

export function spacePreviews(spaces: Space[]): SpacePreview[] {
  return spaces.map(space => ({ id: space.id, name: space.name || space.path.split('/').pop() || 'Space' }));
}

function utf8Length(value: string): number {
  return encodeURIComponent(value).replace(/%[0-9a-f]{2}/gi, 'x').length;
}
