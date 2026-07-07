//! MCP Apps (SEP-1865) — interactive HTML UI templates.

use serde_json::{json, Value};

/// UI resource URI for blast-radius graph.
pub const UI_BLAST_RADIUS: &str = "ui://arbor/blast-radius";

/// UI resource URI for architecture map.
pub const UI_ARCHITECTURE_MAP: &str = "ui://arbor/architecture-map";

/// List MCP App UI template resources.
pub fn list_app_resources() -> Vec<Value> {
    vec![
        json!({
            "uri": UI_BLAST_RADIUS,
            "name": "Blast Radius Graph",
            "description": "Interactive force-directed graph of impact analysis results",
            "mimeType": "text/html"
        }),
        json!({
            "uri": UI_ARCHITECTURE_MAP,
            "name": "Architecture Map",
            "description": "Interactive overview of codebase hotspots and entry points",
            "mimeType": "text/html"
        }),
    ]
}

/// `_meta.ui` annotation for tools that support MCP Apps.
pub fn ui_meta(uri: &str) -> Value {
    json!({
        "ui": {
            "resourceUri": uri,
            "csp": "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline';"
        }
    })
}

/// Read an MCP App template by URI.
pub fn read_app_template(uri: &str) -> Option<String> {
    match uri {
        UI_BLAST_RADIUS => Some(BLAST_RADIUS_HTML.to_string()),
        UI_ARCHITECTURE_MAP => Some(ARCHITECTURE_MAP_HTML.to_string()),
        _ => None,
    }
}

