<script lang="ts">
  import { fade, fly, scale } from 'svelte/transition';
  import { onMount } from 'svelte';
  import {
    analyzeNoteImpact,
    bootstrap,
    findNotePath,
    indexNotes,
    searchNotes,
  } from './lib/api';
  import type {
    DashboardSnapshot,
    ImpactReport,
    IndexRequest,
    PathReport,
    SearchResponse,
    NodeSummary,
  } from './lib/types';

  let dashboard: DashboardSnapshot | null = null;
  let searchResponse: SearchResponse | null = null;
  let impactReport: ImpactReport | null = null;
  let pathReport: PathReport | null = null;
  let errorMessage = '';
  let statusLine = 'Booting deterministic context...';

  let searchQuery = '';
  let impactTarget = '';
  let pathFrom = '';
  let pathTo = '';
  let notesPathInput = '';
  let loading = false;
  let indexing = false;

  onMount(async () => {
    await refresh();
  });

  async function refresh(indexRequest?: IndexRequest) {
    loading = true;
    errorMessage = '';

    try {
      dashboard = indexRequest ? await indexNotes(indexRequest) : await bootstrap();
      notesPathInput = dashboard.notesPath;
      searchResponse = null;
      impactReport = null;
      pathReport = null;
      statusLine = dashboard.lastIndexSummary
        ? `Indexed ${dashboard.lastIndexSummary.filesIndexed} files and wrote ${dashboard.lastIndexSummary.nodesWritten} sections.`
        : `Graph ready with ${dashboard.stats.nodes} nodes and ${dashboard.stats.edges} edges.`;
    } catch (error) {
      errorMessage = normalizeError(error);
      statusLine = 'Waiting for a healthy graph snapshot.';
    } finally {
      loading = false;
      indexing = false;
    }
  }

  async function handleIndex() {
    indexing = true;
    await refresh({
      notesPath: notesPathInput.trim() || undefined,
    });
  }

  async function handleSearch() {
    if (!searchQuery.trim()) {
      searchResponse = null;
      return;
    }

    loading = true;
    errorMessage = '';

    try {
      searchResponse = await searchNotes(searchQuery, 10);
      statusLine = `Found ${searchResponse.results.length} structurally ranked matches for "${searchQuery}".`;
    } catch (error) {
      errorMessage = normalizeError(error);
    } finally {
      loading = false;
    }
  }

  async function handleImpact() {
    if (!impactTarget.trim()) return;

    loading = true;
    errorMessage = '';

    try {
      impactReport = await analyzeNoteImpact(impactTarget, 6);
      statusLine = impactReport.summary;
    } catch (error) {
      errorMessage = normalizeError(error);
    } finally {
      loading = false;
    }
  }

  async function handlePath() {
    if (!pathFrom.trim() || !pathTo.trim()) return;

    loading = true;
    errorMessage = '';

    try {
      pathReport = await findNotePath(pathFrom, pathTo);
      statusLine = `Resolved a ${pathReport.nodes.length}-node path from ${pathReport.from.name} to ${pathReport.to.name}.`;
    } catch (error) {
      errorMessage = normalizeError(error);
    } finally {
      loading = false;
    }
  }

  function adoptNode(node: NodeSummary) {
    impactTarget = node.name;
    if (!pathFrom) pathFrom = node.name;
    else pathTo = node.name;
  }

  function normalizeError(error: unknown) {
    return error instanceof Error ? error.message : 'Unknown runtime failure.';
  }
</script>

<svelte:head>
  <title>Arbor V2</title>
  <meta
    name="description"
    content="Arbor V2 is the Lattice desktop for deterministic graph-native context."
  />
</svelte:head>

