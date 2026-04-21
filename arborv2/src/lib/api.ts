import { invoke } from '@tauri-apps/api/core';
import type {
  DashboardSnapshot,
  ImpactReport,
  IndexRequest,
  PathReport,
  SearchResponse,
} from './types';

export function bootstrap() {
  return invoke<DashboardSnapshot>('bootstrap');
}

export function indexNotes(request: IndexRequest) {
  return invoke<DashboardSnapshot>('index_notes', {
    notesPath: request.notesPath,
  });
}

export function searchNotes(query: string, limit = 10) {
  return invoke<SearchResponse>('search_notes', { query, limit });
}

export function analyzeNoteImpact(node: string, maxDepth = 5) {
  return invoke<ImpactReport>('analyze_note_impact', { node, maxDepth });
}

export function findNotePath(from: string, to: string) {
  return invoke<PathReport>('find_note_path', { from, to });
}
