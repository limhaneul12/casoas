-- name: pragma_readonly_defense
PRAGMA trusted_schema=OFF;
PRAGMA query_only=ON;

-- name: select_neighbors_by_node_id
SELECT 'caller', name FROM (SELECT DISTINCT n.name FROM edges e JOIN nodes n ON e.source = n.id WHERE e.target = ? ORDER BY n.name ASC LIMIT 8) UNION ALL SELECT 'callee', name FROM (SELECT DISTINCT n.name FROM edges e JOIN nodes n ON e.target = n.id WHERE e.source = ? ORDER BY n.name ASC LIMIT 8)

-- name: select_nodes_exact
SELECT id, kind, name, qualified_name, file_path, start_line, end_line FROM nodes WHERE name = ? OR qualified_name = ? ORDER BY file_path ASC, start_line ASC, id ASC LIMIT 3

-- name: select_nodes_suffix_like
SELECT id, kind, name, qualified_name, file_path, start_line, end_line FROM nodes WHERE qualified_name LIKE ? ESCAPE '\' ORDER BY file_path ASC, start_line ASC, id ASC LIMIT 3

-- name: select_nodes_by_file
SELECT id, kind, name, qualified_name, file_path, start_line, end_line FROM nodes WHERE file_path = ? AND start_line > 0 AND end_line >= start_line AND kind NOT IN ('file', 'import') ORDER BY start_line ASC, id ASC LIMIT 128

-- name: select_nodes_by_file_range
SELECT id, kind, name, qualified_name, file_path, start_line, end_line FROM nodes WHERE file_path = ? AND start_line <= ? AND end_line >= ? AND kind NOT IN ('file', 'import') ORDER BY (end_line - start_line) ASC, start_line ASC, id ASC LIMIT 12