<div class="app-shell">
  <div class="ambient ambient-a"></div>
  <div class="ambient ambient-b"></div>
  <div class="ambient ambient-c"></div>

  <main class="dashboard">
    <section class="hero glass" in:fly={{ y: 24, duration: 500 }}>
      <div class="hero-copy">
        <p class="eyebrow">Arbor & Lattice</p>
        <h1>Graph-native memory for serious agent work.</h1>
        <p class="lede">
          Deterministic note indexing, structural search, blast-radius analysis,
          and causal path tracing inside a desktop shell built for high-trust AI
          workflows.
        </p>

        <div class="hero-actions">
          <button class="primary" on:click={handleIndex} disabled={indexing || loading}>
            {#if indexing}Re-indexing...{:else}Refresh Graph{/if}
          </button>
          <button class="ghost" on:click={() => refresh()} disabled={loading}>
            Rehydrate Snapshot
          </button>
        </div>
      </div>

      <div class="hero-status">
        <div class="status-pill">{statusLine}</div>
        {#if errorMessage}
          <div class="error-card" transition:scale={{ duration: 180 }}>
            {errorMessage}
          </div>
        {/if}

        <div class="metric-grid">
          <article class="metric glass-subtle">
            <span>Nodes</span>
            <strong>{dashboard?.stats.nodes ?? '—'}</strong>
          </article>
          <article class="metric glass-subtle">
            <span>Edges</span>
            <strong>{dashboard?.stats.edges ?? '—'}</strong>
          </article>
          <article class="metric glass-subtle">
            <span>Files</span>
            <strong>{dashboard?.stats.files ?? '—'}</strong>
          </article>
        </div>
      </div>
    </section>

    <section class="workspace-grid">
      <article class="glass explorer" in:fly={{ y: 36, duration: 580, delay: 50 }}>
        <header>
          <p class="eyebrow">Workspace</p>
          <h2>Index Control</h2>
        </header>

        <label class="stack">
          <span>Notes path</span>
          <input bind:value={notesPathInput} placeholder="arborv2/notes" />
        </label>

        <div class="meta-list">
          <div>
            <span>DB</span>
            <strong>{dashboard?.dbPath ?? 'Pending bootstrap'}</strong>
          </div>
          <div>
            <span>Sample prompts</span>
            <strong>{dashboard?.sampleQueries.join(' · ') ?? 'Loading'}</strong>
          </div>
        </div>

        {#if dashboard?.lastIndexSummary}
          <div class="summary-band" transition:fade>
            <span>{dashboard.lastIndexSummary.filesIndexed} indexed</span>
            <span>{dashboard.lastIndexSummary.filesSkipped} cached</span>
            <span>{dashboard.lastIndexSummary.filesRemoved} removed</span>
            <span>{dashboard.lastIndexSummary.nodesWritten} sections</span>
          </div>
        {/if}
      </article>

      <article class="glass explorer" in:fly={{ y: 36, duration: 620, delay: 100 }}>
        <header>
          <p class="eyebrow">Structural Search</p>
          <h2>Interrogate the graph</h2>
        </header>

        <div class="input-row">
          <input
            bind:value={searchQuery}
            placeholder="Search section titles or IDs"
            on:keydown={(event) => event.key === 'Enter' && handleSearch()}
          />
          <button class="primary compact" on:click={handleSearch} disabled={loading}>
            Search
          </button>
        </div>

        <div class="results-list">
          {#if searchResponse?.results.length}
            {#each searchResponse.results as node, index (node.id)}
              <button
                class="result-card"
                transition:fly={{ y: 12, duration: 220, delay: index * 30 }}
                on:click={() => adoptNode(node)}
              >
                <div>
                  <strong>{node.name}</strong>
                  <span>{node.file}:{node.lineStart}</span>
                </div>
                <b>{node.score.toFixed(2)}</b>
              </button>
            {/each}
          {:else}
            <div class="empty-state">
              Search results land here with exact titles, file provenance, and
              centrality ranking.
            </div>
          {/if}
        </div>
      </article>
    </section>

    <section class="analysis-grid">
      <article class="glass panel" in:fly={{ y: 40, duration: 640, delay: 140 }}>
        <header>
          <p class="eyebrow">Impact</p>
          <h2>Blast radius explorer</h2>
        </header>
        <div class="input-row">
          <input bind:value={impactTarget} placeholder="Core Habits" />
          <button class="primary compact" on:click={handleImpact} disabled={loading}>
            Analyze
          </button>
        </div>

        {#if impactReport}
          <div class="impact-report" transition:fade>
            <div class="target-headline">
              <strong>{impactReport.target.name}</strong>
              <span>{impactReport.totalAffected} affected nodes</span>
            </div>
            <p>{impactReport.summary}</p>

            <div class="dual-list">
              <div>
                <h3>Upstream</h3>
                {#if impactReport.upstream.length}
                  {#each impactReport.upstream as node}
                    <div class="mini-node">
                      <span>{node.name}</span>
                      <b>{node.severity}</b>
                    </div>
                  {/each}
                {:else}
                  <p class="subtle">No upstream dependents.</p>
                {/if}
              </div>

              <div>
                <h3>Downstream</h3>
                {#if impactReport.downstream.length}
                  {#each impactReport.downstream as node}
                    <div class="mini-node">
                      <span>{node.name}</span>
                      <b>{node.severity}</b>
                    </div>
                  {/each}
                {:else}
                  <p class="subtle">No downstream dependencies.</p>
                {/if}
              </div>
            </div>
          </div>
        {:else}
          <div class="empty-state">
            Select a top node or type one manually to see deterministic upstream
            and downstream fallout.
          </div>
        {/if}
      </article>

      <article class="glass panel" in:fly={{ y: 40, duration: 680, delay: 180 }}>
        <header>
          <p class="eyebrow">Pathfinding</p>
          <h2>Knowledge route planner</h2>
        </header>
        <div class="path-grid">
          <input bind:value={pathFrom} placeholder="From section" />
          <input bind:value={pathTo} placeholder="To section" />
        </div>
        <button class="primary compact wide" on:click={handlePath} disabled={loading}>
          Resolve Path
        </button>

        {#if pathReport}
          <ol class="path-list" transition:fade>
            {#each pathReport.nodes as node, index (node.id)}
              <li style={`--delay:${index * 60}ms`}>
                <strong>{node.name}</strong>
                <span>{node.file}</span>
              </li>
            {/each}
          </ol>
        {:else}
          <div class="empty-state">
            Shortest-path traces reveal how ideas, habits, and strategy nodes
            connect through explicit graph edges.
          </div>
        {/if}
      </article>
    </section>

    <section class="glass topography" in:fly={{ y: 44, duration: 720, delay: 220 }}>
      <header>
        <div>
          <p class="eyebrow">Centrality Map</p>
          <h2>Highest leverage knowledge nodes</h2>
        </div>
        <span class="legend">Tap a node to seed the analysis panels.</span>
      </header>

      <div class="node-cloud">
        {#each dashboard?.topNodes ?? [] as node, index (node.id)}
          <button
            class="cloud-node"
            on:click={() => adoptNode(node)}
            style={`--weight:${0.84 + node.score * 0.42}; --delay:${index * 55}ms;`}
          >
            <span>{node.name}</span>
            <b>{node.score.toFixed(2)}</b>
          </button>
        {/each}
      </div>
    </section>
  </main>
</div>
