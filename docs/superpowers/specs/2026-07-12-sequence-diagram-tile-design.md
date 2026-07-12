# Sequence Diagram Tile — Design

**Status:** approved (brainstorm), 2026-07-12
**Feature branch:** `feat/sequence-tile`

## Goal

Add a new muckdb session tile of `--chart sequence` that renders a **sequence
diagram** — primarily for showing the **interactions/comms between
microservices** — from a single view (one row per message), and offers a button
to **export the diagram as mermaid.js `sequenceDiagram` text**.

## Why

muckdb dashboards present analysis to a human. Service-to-service call flows
(request/response, async fan-out, retries, fallbacks) are naturally a sequence
diagram, and mermaid is the lingua franca for pasting such a diagram into docs,
PRs, and wikis. A hand-rolled tile gives us richer participant shapes than
mermaid supports natively (database, boundary) while still exporting to valid
mermaid.

## Architecture (summary)

A `sequence` tile is a **hand-rolled SVG renderer** in the frontend, wired in
exactly like the existing `timeline` tile: a `Chart` field group + CLI mapping +
a `validate_tile` branch (Rust), a `sequenceHtml()` renderer + `seqPayloads`
payload store + `hydrateSequences()` driver dispatched from `loadTileChart`,
`zoomTile`, and `panelHtml` (frontend), plus docs and tests. Mermaid is an
**export format only**, generated client-side; it is **not** the renderer.

No chart-kind allowlist exists (`kind` is a free `String`), so the addition is
purely additive. The `/api/query` backend is generic and needs no change.

## Data model — one view, one row per message

Every muckdb tile is "one row per thing" from a view or inline SQL. A sequence
tile is **one row per message**. Participants and their types are **inferred**
from the message rows (model "C" from the brainstorm — no separate participants
view).

### CLI flags → Chart fields

| CLI flag | Required | Chart field | Meaning |
|---|---|---|---|
| `--from COL` | ✓ | `from_participant: Option<String>` | source participant column |
| `--to COL` | ✓ | `to_participant: Option<String>` | destination participant column (`from == to` → self-message) |
| `--label COL` | ✓ | `label: Option<String>` (existing field, reused) | the message text |
| `--message-type COL` | – | `message_type: Option<String>` | per-message arrow kind: `sync` (default) · `reply` · `async` · `lost` |
| `--from-type COL` | – | `from_type: Option<String>` | participant shape for the source: `participant` (default) · `actor` · `database` · `boundary` |
| `--to-type COL` | – | `to_type: Option<String>` | participant shape for the destination (same vocabulary) |
| `--group COL` | – | `group_col: Option<String>` | `kind:label` frame spec (`loop:`/`opt:`/`alt:`/`par:`) |
| `--group-branch COL` | – | `group_branch: Option<String>` | `else`/`and` compartment label within a frame |
| `--autonumber` | – | `autonumber: bool` | number messages 1, 2, 3 … down the diagram |

Notes:

- **`from` is a Rust keyword** — hence the struct field names
  `from_participant` / `to_participant` (CLI flags stay `--from` / `--to`).
- All new fields follow the existing `Option<String>` +
  `#[serde(default, skip_serializing_if = "Option::is_none")]` convention
  (and `bool` + `skip_serializing_if = "is_false"` for `autonumber`) so other
  tile kinds' JSON is unaffected. `Chart` does not derive `Default`, so **all
  ~8 `Chart {..}` literals** (the CLI constructor, the `src/export.rs:396` test
  fixture, and the serde/validation test literals in `src/session.rs`) must gain
  the new fields to compile.

### Ordering

- **Message order** = row order — the user controls it with `ORDER BY` in the
  view (same convention as the timeline tile). No order flag.
- **Participant order** = order of first appearance scanning rows top to bottom
  (`from` then `to` within each row), same as timeline lanes.
- A participant's **type** is taken from the `from_type`/`to_type` value on the
  row where it first appears; absent → `participant` (plain box).

### The `--group` value grammar

- A cell value is `kind:label`, e.g. `loop:retry 3x`, `opt:if premium`,
  `alt:auth ok`, `par:fan-out`. `kind` ∈ {`loop`, `opt`, `alt`, `par`}.
- **Contiguous** rows carrying the **same** `--group` value form **one frame**
  drawn around those messages, labelled with the kind and the label text.
- A null/empty `--group` cell → the message is ungrouped.
- **Compartments:** within one frame, when `--group-branch` changes value
  between rows, a divider is drawn labelled with the new branch value
  (mermaid `else` for `alt`, `and` for `par`).
- **Single-level only in v1** — no nesting of one frame inside another. (A
  contiguous run of equal `--group` values is one flat frame.) Nesting is
  explicitly deferred.

### Validation (`validate_tile`, new `kind == "sequence"` branch)

Mirrors the timeline block:

- `--from`, `--to`, `--label` are **required**; bail with a clear message if any
  is missing.
- Every provided column flag (`--from`/`--to`/`--label`/`--message-type`/
  `--from-type`/`--to-type`/`--group`/`--group-branch`) must be a real column of
  the resolved relation; unknown → the standard "did you mean" suggestion +
  available-columns list (reuse the existing `check` closure / `closest`
  helper).
- No type-agreement checks are needed (all are text/category columns).
- `--autonumber` is a valueless boolean flag (add to `BOOL_FLAGS`).

## Rendering (hand-rolled SVG)

Mirrors the timeline pipeline:

- `sequenceHtml(cols, rows, spec, db, table)` returns markup and stashes an
  overlay payload in a `seqPayloads` map keyed by a `data-seq="<id>"` attribute
  (exactly the `tlPayloads` / `data-tl` mechanism).
