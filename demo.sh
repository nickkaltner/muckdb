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
-- Categorical + numeric facts for bars / pies / faceted search. Amounts are
-- deliberately skewed (exponential tails, different scale per category) so
-- distributions differ: Hardware is big-ticket with a long tail, Services
-- cheap and tight — box plots and histograms have real shape to show.
CREATE OR REPLACE TABLE sales AS
WITH base AS (
  SELECT i,
    ['Northland','Eastvale','Southport','Westend','Brightmoor','Cedar Hills',
     'Fairview','Glenwood','Harborline','Ironside','Junewood','Kingsford',
     'Lakeshore','Mistvale'][1 + (i % 14)::BIGINT]                             AS region,
    ['Widget','Gadget','Gizmo','Sprocket','Cog'][1 + (hash(i*7)  % 5)::BIGINT] AS product,
    ['Hardware','Software','Services'][1 + (hash(i*13) % 3)::BIGINT]           AS category
  FROM range(600) g(i))
SELECT
  i AS id, region, product, category,
  round(CASE category
    WHEN 'Hardware' THEN 180 + 140 * (-ln(random()))   -- big-ticket, long tail
    WHEN 'Software' THEN  60 +  35 * (-ln(random()))   -- mid-priced, moderate tail
    ELSE                  18 +  10 * (-ln(random()))   -- cheap and tight
  END, 2)                                                                   AS amount,
  1 + (hash(i*5) % 9)                                                       AS qty,
  (DATE '2026-05-01' + (i % 30)::INTEGER)                                   AS sold_on
FROM base;

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

-- A CI/deploy pipeline for a relative-seconds Gantt: several lanes, an overlap
-- (build/test run concurrently → sublane), colour by outcome, and a dependency
-- chain (checkout → build → {test, package} → deploy).
CREATE OR REPLACE TABLE pipeline AS SELECT * FROM (VALUES
  ('runner-1', 'checkout',  0.0,   12.0, 'ok',      'p1', NULL),
  ('runner-1', 'build',     12.0,  95.0, 'ok',      'p2', 'p1'),
  ('runner-2', 'unit tests',95.0, 180.0, 'ok',      'p3', 'p2'),
  ('runner-2', 'lint',      95.0, 130.0, 'warn',    'p4', 'p2'),   -- overlaps unit tests
  ('runner-1', 'package',  180.0, 210.0, 'ok',      'p5', 'p3'),
  ('runner-2', 'deploy',   220.0, 270.0, 'failed',  'p6', 'p5')  -- in runner-2: dep connector crosses lanes, starts 10s after package ends
) t(resource, step, t0, t1, outcome, sid, parent);

-- An incident timeline on an absolute time axis: phases per system, with
-- colour by severity and event markers for the key moments.
-- Each phase carries a ticket id; a --link format (below) turns it into a
-- clickable tracker link inside the hover tooltip.
CREATE OR REPLACE TABLE incident AS SELECT * FROM (VALUES
  ('api',      'elevated errors', TIMESTAMP '2026-05-01 14:02:00', TIMESTAMP '2026-05-01 14:18:00', 'warning',  'INC-4021'),
  ('api',      'outage',          TIMESTAMP '2026-05-01 14:18:00', TIMESTAMP '2026-05-01 14:41:00', 'critical', 'INC-4021'),
  ('database', 'failover',        TIMESTAMP '2026-05-01 14:22:00', TIMESTAMP '2026-05-01 14:35:00', 'critical', 'INC-4022'),
  ('oncall',   'investigate',     TIMESTAMP '2026-05-01 14:10:00', TIMESTAMP '2026-05-01 14:30:00', 'info',     'INC-4021'),
  ('oncall',   'mitigate',        TIMESTAMP '2026-05-01 14:30:00', TIMESTAMP '2026-05-01 14:41:00', 'info',     'INC-4023')
) t(system, phase, started, ended, severity, ticket);

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
-- One row per box — the shape a box plot wants: label, min/q1/median/q3/max
-- (aggregated here, not in the chart), and a note shown under each label.
CREATE OR REPLACE VIEW amount_spread AS
  SELECT category,
         round(min(amount), 2)                    AS lo,
         round(quantile_cont(amount, 0.25), 2)    AS q1,
         round(median(amount), 2)                 AS med,
         round(quantile_cont(amount, 0.75), 2)    AS q3,
         round(max(amount), 2)                    AS hi,
         count(*)::VARCHAR || ' orders'           AS note
  FROM sales GROUP BY 1 ORDER BY med DESC;
