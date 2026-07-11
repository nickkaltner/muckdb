# Future directions

Speculative ideas for where `muckdb` could go next. Nothing here is committed —
each section lays out the concept, the design forks, and the trade-offs so we can
decide with eyes open. Where an idea builds on something that already exists in
the codebase, that's called out (the point is to reuse machinery, not reinvent
it).

Two ideas so far:

1. **Resource resolver** — turn a `(domain, purpose, id)` tuple into "here's
   everywhere you can go to look at this thing".
2. **Scheduler** — let a session refresh its own data on a cadence so the
   dashboard doesn't rot.

---

## 1. Resource resolver

### The idea

Address any *thing* in your world as a small tuple:

```
(domain, purpose, id)        →  (yourcompany, users, 3f9c…-uuid)
(env, domain, purpose, id)   →  (prod, yourcompany, users, 3f9c…-uuid)
```

- **domain** — who owns it (`yourcompany`, a team, a system).
- **purpose** — what kind of thing it is (`users`, `orders`, `ports`, `invoices`).
- **id** — the specific instance (uuid, slug, numeric id).
- **env** — optional prefix (`prod`, `staging`, `dev`) that shifts which URLs and
  databases the resolver points at.

Given a tuple, a resolver endpoint answers: **"what are all the ways to view
this?"** — links to internal admin portals, external sites, dashboards, and the
actual rows in databases muckdb has already touched. One lookup, every door.

```sh
muckdb resolve prod/yourcompany/users/3f9c…       # everything about this user
muckdb resolve yourcompany/users/3f9c…            # env-agnostic
```

And crucially, **reverse / fuzzy lookup** when you only have half the tuple:

```sh
muckdb resolve --id 3f9c…                 # search every purpose & domain for this id
muckdb resolve --purpose users --id bob   # search domains for a users/bob
```

### Why muckdb is the right home for this

muckdb already has two of the three ingredients:

- **A URL-template engine** (`src/formats.rs`) — the `--link` / `--link-title`
  system with `{value}`/`{any_column}` substitution and `:raw`/`:url` encoding
  controls. A resource resolver is essentially *this same engine promoted from
  "a column" to "a resource type"*.
- **A registry of every database it has touched** (`src/store.rs`,
  `muckdb ls databases` / `ls tables`). That's a ready-made index for the
  "find the id in the data" half of the problem — no new catalog required.

The only genuinely new piece is a **mapping from `purpose` → a set of view
templates**.

### Design fork: where do the mappings come from?

This is the decision that shapes everything. Four options, roughly in order of
how much machinery they need. **(Current lean: config/template registry as the
backbone, with auto-derived DB lookup as a free complement — but see the table.)**

#### Option A — Config / template registry *(recommended backbone)*

A declarative registry keyed by `purpose` (optionally scoped by `domain`/`env`),
each entry a set of link templates — the `format --link` model, one level up.

```toml
# ~/.local/share/muckdb/resolvers.toml   (illustrative)
[users]
links = [
  { title = "Admin portal", url = "https://admin.{env}.yourco.com/users/{id}" },
  { title = "Stripe",       url = "https://dashboard.stripe.com/customers/{id}" },
]
# point at a row in a db muckdb already knows
db  = { path = "~/data/app.duckdb", table = "users", key = "uuid" }

[users.env.prod]   # env override just swaps the substitutions
```

| Pros | Cons |
|------|------|
| Reuses the existing, tested substitution engine (`{value}`, `:raw`/`:url`) | You must author the registry up front (though templates are cheap) |
| Zero code to add a new resource type — just config | Doesn't *discover* anything you didn't declare |
| Portable, diffable, reviewable; travels with a repo/dotfiles | Env matrix can get verbose without good inheritance |
| Same mental model users already learned from `--link` | |

#### Option B — Auto-derived from touched databases *(recommended complement)*

muckdb already knows every DB and every column. Given an id, scan candidate
columns across known tables for a match and offer the row + the faceted explorer
view. **This is what powers the "I only have the id" search essentially for
free.**

| Pros | Cons |
|------|------|
| Powers reverse/fuzzy lookup with no config at all | A raw scan is O(tables × rows); needs an id→(table,col) index to be fast |
| Improves automatically as muckdb touches more DBs | Ambiguous: the same uuid shape can appear in many columns |
| No mapping to maintain | Only reaches data muckdb has seen — not external systems |