const BLAST_RADIUS_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Arbor Blast Radius</title>
<style>
  * { margin: 0; padding: 0; box-sizing: border-box; }
  body { background: #0d1117; color: #c9d1d9; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; overflow: hidden; }
  #canvas { display: block; width: 100vw; height: 100vh; }
  #legend { position: fixed; top: 12px; left: 12px; background: rgba(22,27,34,0.9); border: 1px solid #30363d; border-radius: 8px; padding: 12px; font-size: 12px; }
  .dot { display: inline-block; width: 10px; height: 10px; border-radius: 50%; margin-right: 6px; }
  .target { background: #ef4444; }
  .upstream { background: #f59e0b; }
  .downstream { background: #3b82f6; }
  #stats { position: fixed; bottom: 12px; left: 12px; font-size: 11px; color: #8b949e; }
</style>
</head>
<body>
<canvas id="canvas"></canvas>
<div id="legend">
  <div><span class="dot target"></span>Target</div>
  <div><span class="dot upstream"></span>Upstream (callers)</div>
  <div><span class="dot downstream"></span>Downstream (callees)</div>
</div>
<div id="stats"></div>
<script>
(function() {
  const canvas = document.getElementById('canvas');
  const ctx = canvas.getContext('2d');
  const stats = document.getElementById('stats');

  function resize() {
    canvas.width = window.innerWidth;
    canvas.height = window.innerHeight;
  }
  window.addEventListener('resize', resize);
  resize();

  // Receive graph data from MCP host via postMessage
  let data = { target: null, upstream: [], downstream: [] };
  window.addEventListener('message', (e) => {
    if (e.data && e.data.type === 'arbor/blast-radius') {
      data = e.data.payload;
      initGraph();
    }
  });

  // Request data from parent MCP host
  if (window.parent !== window) {
    window.parent.postMessage({ type: 'arbor/request-data', tool: 'analyze_impact' }, '*');
  }

  let nodes = [], edges = [];

  function initGraph() {
    nodes = [];
    edges = [];
    const cx = canvas.width / 2, cy = canvas.height / 2;

    if (data.target) {
      nodes.push({ id: 'target', label: data.target.name || 'target', x: cx, y: cy, color: '#ef4444', r: 14, fixed: true });
    }
    const upCount = (data.upstream || []).length;
    const downCount = (data.downstream || []).length;
    (data.upstream || []).forEach((n, i) => {
      const angle = Math.PI + (i / Math.max(upCount, 1)) * Math.PI;
      const id = 'up-' + i;
      nodes.push({ id, label: n.name, x: cx + Math.cos(angle) * 180, y: cy + Math.sin(angle) * 120, color: '#f59e0b', r: 8 });
      edges.push({ from: id, to: 'target' });
    });
    (data.downstream || []).forEach((n, i) => {
      const angle = (i / Math.max(downCount, 1)) * Math.PI;
      const id = 'down-' + i;
      nodes.push({ id, label: n.name, x: cx + Math.cos(angle) * 180, y: cy - Math.sin(angle) * 120, color: '#3b82f6', r: 8 });
      edges.push({ from: 'target', to: id });
    });

    stats.textContent = `Affected: ${upCount} upstream, ${downCount} downstream`;
    simulate();
  }

  function simulate() {
    for (let iter = 0; iter < 80; iter++) {
      nodes.forEach(n => {
        if (n.fixed) return;
        let fx = 0, fy = 0;
        nodes.forEach(m => {
          if (n === m) return;
          const dx = n.x - m.x, dy = n.y - m.y;
          const dist = Math.max(Math.sqrt(dx*dx + dy*dy), 1);
          const rep = 800 / (dist * dist);
          fx += (dx / dist) * rep;
          fy += (dy / dist) * rep;
        });
        edges.forEach(e => {
          const a = nodes.find(x => x.id === e.from);
          const b = nodes.find(x => x.id === e.to);
          if (!a || !b) return;
          const other = (n.id === a.id) ? b : (n.id === b.id) ? a : null;
          if (!other) return;
          const dx = other.x - n.x, dy = other.y - n.y;
          fx += dx * 0.02;
          fy += dy * 0.02;
        });
        n.x += fx * 0.1;
        n.y += fy * 0.1;
      });
    }
    draw();
  }

  function draw() {
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    edges.forEach(e => {
      const a = nodes.find(x => x.id === e.from);
      const b = nodes.find(x => x.id === e.to);
      if (!a || !b) return;
      ctx.beginPath();
      ctx.moveTo(a.x, a.y);
      ctx.lineTo(b.x, b.y);
      ctx.strokeStyle = '#30363d';
      ctx.lineWidth = 1;
      ctx.stroke();
    });
    nodes.forEach(n => {
      ctx.beginPath();
      ctx.arc(n.x, n.y, n.r, 0, Math.PI * 2);
      ctx.fillStyle = n.color;
      ctx.fill();
      ctx.fillStyle = '#c9d1d9';
      ctx.font = '11px sans-serif';
      ctx.textAlign = 'center';
      ctx.fillText(n.label, n.x, n.y + n.r + 12);
    });
  }

  // Demo data if no host connection
  if (!data.target) {
    data = {
      target: { name: 'parse_file' },
      upstream: [{ name: 'index_directory' }, { name: 'build_graph' }],
      downstream: [{ name: 'resolve_edges' }, { name: 'analyze_impact' }]
    };
    initGraph();
  }
})();
</script>
</body>
</html>"#;

const ARCHITECTURE_MAP_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Arbor Architecture Map</title>
<style>
  * { margin: 0; padding: 0; box-sizing: border-box; }
  body { background: #0d1117; color: #c9d1d9; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; padding: 16px; }
  h1 { font-size: 16px; margin-bottom: 12px; color: #58a6ff; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(200px, 1fr)); gap: 8px; }
  .card { background: #161b22; border: 1px solid #30363d; border-radius: 8px; padding: 12px; cursor: pointer; transition: border-color 0.2s; }
  .card:hover { border-color: #58a6ff; }
  .card .name { font-weight: 600; font-size: 13px; }
  .card .meta { font-size: 11px; color: #8b949e; margin-top: 4px; }
  .hotspot { border-left: 3px solid #f59e0b; }
  .entry { border-left: 3px solid #22c55e; }
  #stats { margin-bottom: 16px; font-size: 12px; color: #8b949e; }
</style>
</head>
<body>
<h1>Architecture Map</h1>
<div id="stats"></div>
<div class="grid" id="grid"></div>
<script>
(function() {
  const grid = document.getElementById('grid');
  const stats = document.getElementById('stats');
  let data = { hotspots: [], entry_points: [], node_count: 0, edge_count: 0 };

  window.addEventListener('message', (e) => {
    if (e.data && e.data.type === 'arbor/architecture-map') {
      data = e.data.payload;
      render();
    }
  });

  if (window.parent !== window) {
    window.parent.postMessage({ type: 'arbor/request-data', tool: 'get_architecture_overview' }, '*');
  }

  function render() {
    stats.textContent = `${data.node_count || 0} nodes, ${data.edge_count || 0} edges`;
    grid.innerHTML = '';
    (data.entry_points || []).slice(0, 8).forEach(ep => {
      const card = document.createElement('div');
      card.className = 'card entry';
      card.innerHTML = `<div class="name">${ep.name}</div><div class="meta">${ep.kind} · ${ep.file}</div>`;
      grid.appendChild(card);
    });
    (data.hotspots || []).slice(0, 12).forEach(h => {
      const card = document.createElement('div');
      card.className = 'card hotspot';
      card.innerHTML = `<div class="name">${h.name}</div><div class="meta">centrality ${(h.centrality||0).toFixed(3)} · ${h.file}</div>`;
      grid.appendChild(card);
    });
  }

  // Demo data
  data = {
    node_count: 1500, edge_count: 4200,
    entry_points: [{ name: 'main', kind: 'Function', file: 'src/main.rs' }],
    hotspots: [{ name: 'ArborGraph', kind: 'Struct', file: 'graph.rs', centrality: 0.92 }]
  };
  render();
})();
</script>
</body>
</html>"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blast_radius_template_exists() {
        let html = read_app_template(UI_BLAST_RADIUS).unwrap();
        assert!(html.contains("arbor/blast-radius"));
        assert!(html.contains("canvas"));
    }

    #[test]
    fn architecture_map_template_exists() {
        let html = read_app_template(UI_ARCHITECTURE_MAP).unwrap();
        assert!(html.contains("arbor/architecture-map"));
    }
}