-- Two categorical axes + a value — the shape a heatmap wants (one row per
-- weekday × hour pair). Axis order follows row order, so build the FULL grid
-- (cross join + left join): sparse data can't scramble the axes, and a silent
-- hour is an honest 0 rather than a missing cell.
CREATE OR REPLACE VIEW events_heat AS
  WITH hours AS (SELECT lpad(h::VARCHAR, 2, '0') AS hour, h FROM range(24) t(h)),
       days  AS (SELECT dayname(DATE '2026-05-04' + d::INTEGER) AS weekday, d FROM range(7) t(d))
  SELECT hours.hour, days.weekday, count(e.ts) AS events
  FROM hours CROSS JOIN days
  LEFT JOIN events e ON hour(e.ts) = hours.h AND isodow(e.ts) = days.d + 1
  GROUP BY hours.hour, hours.h, days.weekday, days.d
  ORDER BY days.d, hours.h;

-- Geographic points for a MAP tile: ~400 customers jittered around 10 cities.
CREATE OR REPLACE TABLE customers AS
  WITH cities AS (
    SELECT row_number() OVER () - 1 AS idx, city, lat, lon FROM (VALUES
      ('Sydney',-33.87,151.21),('Tokyo',35.68,139.69),('London',51.51,-0.13),
      ('New York',40.71,-74.01),('Sao Paulo',-23.55,-46.63),('Cape Town',-33.92,18.42),
      ('Singapore',1.35,103.82),('Los Angeles',34.05,-118.24),('Berlin',52.52,13.40),
      ('Mumbai',19.08,72.88)) c(city, lat, lon))
  SELECT g.i AS id, ci.city,
         round(ci.lat + (random() - 0.5) * 5, 4) AS latitude,
         round(ci.lon + (random() - 0.5) * 5, 4) AS longitude
  FROM range(400) g(i) JOIN cities ci ON ci.idx = (hash(g.i) % 10);
CREATE OR REPLACE VIEW customer_map AS SELECT id, city, latitude, longitude FROM customers;
-- Named city coordinates, reused as connection endpoints for a "flows" map.
CREATE OR REPLACE TABLE city_pts AS SELECT * FROM (VALUES
  ('Sydney',-33.87,151.21),('Tokyo',35.68,139.69),('London',51.51,-0.13),
  ('New York',40.71,-74.01),('Sao Paulo',-23.55,-46.63),('Cape Town',-33.92,18.42),
  ('Singapore',1.35,103.82),('Los Angeles',34.05,-118.24),('Berlin',52.52,13.40),
  ('Mumbai',19.08,72.88)) c(city, lat, lon);
-- Backbone links between cities: each row is a connection (two endpoints), drawn
-- as a semi-transparent arc on the hi-fi map with a capacity-weighted width.
CREATE OR REPLACE VIEW network_flows AS
  WITH pairs(src, dst, gbps) AS (VALUES
    ('Singapore','Tokyo',120),('Singapore','Mumbai',90),('Singapore','Sydney',75),
    ('London','New York',200),('London','Berlin',140),('London','Cape Town',60),
    ('New York','Los Angeles',110),('New York','Sao Paulo',80),
    ('Tokyo','Los Angeles',95),('Mumbai','London',70))
  SELECT p.src AS from_city, p.dst AS to_city, p.src || ' → ' || p.dst AS label, p.gbps,
         s.lat AS from_lat, s.lon AS from_lon, d.lat AS to_lat, d.lon AS to_lon
  FROM pairs p JOIN city_pts s ON s.city = p.src JOIN city_pts d ON d.city = p.dst;