#### Option C — Pluggable providers

Each `domain` registers a resolver (a script or HTTP endpoint) muckdb shells out
to; it returns links, live status, and *related* resources to chase next.

| Pros | Cons |
|------|------|
| Most powerful: live status, cross-references, computed links | Most machinery; a plugin protocol + trust/security model |
| Domains own their own logic | Slower (network per lookup); failure modes to handle |
| Natural fit for graph-walking ("show me this user's orders") | Overkill until A/B are proven insufficient |

#### Option D — External catalog federation

Don't own the mapping at all — resolve against an existing service catalog /
CMDB / internal directory.

| Pros | Cons |
|------|------|
| Single source of truth; no duplication | Requires that catalog to exist and expose an API |
| Stays correct as the org changes | Couples muckdb to an external dependency |
| | Least useful for a solo/local user with no catalog |

#### Reading of the options

A and B are complementary and both cheap: **A** gives curated, external-reaching
links from config you already know how to write; **B** gives the reverse-lookup
"find it by id" superpower from data muckdb already holds. Together they cover the
forward and reverse cases without a plugin protocol. **C** and **D** are natural
*later* upgrades once the tuple model has proven itself — C when a domain needs
live/graph data, D when there's a real catalog to defer to.

### Reverse / fuzzy search ("I only know the id")

Layered so it's cheap first, thorough second:

1. **Registry hint** — if any `purpose` declares an id shape/regex that matches,
   try those resolvers first (Option A).
2. **Data scan** — search indexed id-like columns across known DBs (Option B).
3. **Rank & disambiguate** — return candidates grouped by `(domain, purpose)`
   with a confidence signal, rather than guessing one.

### Surfacing it

- **CLI**: `muckdb resolve …` prints JSON (links, matched rows, related
  resources) — same read-it-back-as-JSON ergonomics as the `ls` family.
- **Web**: a `/resolve/<env>/<domain>/<purpose>/<id>` page — a card of links plus
  embedded row/explorer views for the DB matches. A resource becomes a
  first-class, linkable object in the UI.
- **Sessions**: a resolver-backed tile or an inline `{{resolve:...}}` in markdown
  so a dashboard can deep-link the exact resources it's about.

### Open questions

- Tuple **syntax**: path-style `env/domain/purpose/id` vs flags. Path is
  URL-friendly and matches the web route; flags are clearer for partial tuples.
- **Id-shape collisions** across purposes — how much disambiguation UI is worth
  building.
- Does `env` **prefix** or **override**? (Prefix = separate namespaces; override
  = same resolver, swapped substitutions. Override is less duplication.)
- Where the registry **lives**: global data dir, per-project file, or a DuckDB
  column-comment-style embed that travels with a database.

---

## 2. Scheduler — keep sessions fresh

### The idea

A session is a live dashboard, but today it's only as fresh as the last time an
agent re-ran the commands that built it. Let a session **schedule a refresh**:
run some script on a cadence so views re-materialize, source data re-imports, and
tiles re-post — without a human or agent in the loop.

Three questions decide the shape:

1. **What runs** (the job definition)?
2. **How the cadence is expressed** (the format)?
3. **Who executes it** (the runner)?

### (1) What runs

| Approach | What it is | Trade-off |
|----------|-----------|-----------|
| **Explicit refresh script** | A shell snippet of `muckdb …` (and any fetch) commands the author writes and attaches to the session | Most flexible & honest — handles "re-fetch from API *then* re-post". Author must write it. **Recommended.** |
| **Ledger replay** | Re-run the commands the ledger already recorded for this `MUCKDB_SESSION` (`src/store.rs` `Record` has `args`/`cwd`/`db_path`/`session`) | Zero authoring — the history is already captured. But fragile: replays *everything*, including one-off setup, and can't reproduce data fetched outside muckdb. |
| **View re-materialize only** | Just refresh the views/tiles | Simplest, but duckdb views are lazy — "refresh" only means something if the *underlying data* changed, which this approach can't cause. |

The refresh script is the safe default; ledger replay is a tempting
"just works" shortcut worth prototyping but likely too blunt on its own.

### (2) How the cadence is expressed — format candidates

This is the part to weigh carefully.

#### Candidate 1 — Field on the session JSON

Add `schedule` + `refresh` to the session file itself.