- `hydrateSequences(slot)` (called right after `innerHTML` assignment in
  `loadTileChart` and `zoomTile`) reads the payload and draws the SVG.
- Dispatch added to the three parallel sites (`loadTileChart` ~line 3548,
  `zoomTile` ~line 3030, and the `panelHtml` toolbar), plus modal sizing (`fit`
  class) in `zoomTile` like timeline/box.

Visual elements:

- **Participant headers** across the top, each drawn per its type:
  - `participant` — labelled rounded box (default)
  - `actor` — stick figure above the label
  - `database` — cylinder
  - `boundary` — UML boundary (circle with a vertical bar + stem)
- **Lifelines** — a vertical dashed line dropping from each participant.
- **Messages** — a horizontal arrow from the source lifeline to the destination
  lifeline at successive vertical steps, with the label above the line:
  - `sync` — solid line, filled arrowhead
  - `reply` — dashed line, filled arrowhead
  - `async` — solid line, open arrowhead
  - `lost` — solid line ending in a cross (no destination arrowhead)
- **Self-message** (`from == to`) — a small loop-back arrow on the participant's
  own lifeline with the label to its right.
- **Group frames** — a rectangle around the messages in a frame, a small
  label tab in the top-left showing `kind` + label, and horizontal dashed
  **compartment dividers** with the branch label where `--group-branch` changes.
- **Autonumber** — a small numbered badge on each message when `--autonumber`
  is set.
- **Hover a message** → the standard rich tooltip (the `tlTip`-style delegated
  `.wm-x` + `data-tip` mechanism): `from → to`, the label, the message type, and
  **every other row column** rendered through its column format/link — so a
  `--link` on a trace-id column becomes a clickable link in the tooltip.

Full-width toggle: extend the `widenBtn` gate in `panelHtml` to include
`sequence` (like `table`/`timeline`), since wide diagrams benefit.

## Mermaid export

A **"export mermaid"** button in the panel toolbar, gated to `kind ===
"sequence"` (add to the `panel-actions` row + a delegated `data-mermaid` click
branch). Clicking it generates a valid mermaid `sequenceDiagram` document
client-side from the same rows/spec and **copies it to the clipboard**, showing
a confirmation toast (matching the existing copy-image button's pattern).

Generated document:

- Header line `sequenceDiagram`.
- `autonumber` line if the flag is set.
- One participant declaration per participant, in appearance order:
  - `actor` type → `actor <Name>`
  - `database` / `boundary` / plain → `participant <Name>`, preceded by a
    `%% database` or `%% boundary` comment line when the type is one of those,
    so the intended shape survives round-trip in a mermaid-valid way.
  - Names with spaces/special chars use the `participant <id> as <Label>` form
    (or are otherwise made mermaid-safe).
- Messages in row order, mapping message-type → arrow:
  `sync`→`->>`, `reply`→`-->>`, `async`→`-)`, `lost`→`-x`, e.g.
  `gateway->>auth: POST /login`.
- Group frames: `loop <label>` / `opt <label>` / `alt <label>` / `par <label>`
  wrapping their messages and closed with `end`; compartment changes emit
  `else <branch>` (alt) or `and <branch>` (par).

The button is client-side only; no new server route.

## Docs

- **`src/assets/skill/SKILL.md`**: add `sequence` to the `session tile` usage
  block and the kind list, and a dedicated **`## Sequence tiles`** section that
  **prominently states it is used to show interactions between microservices**,
  documents all flags, the participant types, the message/arrow vocabulary, the
  `--group` grammar + compartments, `--autonumber`, the mermaid-export button
  (and the db/boundary → `participant` mapping caveat), with a worked example.
- **`CLAUDE.md`**: parallel updates to the usage block and kind list.
- Keep the `session.rs` usage strings in sync (the `--chart sequence` line).

## Tests

- **Rust** (`src/session.rs` `mod tests`):
  - `sequence_chart_serde_roundtrips_fields` — round-trips the new fields and
    confirms other kinds omit them (mirror
    `timeline_chart_serde_roundtrips_fields`).
  - `sequence_validation_requires_core_flags` — builds a small messages table
    and asserts: missing `--from`/`--to`/`--label` each err; a bad column name
    errs with a suggestion; a fully-specified valid spec passes (mirror
    `timeline_validation_requires_core_flags`).
- **E2E** (`tests/e2e/`):
  - Add a `messages` view to `fixtures/seed.ts` (participants of each type, all
    four arrow types, a self-message, an `alt`+`else` group, and a column with a
    `--link` format for tooltip-link coverage) and post a `--chart sequence`
    tile.
  - New `specs/sequence.spec.ts`: participant headers of each shape render;
    correct number of message arrows; a self-message loop renders; a group frame
    + compartment divider render; the message tooltip shows core fields + an
    extra column and **escapes** a hostile value (XSS regression, matching the
    timeline tooltip test); the **export-mermaid** button copies text containing
    `sequenceDiagram` and the expected arrow syntax to the clipboard.

## Out of scope (deferred)

- **Activation bars** on lifelines (request↔reply pairing) — not in v1.
- **Notes** (`note over`/`note right of`) — not in v1.
- **Nested** group frames — v1 is single-level only.
- Importing mermaid text (export only).

## Global constraints

- CI gates must stay green: `cargo fmt --check` → `cargo clippy --all-targets --
  -D warnings` → `cargo build` → `cargo test`, plus the Playwright e2e suite.
- New fields must not appear in other tile kinds' serialized JSON
  (`skip_serializing_if`).
- Every tile must keep requiring `--caption` (existing convention).
- Follow the timeline tile's structure and naming wherever a parallel exists.
