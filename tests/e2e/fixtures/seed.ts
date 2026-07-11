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
`;

// Build the seed database + session. `dbPath` must live under the run's temp dir.
export function seed(env: NodeJS.ProcessEnv, binary: string, dbPath: string): void {
  // 1. Build the database (this also registers it in the ledger so the daemon can browse it).
  run(binary, env, [dbPath, '-c', CREATE_SQL]);

  // Unit format on the array column, so empty vs non-empty rendering is exercised.
  run(binary, env, ['format', dbPath, 'sizes', '--suffix', ' Gbps', '--thousands']);

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
  run(binary, env, ['session', 'tile', 'e2e', '--name', 'all', '--title', 'All widgets',
    '--db', dbPath, '--view', 'widgets_all', '--chart', 'table',
    '--caption', 'The full flattened list.']);
}