-- Running revenue total by day — the shape an AREA chart likes (cumulative over time).
CREATE OR REPLACE VIEW revenue_cumulative AS
  SELECT day, round(sum(revenue) OVER (ORDER BY day), 2) AS cumulative_revenue FROM sales_per_day;

-- Timeline (Gantt-style) views: one row per bar, lane/label/start/end plus
-- optional colour, id and depends-on columns for connectors.
CREATE OR REPLACE VIEW pipeline_timeline AS
  SELECT resource, step, t0, t1, outcome, sid, parent FROM pipeline;
CREATE OR REPLACE VIEW incident_timeline AS
  SELECT system, phase, started, ended, severity, ticket FROM incident ORDER BY started;

-- Sequence diagram view: one row per message in a checkout flow across services.
-- Participants are typed (actor/boundary/database/participant), messages carry an
-- arrow kind, and an alt group splits the session-valid path from the expired one.
CREATE OR REPLACE VIEW service_calls AS SELECT * FROM (VALUES
  (1,  'user','gateway', 'POST /checkout',       'sync',  'actor',      'participant', NULL,               NULL,      'trace-8a01'),
  (2,  'gateway','auth',  'verify session',       'sync',  'participant','boundary',    'alt:session valid','valid',   'trace-8a02'),
  (3,  'auth','db',       'SELECT session',       'sync',  'boundary',   'database',    'alt:session valid','valid',   'trace-8a03'),
  (4,  'gateway','orders','create order',         'sync',  'participant','participant', 'alt:session valid','valid',   'trace-8a04'),
  (5,  'orders','payments','charge card',         'sync',  'participant','participant', 'alt:session valid','valid',   'trace-8a05'),
  (6,  'payments','orders','payment ok',          'reply', 'participant','participant', 'alt:session valid','valid',   'trace-8a06'),
  (7,  'orders','db',     'INSERT order',         'sync',  'participant','database',    'alt:session valid','valid',   'trace-8a07'),
  (8,  'gateway','user',  '401 unauthorized',     'reply', 'participant','actor',       'alt:session valid','expired', 'trace-8a08'),
  (9,  'orders','orders', 'publish order.created','async', 'participant','participant', NULL,               NULL,      'trace-8a09'),
  (10, 'payments','gateway','webhook',            'lost',  'participant','participant', NULL,               NULL,      'trace-8a10')
) t(seq, caller, callee, message, msg_type, caller_type, callee_type, grp, branch, trace) ORDER BY seq;

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
# The box-plot view's five quantile columns — same money format as amount.
for col in lo q1 med q3 hi; do "$MUCKDB" format "$DB" "$col" --currency USD >/dev/null; done
"$MUCKDB" format "$DB" temp_c   --suffix '°C' --decimals 1 >/dev/null
"$MUCKDB" format "$DB" humidity --suffix '%'  --decimals 0 >/dev/null
"$MUCKDB" format "$DB" cumulative_revenue --currency USD >/dev/null
"$MUCKDB" format "$DB" gbps --suffix ' Gbps' --thousands >/dev/null
# Link formats: id/reference columns become clickable in the timeline tooltips.
# 'ticket' → an incident tracker; 'sid' (a pipeline step id) → its CI build page.
"$MUCKDB" format "$DB" ticket --table incident_timeline \
  --link 'https://tracker.example.com/{value}' --link-title 'open {value}' >/dev/null
"$MUCKDB" format "$DB" sid --table pipeline_timeline \
  --link 'https://ci.example.com/builds/{value}' --link-title 'build {value} · {step}' >/dev/null
# 'trace' (a distributed-trace id on each message) → its trace viewer, so the
# sequence tile's hover tooltip carries a clickable link.
"$MUCKDB" format "$DB" trace --table service_calls \
  --link 'https://trace.example.com/{value}' --link-title 'trace {value}' >/dev/null
