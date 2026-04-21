export interface GraphStatsView {
  nodes: number;
  edges: number;
  files: number;
}

export interface NodeSummary {
  id: string;
  name: string;
  kind: string;
  file: string;
  lineStart: number;
  lineEnd: number;
  score: number;
  signature: string | null;
}

export interface IndexSummary {
  notesPath: string;
  dbPath: string;
  filesIndexed: number;
  filesSkipped: number;
  filesRemoved: number;
  nodesWritten: number;
}

export interface DashboardSnapshot {
  notesPath: string;
  dbPath: string;
  stats: GraphStatsView;
  topNodes: NodeSummary[];
  sampleQueries: string[];
  lastIndexSummary: IndexSummary | null;
}

export interface SearchResponse {
  query: string;
  results: NodeSummary[];
}

export interface ImpactNode extends NodeSummary {
  severity: string;
  hopDistance: number;
  direction: string;
}

export interface ImpactReport {
  target: NodeSummary;
  summary: string;
  upstream: ImpactNode[];
  downstream: ImpactNode[];
  totalAffected: number;
  maxDepth: number;
  queryTimeMs: number;
}

export interface PathReport {
  from: NodeSummary;
  to: NodeSummary;
  nodes: NodeSummary[];
}

export interface IndexRequest {
  notesPath?: string;
}
