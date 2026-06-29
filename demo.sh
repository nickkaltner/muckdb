#!/usr/bin/env bash
#
# demo.sh — seed a muckdb demo: sample data + a session dashboard.
#
# Usage:
#   ./demo.sh                       # uses `muckdb` on PATH, db at ~/muckdb-demo/demo.duckdb
#   MUCKDB=./target/release/muckdb ./demo.sh [DB_PATH]
#
# Then open the URL it prints (http://localhost:11000/session/demo/).
set -euo pipefail

MUCKDB="${MUCKDB:-muckdb}"
DB="${1:-$HOME/muckdb-demo/demo.duckdb}"
SESSION="demo"
mkdir -p "$(dirname "$DB")"
export MUCKDB_SESSION="$SESSION"

echo "muckdb demo → seeding $DB"

# ---- sample data --------------------------------------------------------------
# Three flavours of data: categorical sales, a regular hourly sensor series, and
# an irregular event stream whose density varies a lot per hour and per day.
"$MUCKDB" "$DB" -c "
-- Categorical + numeric facts for bars / pies / faceted search.
CREATE OR REPLACE TABLE sales AS
SELECT
  i AS id,
  ['Northland','Eastvale','Southport','Westend','Brightmoor','Cedar Hills',
   'Fairview','Glenwood','Harborline','Ironside','Junewood','Kingsford',
   'Lakeshore','Mistvale'][1 + (i % 14)::BIGINT]                                  AS region,
  ['Widget','Gadget','Gizmo','Sprocket','Cog'][1 + (hash(i*7)  % 5)::BIGINT]      AS product,
  ['Hardware','Software','Services'][1 + (hash(i*13) % 3)::BIGINT]                AS category,
  round(20 + (hash(i*3) % 480) + random()*50, 2)                           AS amount,
  1 + (hash(i*5) % 9)                                                       AS qty,
  (DATE '2026-05-01' + (i % 30)::INTEGER)                                   AS sold_on
FROM range(600) g(i);

-- A clean, regular hourly time series (30 days) — daily + weekly cycles.
CREATE OR REPLACE TABLE sensors AS
SELECT
  TIMESTAMP '2026-05-01' + INTERVAL (h) HOUR                               AS ts,
  round(15 + 9*sin(h/24.0*2*pi()) + 3*sin(h/168.0*2*pi()) + random()*1.5, 2) AS temp_c,
  round(60 + 20*cos(h/24.0*2*pi()) + random()*4, 1)                        AS humidity
FROM range(720) g(h);

-- An IRREGULAR event stream over 30 days. Each minute fires with a probability
-- that swings by time-of-day (busy midday) AND by day (some days far busier),
-- so counts per hour/day vary widely — good for seeing uneven time density.
CREATE OR REPLACE TABLE events AS
SELECT
  TIMESTAMP '2026-05-01' + INTERVAL (m) MINUTE                             AS ts,
  ['login','purchase','error','view','signup'][1 + (hash(m) % 5)::BIGINT]  AS kind,
  round(random()*100, 2)                                                   AS value
