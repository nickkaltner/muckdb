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
  ['Northland','Eastvale','Southport','Westend'][1 + (i % 4)::BIGINT]              AS region,
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
CREATE OR REPLACE VIEW events_per_hour   AS SELECT date_trunc('hour', ts) AS hour, count(*) AS events FROM events GROUP BY 1 ORDER BY 1;
CREATE OR REPLACE VIEW events_points     AS SELECT ts, value, kind FROM events;
" >/dev/null

# ---- session dashboard --------------------------------------------------------
"$MUCKDB" session create "$SESSION" --title "muckdb demo" >/dev/null

"$MUCKDB" session post "$SESSION" --name intro --title "About this demo" --md "# muckdb demo

A quick tour of what muckdb can do, all driven from the command line.

- **Bars / pies** from aggregated duckdb views (\`sales\`)
- A **regular** hourly time series (\`sensors\`)
- An **irregular** event stream (\`events\`) — notice how the points per time
  period vary a lot

Click **explore** on any data panel to open it in the faceted table browser
(search, facets, range/date sliders, sorting, stats, CSV/JSON export)." >/dev/null

"$MUCKDB" session tile "$SESSION" --name revenue --title "Revenue by region" \
  --db "$DB" --view sales_by_region --chart bar --x region --y revenue >/dev/null

"$MUCKDB" session tile "$SESSION" --name categories --title "Orders by category" \
  --db "$DB" --view sales_by_category --chart pie --x category --y orders >/dev/null

"$MUCKDB" session tile "$SESSION" --name climate --title "Sensors (regular hourly series)" \
  --db "$DB" --view sensors --chart line --x ts --y temp_c,humidity \
  --caption "A clean, evenly-spaced time series — daily + weekly cycles." >/dev/null

"$MUCKDB" session tile "$SESSION" --name throughput --title "Events per hour (uneven density)" \
  --db "$DB" --view events_per_hour --chart bar --x hour --y events \
  --caption "Bucketed counts — bar heights swing with the per-hour/-day intensity." >/dev/null

"$MUCKDB" session tile "$SESSION" --name scatter --title "Each event over time" \
  --db "$DB" --view events_points --chart scatter --x ts --y value \
  --caption "Every event as a point — clusters show where activity bunched up." >/dev/null

PORT="${MUCKDB_PORT:-11000}"
echo
echo "Done. Open:  http://localhost:${PORT}/session/${SESSION}/"
echo "Explore the data:  http://localhost:${PORT}/  (Databases tab → $(basename "$DB"))"
