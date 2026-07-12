import { execFileSync } from 'node:child_process';
import { PORT } from '../constants';

// Run one `muckdb` command with the isolated env; throws on failure.
function run(binary: string, env: NodeJS.ProcessEnv, args: string[]): void {
  execFileSync(binary, ['--port', String(PORT), ...args], {
    env,
    stdio: 'pipe',
  });
}

const CREATE_SQL = `
CREATE TABLE widgets AS
SELECT i AS id,
       (['Alpha','Beta','Gamma','Delta','Epsilon'])[(i % 5) + 1] AS category,
       (['US','EU','APAC'])[(i % 3) + 1]                        AS region,
       round((i * 7) % 100 + 0.5, 2)                            AS price,
       TIMESTAMP '2026-01-01 00:00:00' + (i * INTERVAL 6 HOUR)  AS created,
       -- an array column with a unit format; every 4th row is empty, to cover
       -- empty-list rendering (must show "—", not a bare unit suffix).
       CASE WHEN i % 4 = 0 THEN CAST([] AS INTEGER[]) ELSE [10, 100] END AS sizes,
       -- lat/long jittered around three cities, for a map tile.
       round(([-33.87, 51.51, 40.71])[(i % 3) + 1] + ((i % 7) - 3) * 0.4, 4)   AS latitude,
       round(([151.21, -0.13, -74.01])[(i % 3) + 1] + ((i % 5) - 2) * 0.4, 4)  AS longitude
FROM range(200) t(i);
CREATE VIEW widgets_all AS SELECT * FROM widgets;
CREATE VIEW by_category AS SELECT category, count(*) AS n FROM widgets GROUP BY 1 ORDER BY n DESC;
CREATE VIEW by_day AS SELECT created::DATE AS day, count(*) AS n FROM widgets GROUP BY 1 ORDER BY 1;
CREATE VIEW widget_map AS SELECT id, category, latitude, longitude FROM widgets;
-- A connections/flows view: each row is an arc between two fixed cities, for
-- the map tile's connection rendering (arcs + labels).
CREATE VIEW widget_flows AS SELECT * FROM (VALUES
  (-33.87, 151.21, 51.51, -0.13,  'Sydney',   'London',   'Sydney → London',   120),
  (51.51,  -0.13,  40.71, -74.01, 'London',   'New York', 'London → New York', 200),
  (40.71,  -74.01, -33.87, 151.21,'New York', 'Sydney',   'New York → Sydney', 90)
) f(from_lat, from_lon, to_lat, to_lon, from_city, to_city, label, gbps);
-- Timeline (Gantt) fixture: a small deploy pipeline on a relative-seconds axis
-- with two lanes, an overlap (→ sublane), a colour category, and a dependency.
CREATE VIEW deploy_timeline AS SELECT * FROM (VALUES
  ('build',  'compile',   0.0,  40.0, 'ok',     's1', NULL),
  ('build',  'lint',      5.0,  30.0, 'ok',     's2', NULL),   -- overlaps compile → sublane
  ('deploy', 'push',     40.0,  70.0, 'ok',     's3', 's1'),
  ('deploy', 'migrate',  70.0,  95.0, 'failed', 's4', 's3')
) t(lane, task, t0, t1, status, sid, parent);

-- An absolute-time timeline (UTC in the db); a --tz local format shifts its axis
-- to local time, and the hover readout then also shows the UTC instant.
CREATE VIEW ts_timeline AS SELECT * FROM (VALUES
  ('api', 'outage',   TIMESTAMP '2026-05-01 14:00:00', TIMESTAMP '2026-05-01 14:30:00'),
  ('db',  'failover', TIMESTAMP '2026-05-01 14:10:00', TIMESTAMP '2026-05-01 14:25:00')
) t(sys, phase, started, ended);

-- Sequence diagram fixture: one row per message, each participant type, all four
-- arrow kinds, a self-message, and an alt/else group. 'trace' carries a --link
-- format (tooltip-link coverage); 'note' carries a hostile value (XSS coverage).
CREATE VIEW messages AS SELECT * FROM (VALUES
  (1,'user','gateway','GET /orders','sync','actor','participant',NULL,NULL,'t-1','<img src=x onerror=alert(1)>'),
  (2,'gateway','auth','verify','sync','participant','boundary','alt:token valid','valid','t-2','ok'),
  (3,'auth','db','SELECT session','sync','boundary','database','alt:token valid','valid','t-3','ok'),
  (4,'gateway','user','401','reply','participant','actor','alt:token valid','expired','t-4','denied'),
  (5,'orders','orders','retry','async','participant','participant',NULL,NULL,'t-5','backoff'),
  (6,'gateway','cache','ping','lost','participant','participant',NULL,NULL,'t-6','timeout')
) t(seq,src,dst,msg,mtype,st,dt,grp,branch,trace,note)
ORDER BY seq;

-- A loop-group fixture whose --group-branch CHANGES within the frame — regression
-- coverage for the seqToMermaid bug where a loop/opt frame incorrectly emitted an
-- alt/par-only 'else'/'and' compartment line on a branch change (invalid mermaid).
CREATE VIEW msgs_loop AS SELECT * FROM (VALUES
  (1,'client','server','attempt','loop:retry','try1'),
  (2,'client','server','attempt','loop:retry','try2'),
  (3,'client','server','attempt','loop:retry','try3')
) t(seq,src,dst,msg,grp,branch)
ORDER BY seq;
`;

