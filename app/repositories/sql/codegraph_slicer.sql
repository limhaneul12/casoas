-- Read-only SQL queries for CodegraphContextSlicer (`codegraph_slicer.sql`)
-- All runtime values MUST be bound via `?` positional parameters.

-- name: pragma_readonly_defense
PRAGMA trusted_schema=OFF;
PRAGMA query_only=ON;

-- name: select_neighbors_by_node_id
SELECT 'caller', name FROM (
    SELECT DISTINCT n.name
    FROM edges e
    JOIN nodes n ON e.source = n.id
    WHERE e.target = ?
    LIMIT 8
)
UNION ALL
SELECT 'callee', name FROM (
    SELECT DISTINCT n.name
    FROM edges e
    JOIN nodes n ON e.target = n.id
    WHERE e.source = ?
    LIMIT 8
)

-- name: select_nodes_exact
SELECT id, kind, name, qualified_name, file_path, start_line, end_line
FROM nodes
WHERE name = ? OR qualified_name = ?
ORDER BY start_line ASC
LIMIT 3

-- name: select_nodes_suffix_like
SELECT id, kind, name, qualified_name, file_path, start_line, end_line
FROM nodes
WHERE qualified_name LIKE ? ESCAPE '\'
ORDER BY start_line ASC
LIMIT 3

-- name: select_nodes_by_diff_hunk
SELECT DISTINCT name, (end_line - start_line) AS span, start_line
FROM nodes
WHERE file_path = ?
  AND kind IN ('class', 'function', 'method')
  AND start_line <= ?
  AND end_line >= ?
ORDER BY span ASC, start_line ASC
LIMIT 4

-- name: select_nodes_by_files_json
SELECT name FROM (
    SELECT
        file_path,
        name,
        start_line,
        ROW_NUMBER() OVER (PARTITION BY file_path ORDER BY start_line ASC) AS rn
    FROM (
        SELECT DISTINCT file_path, name, start_line
        FROM nodes
        WHERE file_path IN (SELECT value FROM json_each(?))
          AND kind IN ('class', 'function', 'method')
    )
)
WHERE rn <= 4
