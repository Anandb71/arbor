# Arbor v1.6.2 — Language Expansion + Momentum Release (2026-03-24)

Arbor `v1.6.2` is a forward-driving release focused on broadening parser coverage and making the product feel alive again for day-to-day engineering workflows.

## ✨ Highlights

- Added **5+ new parsing language families**:
  - Kotlin (`.kt`, `.kts`)
  - Swift (`.swift`)
  - Ruby (`.rb`)
  - PHP (`.php`, `.phtml`)
  - Shell (`.sh`, `.bash`, `.zsh`)
- Implemented a new **fallback parser engine** for rapid language onboarding
- Extended **MCP bridge / sync watcher coverage** so new extensions update live
- Bumped workspace version to **1.6.2** and aligned internal crate dependency versions

## 🧠 Why this matters

This release reduces adoption friction across mixed-language teams and keeps Arbor useful in polyglot repos even before full grammar-specific parser modules are wired in every path.

## 🔧 Technical Notes

- Fallback parser lives in `crates/arbor-core/src/fallback_parser.rs`
- Legacy parser (`parser.rs`) and query parser (`parser_v2.rs`) now both route unsupported-but-recognized extensions through fallback extraction
- `languages::is_supported` now includes fallback extension checks
- Real-time watch extension lists were updated in CLI visualizer/bridge and server defaults

## ✅ Validation

- Added tests for fallback extension recognition and extraction
- Added parser tests in both parser paths for Kotlin-style symbol extraction

## Next up (toward 1.7)

- Promote fallback-supported languages to full Tree-sitter first-class parsers incrementally
- Add richer relation extraction (imports/calls) for newly added ecosystems
- Expand enterprise policy workflows and AI-assisted audit surfaces