// Build the seed database + session. `dbPath` must live under the run's temp dir.
export function seed(env: NodeJS.ProcessEnv, binary: string, dbPath: string): void {
  // 1. Build the database (this also registers it in the ledger so the daemon can browse it).
  run(binary, env, [dbPath, '-c', CREATE_SQL]);

  // Unit format on the array column, so empty vs non-empty rendering is exercised.
  run(binary, env, ['format', dbPath, 'sizes', '--suffix', ' Gbps', '--thousands']);

  // A link + link_title on the timeline's `sid` column, scoped to deploy_timeline,
  // with a deliberately hostile link_title — regression coverage for the tlTip()
  // XSS fix (the tooltip must render this as escaped text, never as a live <img>).
  run(binary, env, [
    'format', dbPath, 'sid', '--table', 'deploy_timeline',
    '--link', 'https://example.test/{value}',
    '--link-title', '<img src=x onerror=alert(1)>',
  ]);

  // 2. Build the dashboard session.
  run(binary, env, ['session', 'create', 'e2e', '--title', 'E2E fixtures']);
  run(binary, env, ['session', 'post', 'e2e', '--name', 'summary', '--title', 'Summary',
    '--md', '# E2E\n\n**200 widgets**, 5 categories.']);
  run(binary, env, ['session', 'tile', 'e2e', '--name', 'by-cat', '--title', 'By category',
    '--db', dbPath, '--view', 'by_category', '--chart', 'bar', '--x', 'category', '--y', 'n',
    '--caption', 'Widgets per category (deterministic: 40 each).']);
  run(binary, env, ['session', 'tile', 'e2e', '--name', 'by-day', '--title', 'By day',
    '--db', dbPath, '--view', 'by_day', '--chart', 'line', '--x', 'day', '--y', 'n',
    '--caption', 'Widgets created per day.']);
  run(binary, env, ['session', 'tile', 'e2e', '--name', 'map', '--title', 'Widget map',
    '--db', dbPath, '--view', 'widget_map', '--chart', 'map',
    '--lat', 'latitude', '--lon', 'longitude', '--label', 'category',
    '--caption', 'Widgets by lat/long — hover a marker for its category.']);
  run(binary, env, ['session', 'tile', 'e2e', '--name', 'flows', '--title', 'Flows',
    '--db', dbPath, '--view', 'widget_flows', '--chart', 'map',
    '--from-lat', 'from_lat', '--from-lon', 'from_lon', '--to-lat', 'to_lat', '--to-lon', 'to_lon',
    '--from-label', 'from_city', '--to-label', 'to_city',
    '--label', 'label', '--value', 'gbps',
    '--caption', 'Connections drawn as arcs between city pairs.']);
  run(binary, env, ['session', 'tile', 'e2e', '--name', 'timeline', '--title', 'Deploy timeline',
    '--db', dbPath, '--view', 'deploy_timeline', '--chart', 'timeline',
    '--lane', 'lane', '--label', 'task', '--start', 't0', '--end', 't1',
    '--color', 'status', '--id', 'sid', '--depends-on', 'parent',
    '--event', '50|cutover',
    '--caption', 'A Gantt-style timeline: lanes stack overlapping bars into sublanes; colour = status.']);
  // An absolute-time timeline in local zone — regression coverage for the UTC
  // hover readout and the tz-aware axis.
  run(binary, env, ['format', dbPath, 'started', '--table', 'ts_timeline', '--tz', 'local']);
  run(binary, env, ['session', 'tile', 'e2e', '--name', 'timeline-ts', '--title', 'Incident (local tz)',
    '--db', dbPath, '--view', 'ts_timeline', '--chart', 'timeline',
    '--lane', 'sys', '--label', 'phase', '--start', 'started', '--end', 'ended',
    '--event', '2026-05-01 14:15|escalated',
    '--caption', 'Absolute-time timeline shown in local zone; hover shows UTC too.']);
  run(binary, env, ['session', 'tile', 'e2e', '--name', 'all', '--title', 'All widgets',
    '--db', dbPath, '--view', 'widgets_all', '--chart', 'table',
    '--caption', 'The full flattened list.']);

  // A --link format on the sequence fixture's `trace` column, scoped to `messages`
  // — tooltip-link coverage. `note` (seeded above with a hostile value) has no
  // format, so it renders as plain escaped text (XSS coverage).
  run(binary, env, [
    'format', dbPath, 'trace', '--table', 'messages',
    '--link', 'https://trace.example.test/{value}',
  ]);
  run(binary, env, ['session', 'tile', 'e2e', '--name', 'sequence', '--title', 'Service comms',
    '--db', dbPath, '--view', 'messages', '--chart', 'sequence',
    '--from', 'src', '--to', 'dst', '--label', 'msg', '--message-type', 'mtype',
    '--from-type', 'st', '--to-type', 'dt', '--group', 'grp', '--group-branch', 'branch',
    '--autonumber',
    '--caption', 'A sequence diagram: participant types, arrow kinds, a self-message, an alt group.']);

  run(binary, env, ['session', 'tile', 'e2e', '--name', 'sequence-loop', '--title', 'Retry loop',
    '--db', dbPath, '--view', 'msgs_loop', '--chart', 'sequence',
    '--from', 'src', '--to', 'dst', '--label', 'msg', '--group', 'grp', '--group-branch', 'branch',
    '--caption', 'A loop frame whose group-branch changes mid-frame — must export valid mermaid (no else/and).']);
}
