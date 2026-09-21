-- Illustrative private addressing; not a discovered production network.
CREATE OR REPLACE TABLE mesh_routers AS
SELECT * FROM (VALUES
  ('start', 'Start test router', 'DC 1', '10.10.0.0/24'),
  ('A', 'Router A', 'DC 1', '10.10.1.0/24'),
  ('B', 'Router B', 'DC 1', '10.10.2.0/24'),
  ('C', 'Router C', 'DC 2', '10.20.1.0/24'),
  ('D', 'Router D', 'DC 2', '10.20.2.0/24'),
  ('end', 'End test router', 'DC 2', '10.20.3.0/24')
) t(id, name, dc, lan);

CREATE OR REPLACE TABLE mesh_links AS
SELECT * FROM (VALUES
  ('start', 'A', '172.16.0.0/30', '172.16.0.1', '172.16.0.2', 'Test access'),
  ('A', 'B', '10.255.0.0/30', '10.255.0.1', '10.255.0.2', 'Mesh'),
  ('A', 'C', '10.255.0.4/30', '10.255.0.5', '10.255.0.6', 'Mesh'),
  ('A', 'D', '10.255.0.8/30', '10.255.0.9', '10.255.0.10', 'Mesh'),
  ('B', 'C', '10.255.0.12/30', '10.255.0.13', '10.255.0.14', 'Mesh'),
  ('B', 'D', '10.255.0.16/30', '10.255.0.17', '10.255.0.18', 'Mesh'),
  ('C', 'D', '10.255.0.20/30', '10.255.0.21', '10.255.0.22', 'Mesh'),
  ('D', 'end', '172.16.0.4/30', '172.16.0.5', '172.16.0.6', 'Test access')
) t(src, dst, subnet, src_ip, dst_ip, link_class);

CREATE OR REPLACE VIEW router_mesh AS
SELECT a.dc AS src_dc, b.dc AS dst_dc, l.link_class,
  a.name AS src_name, b.name AS dst_name, l.subnet,
  a.lan AS src_lan, b.lan AS dst_lan,
  'router' AS src_type, 'router' AS dst_type,
  l.src_ip, l.dst_ip, l.src, l.dst
FROM mesh_links l
JOIN mesh_routers a ON a.id = l.src
JOIN mesh_routers b ON b.id = l.dst
ORDER BY CASE l.src WHEN 'start' THEN 0 WHEN 'A' THEN 1
  WHEN 'B' THEN 2 WHEN 'C' THEN 3 ELSE 4 END, l.dst;
