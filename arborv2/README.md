# Arbor V2

Arbor V2 is the desktop Lattice surface for graph-native work in the Arbor ecosystem.
It combines:

- **Tauri v2** for the desktop runtime shell
- **Rust** for deterministic indexing, graph operations, and command execution
- **Svelte + Vite** for the interactive operator UI

The project is designed to keep local context explicit, queryable, and auditable.

## Current capabilities

- **Local-first note indexing** into an Arbor graph store
- **Search** over indexed note sections
- **Impact analysis** (upstream/downstream blast radius)
- **Path discovery** between two graph nodes
- **Dashboard view** with graph stats and top-ranked nodes
- **Seed note bootstrapping** for first run in `notes/`

## Architecture (human version)

Think of Arbor V2 as a **desktop control room** for your notes graph:

1. Notes are parsed into section-level nodes.
2. References are converted into graph links.
3. Rust commands expose graph operations to the UI.
4. The Svelte app calls those commands and renders analysis in real time.

Key backend modules:

- `src-tauri/src/lattice.rs` – indexing, graph loading, scoring, and analysis logic
- `src-tauri/src/commands.rs` – Tauri command bridge
- `src-tauri/src/models.rs` – API response models for frontend consumption

Key frontend modules:

- `src/App.svelte` – main dashboard and interaction flows
- `src/lib/api.ts` – command invocations
- `src/lib/types.ts` – UI type contracts

## Requirements

- **Node.js** (LTS recommended)
- **Rust toolchain** (stable)
- **Tauri prerequisites** for your OS

## Run locally

Install dependencies:

```bash
npm install
```

Start the desktop app (preferred):

```bash
npm run tauri dev
```

If you want frontend-only iteration:

```bash
npm run dev
```

## Verification

Typecheck and frontend validation:

```bash
npm run check
```

Production frontend build:

```bash
npm run build
```

Rust compile check:

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

## Command surface

Tauri commands currently exposed by the backend:

- `bootstrap`
- `index_notes`
- `search_notes`
- `analyze_note_impact`
- `find_note_path`

## Notes directory behavior

- By default, the app uses a local notes directory and can seed starter markdown notes.
- You can point indexing to another path from the UI.
- Indexed state is persisted in a local graph DB managed by the Rust backend.

## Troubleshooting

- If the desktop app fails to launch, verify Tauri prerequisites and Rust install.
- If no nodes appear, run indexing again and check the selected notes path.
- If types fail in frontend checks, run `npm install` to sync lockfile/deps.

## Status

Arbor V2 is in active development and already supports the full local loop:
**index → search → impact → path analysis**.
