CREATE OR REPLACE TABLE label_fanout_edges AS
SELECT 'Core switch' src, 'Port ' || i dst, 'et-0/0/' || i src_port,
       'uplink' dst_port, '100G' link_label, 'Backbone' link_class
FROM range(1, 7) t(i)
UNION ALL
SELECT 'Port ' || i, 'MVE ' || (1 + (i - 1) % 3), 'access',
       'ge-0/0/' || (1 + (i - 1) // 3), '10G', 'Access'
FROM range(1, 7) t(i);
CREATE OR REPLACE VIEW label_fanout AS SELECT * FROM label_fanout_edges;
CREATE OR REPLACE VIEW long_label_fanout AS
SELECT CASE WHEN src = 'Core switch' THEN 'Brisbane enterprise backbone aggregation switch'
            ELSE 'Customer interconnect access port ' || split_part(src, ' ', 2) END src,
       CASE WHEN dst LIKE 'Port %' THEN 'Customer interconnect access port ' || split_part(dst, ' ', 2)
            ELSE 'Production virtual routing gateway ' || split_part(dst, ' ', 2) END dst,
       CASE WHEN link_class = 'Backbone' THEN 'et-0/0/' || split_part(dst, ' ', 2) || ' · backbone handoff'
            ELSE 'xe-0/0/1 · customer-facing access' END src_port,
       CASE WHEN link_class = 'Backbone' THEN 'uplink · primary transport interface'
            ELSE dst_port || ' · production routing interface' END dst_port,
       CASE WHEN link_class = 'Backbone' THEN '100 Gbps · dedicated enterprise backbone'
            ELSE '10 Gbps · private production connectivity' END link_label,
       link_class
FROM label_fanout_edges;
CREATE OR REPLACE VIEW long_label_triangle AS
SELECT * FROM (VALUES
 ('Enterprise ingress aggregation router', 'Primary application delivery service', 'ae-100 · enterprise ingress', 'eth0 · northbound traffic', 'Encrypted production application traffic', 'Traffic'),
 ('Enterprise ingress aggregation router', 'Secondary application delivery service', 'ae-101 · redundant ingress', 'eth0 · northbound traffic', 'Redundant application traffic path', 'Traffic'),
 ('Primary application delivery service', 'Secondary application delivery service', 'eth1 · peer synchronization', 'eth1 · peer synchronization', 'Session replication and health synchronization', 'mesh'),
 ('Primary application delivery service', 'Shared production transaction database', 'eth2 · database connection pool', 'db0 · primary listener', 'TLS-protected database queries and transactions', 'Traffic'),
 ('Secondary application delivery service', 'Shared production transaction database', 'eth2 · database connection pool', 'db1 · redundant listener', 'TLS-protected standby queries and transactions', 'Traffic')
) t(src,dst,src_port,dst_port,link_label,link_class);
CREATE OR REPLACE VIEW long_label_regions AS
SELECT * FROM (VALUES
 ('Brisbane primary interconnect gateway', 'Sydney disaster recovery interconnect gateway', 'Brisbane production network', 'Sydney disaster recovery network', 'ae-200 · interstate backbone', 'ae-210 · interstate termination', '100 Gbps · encrypted interstate replication', 'Replication'),
 ('Brisbane primary interconnect gateway', 'Melbourne analytics interconnect gateway', 'Brisbane production network', 'Melbourne analytics network', 'ae-201 · analytics transport', 'ae-310 · warehouse ingestion', '40 Gbps · continuous analytics ingestion', 'Analytics'),
 ('Sydney disaster recovery interconnect gateway', 'Melbourne analytics interconnect gateway', 'Sydney disaster recovery network', 'Melbourne analytics network', 'ae-211 · recovery transport', 'ae-311 · alternate ingestion', '40 Gbps · alternate ingestion during failover', 'Recovery')
) t(src,dst,src_zone,dst_zone,src_port,dst_port,link_label,link_class);
