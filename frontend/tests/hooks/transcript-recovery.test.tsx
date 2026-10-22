import { afterAll, afterEach, beforeEach, expect, mock, test } from 'bun:test';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import type { MeetingMetadata } from '../../src/services/indexedDBService';

(globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;

const originalCore = { ...await import('@tauri-apps/api/core') };
const originalIndexed = { ...await import('../../src/services/indexedDBService') };
const originalStorage = { ...await import('../../src/services/storageService') };
const originalLanguage = { ...await import('../../src/lib/summary-language-preferences') };
let draft: MeetingMetadata;
let matches: { draftId: string; meetingId: string }[];
const markSaved = mock(async () => { draft.savedToSQLite = true; });
const transcripts = mock(async () => [{ id: 1, meetingId: draft.meetingId, text: 'Recovered speech', timestamp: '00:01', sequenceId: 1 }]);
const save = mock(async (_title: string, _transcripts: unknown[], _folder: string | null) => ({ meeting_id: 'new-native-meeting' }));
const invoke = mock(async (command: string): Promise<unknown> => {
  if (command === 'api_match_saved_recordings') return matches;
  if (command === 'has_audio_checkpoints') return true;
  if (command === 'recover_audio_from_checkpoints') return { status: 'success', chunk_count: 1 };
  if (command === 'cleanup_checkpoints') return;
  throw new Error(`Unexpected command: ${command}`);
});
mock.module('@tauri-apps/api/core', () => ({ ...originalCore, invoke }));
mock.module('../../src/services/indexedDBService', () => ({ indexedDBService: {
  getAllMeetings: async () => draft.savedToSQLite ? [] : [draft],
  getMeetingMetadata: async () => draft,
  markMeetingSaved: markSaved,
  getTranscripts: transcripts,
} }));
mock.module('../../src/services/storageService', () => ({ ...originalStorage, storageService: { saveMeeting: save } }));
mock.module('../../src/lib/summary-language-preferences', () => ({ ...originalLanguage, applyPinnedSummaryLanguageToMeeting: async () => true }));
const { useTranscriptRecovery } = await import('../../src/hooks/useTranscriptRecovery');
let state: ReturnType<typeof useTranscriptRecovery>;
let renderer: ReactTestRenderer | undefined;
function View() { state = useTranscriptRecovery(); return null; }

beforeEach(async () => {
  draft = { meetingId: 'browser-draft', title: 'Auto Meeting', folderPath: 'recording-folder', savedToSQLite: false,
    transcriptCount: 11, startTime: Date.now() - 60000, lastUpdated: Date.now() - 30000 };
  matches = [];
  invoke.mockClear(); markSaved.mockClear(); transcripts.mockClear(); save.mockClear();
  await act(async () => { renderer = create(<View />); });
});
afterEach(async () => { await act(async () => renderer?.unmount()); });
afterAll(() => {
  mock.module('@tauri-apps/api/core', () => originalCore);
  mock.module('../../src/services/indexedDBService', () => originalIndexed);
  mock.module('../../src/services/storageService', () => originalStorage);
  mock.module('../../src/lib/summary-language-preferences', () => originalLanguage);
});

test('startup reconciles a saved native recording before inspecting audio', async () => {
  matches = [{ draftId: draft.meetingId, meetingId: 'existing-native-id' }];
  await act(async () => { await state.checkForRecoverableTranscripts(); });
  expect(state.recoverableMeetings).toEqual([]);
  expect(markSaved).toHaveBeenCalledWith('browser-draft');
  expect(draft.savedToSQLite).toBe(true);
  expect(invoke.mock.calls.map(call => call[0])).toEqual(['api_match_saved_recordings']);
  expect(save).not.toHaveBeenCalled();
});

test('a genuinely interrupted recording remains recoverable', async () => {
  await act(async () => { await state.checkForRecoverableTranscripts(); });
  expect(state.recoverableMeetings).toHaveLength(1);
  expect(markSaved).not.toHaveBeenCalled();
  expect(invoke.mock.calls.map(call => call[0])).toEqual(['api_match_saved_recordings', 'has_audio_checkpoints']);
});

test('rechecking on recovery reuses an existing meeting without rewriting audio or transcripts', async () => {
  matches = [{ draftId: draft.meetingId, meetingId: 'existing-native-id' }];
  let result: Awaited<ReturnType<typeof state.recoverMeeting>> = { success: false };
  await act(async () => { result = await state.recoverMeeting(draft.meetingId); });
  expect(result).toEqual({ success: true, meetingId: 'existing-native-id' });
  expect(transcripts).not.toHaveBeenCalled();
  expect(save).not.toHaveBeenCalled();
  expect(invoke.mock.calls.map(call => call[0])).toEqual(['api_match_saved_recordings']);
});

test('unsaved transcripts and their own audio are still recovered', async () => {
  let result: Awaited<ReturnType<typeof state.recoverMeeting>> = { success: false };
  await act(async () => { result = await state.recoverMeeting(draft.meetingId); });
  expect(result).toMatchObject({ success: true, meetingId: 'new-native-meeting' });
  expect(save).toHaveBeenCalledTimes(1);
  expect(save.mock.calls[0][2]).toBe('recording-folder');
  expect(markSaved).toHaveBeenCalled();
  expect(invoke.mock.calls.map(call => call[0])).toEqual(['api_match_saved_recordings', 'recover_audio_from_checkpoints', 'cleanup_checkpoints']);
});

test('a draft without its own folder never borrows the active recording folder', async () => {
  draft.folderPath = undefined;
  await act(async () => { await state.recoverMeeting(draft.meetingId); });
  expect(save.mock.calls[0][2]).toBeNull();
  expect(invoke.mock.calls.map(call => call[0])).toEqual(['api_match_saved_recordings']);
});
