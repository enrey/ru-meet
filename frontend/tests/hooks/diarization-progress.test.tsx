import { afterAll, afterEach, beforeEach, expect, mock, test } from 'bun:test';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';

const originalCore = { ...await import('@tauri-apps/api/core') };
const originalEvents = { ...await import('@tauri-apps/api/event') };
const originalI18n = { ...await import('../../src/lib/i18n') };
type Status = { inProgress: boolean; message: string; meetingId: string };
let status: Status;
const listeners = new Map<string, (event: { payload: unknown }) => void>();
const timers = new Map<number, () => void>();
const realSetInterval = globalThis.setInterval;
const realClearInterval = globalThis.clearInterval;
let nextTimer = 0;
const invoke = mock(async (_command: string, _args?: unknown) => status);
mock.module('@tauri-apps/api/core', () => ({ ...originalCore, invoke }));
mock.module('@tauri-apps/api/event', () => ({
  ...originalEvents,
  listen: async (name: string, callback: (event: { payload: unknown }) => void) => {
    listeners.set(name, callback);
    return () => { listeners.delete(name); };
  },
}));
mock.module('../../src/lib/i18n', () => ({ ...originalI18n, useI18n: () => ({ t: (text: string) => text }) }));
const { DiarizationProgress } = await import('../../src/components/MeetingDetails/DiarizationProgress');
let renderer: ReactTestRenderer | undefined;
const refresh = mock(async () => {});
const changed = mock((_active: boolean) => {});

beforeEach(() => {
  status = { inProgress: true, message: 'Identifying speakers…', meetingId: 'meeting-a' };
  listeners.clear(); timers.clear(); invoke.mockClear(); refresh.mockClear(); changed.mockClear();
  globalThis.setInterval = ((callback: () => void) => {
    const id = ++nextTimer; timers.set(id, callback); return id;
  }) as typeof setInterval;
  globalThis.clearInterval = ((id: number) => timers.delete(id)) as typeof clearInterval;
});
afterEach(async () => {
  await act(async () => { renderer?.unmount(); });
  renderer = undefined;
  globalThis.setInterval = realSetInterval;
  globalThis.clearInterval = realClearInterval;
});
afterAll(() => {
  mock.module('@tauri-apps/api/core', () => originalCore);
  mock.module('@tauri-apps/api/event', () => originalEvents);
  mock.module('../../src/lib/i18n', () => originalI18n);
});
async function show() {
  await act(async () => {
    renderer = create(<DiarizationProgress meetingId="meeting-a" onLabelsSaved={refresh} onStatusChange={changed} />);
  });
}
async function emit(name: string, payload: unknown) {
  await act(async () => { listeners.get(name)?.({ payload }); });
}

test('shows a job started before the meeting page opened', async () => {
  await show();
  expect(invoke).toHaveBeenCalledWith('get_diarization_status', { meetingId: 'meeting-a' });
  expect(changed).toHaveBeenLastCalledWith(true);
  expect(JSON.stringify(renderer!.toJSON())).toContain('Identifying speakers…');
});

test('other meetings cannot finish or fail the current indicator', async () => {
  await show();
  await emit('diarization-complete', { meetingId: 'meeting-b' });
  await emit('diarization-rerun-error', { meetingId: 'meeting-b' });
  expect(changed).toHaveBeenLastCalledWith(true);
  expect(refresh).not.toHaveBeenCalled();
});

test('completion refreshes labels once despite duplicate events and polling', async () => {
  await show();
  status = { inProgress: false, message: 'Speaker labels are ready', meetingId: 'meeting-a' };
  await emit('diarization-complete', { meetingId: 'meeting-a' });
  await emit('diarization-labels-saved', { meetingId: 'meeting-a' });
  await act(async () => { for (const callback of timers.values()) callback(); });
  expect(refresh).toHaveBeenCalledTimes(1);
  expect(changed).toHaveBeenLastCalledWith(false);
  expect(renderer!.toJSON()).toBeNull();
});

test('opening after completion recovers missed labels-saved event', async () => {
  status = { inProgress: false, message: 'Speaker labels are ready', meetingId: 'meeting-a' };
  await show();
  expect(refresh).toHaveBeenCalledTimes(1);
  expect(renderer!.toJSON()).toBeNull();
});

test('failure remains visible while the transcript stays available', async () => {
  await show();
  await emit('diarization-rerun-error', { meetingId: 'meeting-a' });
  expect(JSON.stringify(renderer!.toJSON())).toContain('Speaker diarization failed');
  expect(changed).toHaveBeenLastCalledWith(false);
  expect(refresh).not.toHaveBeenCalled();
});