# Show the incident timeline in the viewer's LOCAL zone (timestamps are UTC in the
# db; without a --tz they render in UTC). The hover readout then also shows the
# UTC instant so a local-time axis stays unambiguous.
"$MUCKDB" format "$DB" started --table incident_timeline --tz local >/dev/null
"$MUCKDB" format "$DB" ended   --table incident_timeline --tz local >/dev/null

# ---- session dashboard --------------------------------------------------------
# Rebuild from scratch so tiles land in this script's order (a pre-existing demo
# keeps old tile positions and would append new panels out of order).
"$MUCKDB" session rm "$SESSION" >/dev/null 2>&1 || true
"$MUCKDB" session create "$SESSION" --title "muckdb demo" >/dev/null

"$MUCKDB" session post "$SESSION" --name intro --title "About this demo" --md "# muckdb demo 🦆

A quick tour of what muckdb can do, all driven from the command line.

- 📊 **Bars / stacked bars / pies** from aggregated duckdb views (\`sales\`)
- 🌡️ A **regular** hourly time series (\`sensors\`)
- ⚡ An **irregular** event stream (\`events\`) — notice how the points per time period vary a lot
- 🔥 A **heatmap** (weekday × hour density) and 📦 **box plots** comparing whole distributions on one scale, each box with its own note

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

# Section headings (a tile type of their own) group the dashboard and show up as
# headers in the contents.
"$MUCKDB" session section "$SESSION" --name sec-sales --title "Sales" >/dev/null

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

"$MUCKDB" session section "$SESSION" --name sec-time --title "Time series" >/dev/null

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

"$MUCKDB" session tile "$SESSION" --name cumulative --title "Cumulative revenue" \
  --db "$DB" --view revenue_cumulative --chart area --x day --y cumulative_revenue --trend \
  --xlabel "Day" --ylabel "Cumulative revenue" \
  --caption "Area chart — a running revenue total over the month; the trendline smooths its climb." >/dev/null

"$MUCKDB" session tile "$SESSION" --name scatter --title "Each event over time" \
  --db "$DB" --view events_points --chart scatter --x ts --y value \
  --caption "Every event as a point — clusters show where activity bunched up." >/dev/null

"$MUCKDB" session section "$SESSION" --name sec-dist --title "Distributions" >/dev/null

"$MUCKDB" session tile "$SESSION" --name spread --title "Order value spread by category" \
  --db "$DB" --view amount_spread --chart box --x category --y lo,q1,med,q3,hi --desc note \
  --caption "Box-and-whisker per category on one shared scale — the box is the middle 50%, the notch the median; whiskers reach min/max." >/dev/null

"$MUCKDB" session tile "$SESSION" --name heat --title "Activity by weekday × hour" \
  --db "$DB" --view events_heat --chart heatmap --x hour --y weekday --value events \
  --caption "A heatmap crosses two categoricals and shades cells by a value — the midday hump shows as a bright column, quieter days as dim rows." >/dev/null

"$MUCKDB" session section "$SESSION" --name sec-geo --title "Geography" >/dev/null

"$MUCKDB" session tile "$SESSION" --name map --title "Where our customers are" \
  --db "$DB" --view customer_map --chart map --lat latitude --lon longitude --label city \
  --caption "A map tile plots lat/long points on an ASCII world map; brighter cells hold more customers. Hover a marker for its city." >/dev/null

"$MUCKDB" session tile "$SESSION" --name flows --title "Backbone flows between cities" \
  --db "$DB" --view network_flows --chart map \
  --from-lat from_lat --from-lon from_lon --to-lat to_lat --to-lon to_lon \
  --from-label from_city --to-label to_city \
  --label label --value gbps \
  --caption "A connections map: each row links two cities as a fluid semi-transparent arc — drawn over the ASCII backdrop or the hi-fi world map (whose sea gently shimmers), taking the shorter way round the globe when that wraps the date line, with opacity scaling with capacity. Arc labels sit on a top layer and shift to avoid overlapping; hover an arc for its route, or a city marker for its name (--from-label/--to-label)." >/dev/null

"$MUCKDB" session section "$SESSION" --name sec-timeline --title "Timelines" >/dev/null

"$MUCKDB" session tile "$SESSION" --name pipeline --title "CI/CD pipeline (relative seconds)" \
  --db "$DB" --view pipeline_timeline --chart timeline \
  --lane resource --label step --start t0 --end t1 \
  --color outcome --id sid --depends-on parent \
  --event '95|tests start' \
  --caption "A Gantt-style pipeline on a 0→seconds axis: lanes are runners, bars are steps; lint overlaps unit tests so it stacks into a sublane; right-angle connectors show the dependency chain and colour encodes the step outcome." >/dev/null

"$MUCKDB" session tile "$SESSION" --name incident --title "Incident timeline (absolute time)" \
  --db "$DB" --view incident_timeline --chart timeline \
  --lane system --label phase --start started --end ended \
  --color severity \
  --event '2026-05-01 14:18|outage declared' --event '2026-05-01 14:41|resolved' \
  --caption "The same tile on an absolute time axis, shown in your local zone (a --tz local format on the column; the db stores UTC). Each system's phases over the incident, coloured by severity, with dashed markers for when the outage was declared and resolved. Hover the plot for the time (local + UTC); hover any bar for its window, details, and a clickable ticket link." >/dev/null

"$MUCKDB" session section "$SESSION" --name sec-sequence --title "Sequences" >/dev/null

"$MUCKDB" session tile "$SESSION" --name checkout --title "Checkout flow across services" \
  --db "$DB" --view service_calls --chart sequence \
  --from caller --to callee --label message --message-type msg_type \
  --from-type caller_type --to-type callee_type --group grp --group-branch branch --autonumber \
  --caption "A sequence diagram of microservice comms: participants are typed (user is an actor, auth a boundary, db a database, the rest plain services), messages are sync/reply/async/lost arrows, and the alt frame splits the session-valid path from the expired one — with a self-message and a lost webhook at the end. Autonumbered; hover a message for its details and a clickable trace link. The 'mermaid' button copies a mermaid.js sequenceDiagram to the clipboard." >/dev/null

# A closing summary panel — the takeaways, so the dashboard reads top-to-bottom.
"$MUCKDB" session post "$SESSION" --name summary --title "Summary" --md "## Summary

This dashboard tours every muckdb panel type from one shell script:

| What                | Panel                          | Why it's shaped this way                 |
|:--------------------|:-------------------------------|:-----------------------------------------|
| **Revenue / types** | solid bars                     | categorical buckets → one colour each    |
| **Orders per day**  | gradient bars on a UTC axis    | continuous over time → gradient          |
| **Sensors**         | line + event & target markers  | a clean series with annotations          |
| **Cumulative rev.** | area + trendline               | a running total over time                 |
| **Top products**    | table (fills, no scroll)       | small result set read in full            |
| **Each event**      | scatter                        | raw points show where activity bunched   |
| **Activity heat**   | heatmap (weekday × hour)       | two categoricals × a value → density at a glance |
| **Value spread**    | box plots on a shared scale    | compare whole distributions, each with a note |
| **Customers**       | map (lat/long → world map)     | geographic points, brighter = denser      |
| **CI pipeline**     | timeline (relative seconds)    | tasks over time, sublanes for overlap, dep arrows |
| **Incident**        | timeline (absolute time)       | phases per system, severity colour, event markers |
| **Checkout flow**   | sequence diagram               | service comms — typed participants, arrow kinds, an alt frame |

Section headers (**Sales**, **Time series**, **Distributions**, **Geography**, **Timelines**, **Sequences**)
group the panels and appear in the contents.

Every figure here is backed by a **view** you can open (hit **explore**),
re-sort, filter, and export — nothing is take-my-word-for-it." >/dev/null

PORT="${MUCKDB_PORT:-11000}"
echo
echo "Done. Open:  http://localhost:${PORT}/session/${SESSION}/"
echo "Explore the data:  http://localhost:${PORT}/  (Databases tab → $(basename "$DB"))"