```json
{ "name": "pond-analysis",
  "schedule": "*/15 * * * *",
  "refresh": { "cwd": "~/proj", "script": "fetch.sh && muckdb session tile …" } }
```

| Pros | Cons |
|------|------|
| Self-contained — the schedule travels with `session export` | Couples "what to show" with "how to refresh" |
| One place to look; obvious ownership | Editing a script inside JSON is awkward |

#### Candidate 2 — Standalone job registry *(recommended)*

Jobs live in their own files (or a `jobs.jsonl` ledger beside `history.jsonl`),
each referencing a session. Managed by a `muckdb schedule` subcommand.

```sh
muckdb schedule add pond-analysis --every 15m --script ./refresh.sh
muckdb schedule list          # JSON, like the ls family
muckdb schedule run pond-analysis   # run now
muckdb schedule rm pond-analysis
```

| Pros | Cons |
|------|------|
| Clean separation; refresh script is a real file you can test | A second registry to keep consistent with sessions |
| Matches muckdb's existing CLI + JSON-readback idioms | Export must decide whether to bundle the job |
| Easy to list/inspect all jobs at once | |

#### Candidate 3 — Emit to the OS scheduler

muckdb writes a real cron entry / systemd timer / launchd plist that invokes the
script.

| Pros | Cons |
|------|------|
| Survives daemon restarts and reboots for free | Cross-platform pain: cron vs systemd vs launchd (three code paths) |
| Battle-tested runners; no in-process scheduling to maintain | Jobs live *outside* muckdb — harder to list/inspect/clean up |
| | Permissions/PATH/env drift between the shell and the timer |

#### Candidate 4 — Per-tile refresh directives

Each tile carries its own `--refresh "cmd" --every 5m`; tiles self-refresh.

| Pros | Cons |
|------|------|
| Fine-grained — only the expensive tiles refresh often | Fragmented; no single "refresh the whole report" story |
| Co-located with the thing being refreshed | N schedules per session to reason about |

#### Candidate 5 — Ledger replay on a timer

Combine "what runs = ledger replay" with any cadence: "re-run everything this
session did, every hour."

| Pros | Cons |
|------|------|
| Nearly zero setup | Inherits ledger-replay fragility (setup cmds, external fetches) |
| Great for pure-SQL sessions over a live DB | Wrong for anything with a fetch step |

### (3) Who executes it

| Runner | Pros | Cons |
|--------|------|------|
| **In-daemon scheduler** | Simplest UX; muckdb is meant to stay running; can show next-run/last-run in the UI live | Misses runs while the daemon is down |
| **OS timers** (Candidate 3) | Robust across reboots | Cross-platform, opaque, harder to manage |
| **Hybrid** *(recommended)* | Daemon schedules while alive **and** on startup catches up any missed runs (compare `now` to `last_run` + interval) | A little more logic, but best of both |

### Recommendation to prototype first

**Standalone job registry (Candidate 2) + explicit refresh script + hybrid
in-daemon runner with catch-up.** It matches muckdb's existing idioms (a CLI
verb, a JSONL ledger, JSON readback), keeps "what to show" separate from "how to
refresh", and degrades gracefully when the daemon has been off. Ledger replay
(Candidate 5) is worth a spike as a zero-config option for pure-SQL sessions.

### Cross-cutting concerns (whichever format wins)

- **Freshness UX**: show *last refreshed* / *next run* on the session and a
  manual **Refresh now** button — the data's age should never be a guess.
- **Failure surfacing**: a failed scheduled run must show up *in the session*
  (a banner / status tile), not vanish into a log. Reuse the ledger's
  `exit_code`.
- **Concurrency & cost**: don't stack runs if one overruns its interval; make
  refreshes idempotent (tiles already upsert by `--name`, which helps).
- **Security**: a scheduled script runs arbitrary commands unattended — needs the
  same care as any cron job (explicit opt-in, visible in `schedule list`).

### Open questions

- Cadence grammar: friendly `--every 15m` vs full cron strings vs both.
- Does a scheduled job belong *inside* `session export`, or is it host-local?
- What "success" means: exit code only, or assert the tiles actually re-posted?

---

## How the two ideas reinforce each other

A scheduled job is a natural way to **keep resolver data fresh** — e.g. re-import
the table that the resource resolver scans for id matches (Option B) on a
cadence, so reverse lookups stay accurate. Build the resolver's data index once,
and the scheduler keeps it warm.