FROM range(43200) g(m)
WHERE random() < 0.012
  + 0.06 * pow(sin((m % 1440)/1440.0 * pi()), 2)   -- daily hump
        * (0.15 + (hash(m // 1440) % 100)/110.0);  -- per-day intensity

-- Views: what the dashboard charts and what the human can 'explore'.
CREATE OR REPLACE VIEW sales_by_region   AS SELECT region, round(sum(amount),2) AS revenue FROM sales GROUP BY 1 ORDER BY revenue DESC;
CREATE OR REPLACE VIEW sales_by_category AS SELECT category, count(*) AS orders FROM sales GROUP BY 1 ORDER BY orders DESC;
CREATE OR REPLACE VIEW top_products      AS SELECT product, count(*) AS orders, round(sum(amount),2) AS revenue FROM sales GROUP BY 1 ORDER BY revenue DESC;
-- Categorical breakdown (like HTTP methods) — each bar its own solid colour.
CREATE OR REPLACE VIEW events_by_kind    AS SELECT kind, count(*) AS "count" FROM events GROUP BY 1 ORDER BY "count" DESC;
-- One row per region, revenue split into category columns — the shape a stacked
-- bar wants: x = region, y = the category series that stack into a total.
CREATE OR REPLACE VIEW revenue_by_region_category AS
  SELECT region,
         round(sum(amount) FILTER (category = 'Hardware'), 2) AS hardware,
         round(sum(amount) FILTER (category = 'Software'), 2) AS software,
         round(sum(amount) FILTER (category = 'Services'), 2) AS services
  FROM sales GROUP BY 1 ORDER BY region;
CREATE OR REPLACE VIEW events_per_hour   AS SELECT date_trunc('hour', ts) AS hour, count(*) AS events FROM events GROUP BY 1 ORDER BY 1;
-- Daily volume keyed by a DATE column — one bar per calendar day. (Good test of
-- the time axis: a DATE must sit on its own day, not skew by the viewer's tz.)
CREATE OR REPLACE VIEW sales_per_day     AS SELECT sold_on AS day, count(*) AS orders, round(sum(amount),2) AS revenue FROM sales GROUP BY 1 ORDER BY 1;
CREATE OR REPLACE VIEW events_points     AS SELECT ts, value, kind FROM events;

-- A column comment carries a display format that travels with the database.
COMMENT ON COLUMN sales.qty IS 'order size muckdb:{\"suffix\":\" units\"}';
" >/dev/null

# ---- column display formats ---------------------------------------------------
# Registry formats apply by column name everywhere the column appears (base
# tables AND the derived view columns the charts plot). 'qty' is formatted via
# the column comment above instead. So facets/charts/tables show '$1,234.56 USD'.
"$MUCKDB" format "$DB" amount   --currency USD >/dev/null
"$MUCKDB" format "$DB" revenue  --currency USD >/dev/null
"$MUCKDB" format "$DB" hardware --currency USD >/dev/null
"$MUCKDB" format "$DB" software --currency USD >/dev/null
"$MUCKDB" format "$DB" services --currency USD >/dev/null
"$MUCKDB" format "$DB" temp_c   --suffix '°C' --decimals 1 >/dev/null
"$MUCKDB" format "$DB" humidity --suffix '%'  --decimals 0 >/dev/null

# ---- session dashboard --------------------------------------------------------
"$MUCKDB" session create "$SESSION" --title "muckdb demo" >/dev/null

"$MUCKDB" session post "$SESSION" --name intro --title "About this demo" --md "# muckdb demo 🦆

A quick tour of what muckdb can do, all driven from the command line.

- 📊 **Bars / stacked bars / pies** from aggregated duckdb views (\`sales\`)
- 🌡️ A **regular** hourly time series (\`sensors\`)
- ⚡ An **irregular** event stream (\`events\`) — notice how the points per time
  period vary a lot

## What's in this database

| Table     | Rows  | What it shows                          |
|-----------|------:|----------------------------------------|
| sales     |   600 | 🛒 orders by region / product / category |
| sensors   |   720 | 🌡️ 30 days of hourly temp + humidity     |
| events    |  ~1.4k | ⚡ irregular event stream over 30 days   |

Click **explore** on any data panel to open it in the faceted table browser
(search, facets, range/date sliders, sorting, stats, CSV/JSON export)." >/dev/null

# A panel showing how markdown renders inline `code` and fenced code blocks.
# (quoted heredoc so the backticks stay literal rather than running as commands.)
"$MUCKDB" session post "$SESSION" --name recipe --title "How a panel is built" --md - <<'MD'
Every panel here came from one command. Inline code like `muckdb session tile`
renders as code, and a fenced block shows a whole command:

```
muckdb session tile demo --name revenue --title "Revenue by region" \
  --db demo.duckdb --view sales_by_region \
  --chart bar --x region --y revenue --bars solid
```

Re-run with the same `--name` to update a panel in place.

**Links** render too: the [muckdb repo](https://github.com/nickkaltner/muckdb),
the [docs](https://github.com/nickkaltner/muckdb#readme), or a deep link straight
to this [session dashboard](http://localhost:11000/session/demo/).
MD

"$MUCKDB" session tile "$SESSION" --name revenue --title "Revenue by region" \
  --db "$DB" --view sales_by_region --chart bar --x region --y revenue --bars solid \
  --xlabel Region --ylabel Revenue \
  --caption "Categorical — regions are distinct buckets, so each bar gets its own solid colour. Past the curated palette the colours sweep the hue scale so all 14 stay easy to tell apart." >/dev/null

"$MUCKDB" session tile "$SESSION" --name by_kind --title "Events by type" \
  --db "$DB" --view events_by_kind --chart bar --x kind --y count --bars solid \
  --xlabel "Event type" --ylabel Count \
  --caption "Categorical like HTTP methods (GET/POST/…): solid bars, one colour each." >/dev/null

"$MUCKDB" session tile "$SESSION" --name products --title "Top products" \
  --db "$DB" --view top_products --chart table \
  --caption "A table tile fills to its rows — no inner scrollbar; use the contract icon to shrink it." >/dev/null

"$MUCKDB" session tile "$SESSION" --name categories --title "Orders by category" \
  --db "$DB" --view sales_by_category --chart pie --x category --y orders >/dev/null

"$MUCKDB" session tile "$SESSION" --name region_mix --title "Revenue mix by region" \
  --db "$DB" --view revenue_by_region_category --chart stacked --x region --y hardware,software,services \
  --caption "Stacked bars — each region's total split into Hardware / Software / Services." >/dev/null

"$MUCKDB" session tile "$SESSION" --name climate --title "Sensors (regular hourly series)" \
  --db "$DB" --view sensors --chart line --x ts --y temp_c,humidity \
  --event '2026-05-08T00:00|firmware v2' --event '2026-05-21T12:00|heatwave' \
  --target '25|comfort max' \
  --caption "A clean, evenly-spaced time series — daily + weekly cycles. Vertical lines mark events; the dotted line is a target." >/dev/null

"$MUCKDB" session tile "$SESSION" --name throughput --title "Events per hour (uneven density)" \
  --db "$DB" --view events_per_hour --chart bar --x hour --y events \
  --xlabel "Hour (UTC)" --ylabel "Events" \
  --event '2026-05-15T00:00|marketing email' \
  --caption "Bucketed counts — bar heights swing with the per-hour/-day intensity. The vertical line marks a campaign send." >/dev/null

"$MUCKDB" session tile "$SESSION" --name daily --title "Orders per day" \
  --db "$DB" --view sales_per_day --chart bar --x day --y orders --bars gradient \
  --xlabel "Day" --ylabel "Orders" \
  --caption "Continuous over time — gradient bars. One bar per calendar day; they line up on day boundaries regardless of your timezone." >/dev/null

"$MUCKDB" session tile "$SESSION" --name scatter --title "Each event over time" \
  --db "$DB" --view events_points --chart scatter --x ts --y value \
  --caption "Every event as a point — clusters show where activity bunched up." >/dev/null

# A closing summary panel — the takeaways, so the dashboard reads top-to-bottom.
"$MUCKDB" session post "$SESSION" --name summary --title "Summary" --md "## Summary

This dashboard tours every muckdb panel type from one shell script:

| What                | Panel                          | Why it's shaped this way                 |
|:--------------------|:-------------------------------|:-----------------------------------------|
| **Revenue / types** | solid bars                     | categorical buckets → one colour each    |
| **Orders per day**  | gradient bars on a UTC axis    | continuous over time → gradient          |
| **Sensors**         | line + event & target markers  | a clean series with annotations          |
| **Top products**    | table (fills, no scroll)       | small result set read in full            |
| **Each event**      | scatter                        | raw points show where activity bunched   |

Every figure here is backed by a **view** you can open (hit **explore**),
re-sort, filter, and export — nothing is take-my-word-for-it." >/dev/null

PORT="${MUCKDB_PORT:-11000}"
echo
echo "Done. Open:  http://localhost:${PORT}/session/${SESSION}/"
echo "Explore the data:  http://localhost:${PORT}/  (Databases tab → $(basename "$DB"))"
