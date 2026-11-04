use std::collections::BTreeMap;
use std::io::{Error as IoError, ErrorKind};
use std::path::Path;

use rusqlite::{Connection, OpenFlags, Params, params, types::ValueRef};
use url::Url;

const JOB_SQL: &str = include_str!("../../sql/job_store.sql");
const HANDOFF_SQL: &str = include_str!("../../sql/handoff_store.sql");
const CODEGRAPH_SQL: &str = include_str!("../../sql/codegraph_slicer.sql");

#[derive(Clone, Debug)]
struct Query {
    text: String,
    parameters: usize,
}

#[derive(Clone, Debug)]
pub struct SqlCatalog {
    queries: BTreeMap<String, Query>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodegraphNode {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub qualified_name: String,
    pub file_path: String,
    pub start_line: i64,
    pub end_line: i64,
    pub callers: Vec<String>,
    pub callees: Vec<String>,
}

pub struct CodegraphReader {
    connection: Connection,
    catalog: SqlCatalog,
}

impl CodegraphReader {
    pub fn open(db_path: &Path) -> crate::Result<Self> {
        Ok(Self {
            connection: open_readonly_query_only(db_path)?,
            catalog: SqlCatalog::codegraph()?,
        })
    }

    pub fn find_symbol(&self, symbol: &str) -> crate::Result<Vec<CodegraphNode>> {
        validate_text(symbol, "CodeGraph symbol")?;
        let exact = self.catalog.query("select_nodes_exact", 2)?;
        let mut nodes = read_nodes(&self.connection, exact, [symbol, symbol])?;
        if nodes.is_empty() {
            let pattern = escape_like_suffix(symbol)?;
            let suffix = self.catalog.query("select_nodes_suffix_like", 1)?;
            nodes = read_nodes(&self.connection, suffix, [&pattern])?;
        }
        self.attach_neighbors(&mut nodes)?;
        Ok(nodes)
    }

    pub fn find_file_symbols(
        &self,
        file: &str,
        lines: Option<(usize, usize)>,
    ) -> crate::Result<Vec<CodegraphNode>> {
        validate_text(file, "CodeGraph file")?;
        let mut nodes = if let Some((start, end)) = lines {
            if start == 0 || end < start {
                return Err(invalid_input("invalid CodeGraph line range"));
            }
            let query = self.catalog.query("select_nodes_by_file_range", 3)?;
            read_nodes(
                &self.connection,
                query,
                params![file, i64::try_from(end)?, i64::try_from(start)?],
            )?
        } else {
            let query = self.catalog.query("select_nodes_by_file", 1)?;
            read_nodes(&self.connection, query, [file])?
        };
        self.attach_neighbors(&mut nodes)?;
        Ok(nodes)
    }

    fn attach_neighbors(&self, nodes: &mut [CodegraphNode]) -> crate::Result<()> {
        let neighbors = self.catalog.query("select_neighbors_by_node_id", 2)?;
        for node in nodes {
            let mut statement = self.connection.prepare(neighbors)?;
            let rows = statement.query_map([node.id.as_str(), node.id.as_str()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (direction, name) = row?;
                match direction.as_str() {
                    "caller" => node.callers.push(name),
                    "callee" => node.callees.push(name),
                    _ => {}
                }
            }
        }
        Ok(())
    }
}

impl SqlCatalog {
    pub fn jobs() -> crate::Result<Self> {
        Self::parse(JOB_SQL)
    }

    pub fn handoffs() -> crate::Result<Self> {
        Self::parse(HANDOFF_SQL)
    }

    pub fn codegraph() -> crate::Result<Self> {
        Self::parse(CODEGRAPH_SQL)
    }

    pub fn parse(source: &str) -> crate::Result<Self> {
        let mut queries = BTreeMap::new();
        let mut current_name: Option<String> = None;
        let mut lines = Vec::new();
        for line in source.lines() {
            if let Some(name) = line.trim().strip_prefix("-- name:") {
                if let Some(previous) = current_name.take() {
                    insert_query(&mut queries, &previous, &lines.join("\n"))?;
                }
                current_name = Some(validate_query_name(name.trim())?);
                lines.clear();
            } else if current_name.is_some() && !line.trim().starts_with("--") {
                lines.push(line);
            }
        }
        if let Some(previous) = current_name {
            insert_query(&mut queries, &previous, &lines.join("\n"))?;
        }
        if queries.is_empty() {
            return Err(invalid_data("SQL catalog contains no named queries"));
        }
        Ok(Self { queries })
    }

    pub fn query(&self, name: &str, parameter_count: usize) -> crate::Result<&str> {
        let query = self
            .queries
            .get(name)
            .ok_or_else(|| invalid_input(format!("unknown SQL query '{name}'")))?;
        if query.parameters != parameter_count {
            return Err(invalid_input(format!(
                "SQL parameter arity mismatch for '{name}': expected {}, got {parameter_count}",
                query.parameters
            )));
        }
        Ok(&query.text)
    }

    pub fn script(&self, name: &str) -> crate::Result<Vec<&str>> {
        if !matches!(
            name,
            "init_schema" | "pragma_connect" | "pragma_readonly_defense"
        ) {
            return Err(invalid_input(format!(
                "SQL script '{name}' is not allowlisted"
            )));
        }
        let query = self
            .queries
            .get(name)
            .ok_or_else(|| invalid_input(format!("unknown SQL script '{name}'")))?;
        if query.parameters != 0 {
            return Err(invalid_data(format!(
                "SQL script '{name}' contains parameters"
            )));
        }
        Ok(query
            .text
            .split(';')
            .map(str::trim)
            .filter(|statement| !statement.is_empty())
            .collect())
    }
}

fn validate_query_name(name: &str) -> crate::Result<String> {
    let valid = !name.is_empty()
        && name.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase() || byte == b'_' || (index > 0 && byte.is_ascii_digit())
        });
    if !valid {
        return Err(invalid_data(format!("invalid SQL query name '{name}'")));
    }
    Ok(name.to_owned())
}

fn insert_query(
    queries: &mut BTreeMap<String, Query>,
    name: &str,
    untrimmed: &str,
) -> crate::Result<()> {
    let text = untrimmed.trim();
    if text.is_empty() {
        return Err(invalid_data(format!("empty SQL query '{name}'")));
    }
    let sanitized = strip_string_literals(text);
    let upper = sanitized.to_ascii_uppercase();
    if upper.contains("/*") || upper.contains("*/") || upper.contains("--") {
        return Err(invalid_data(format!(
            "embedded SQL comments are forbidden in '{name}'"
        )));
    }
    let has_forbidden_token = upper
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .any(|token| {
            matches!(
                token,
                "ATTACH" | "DETACH" | "WRITABLE_SCHEMA" | "LOAD_EXTENSION"
            )
        });
    if has_forbidden_token {
        return Err(invalid_data(format!("forbidden SQL command in '{name}'")));
    }
    let is_script = matches!(
        name,
        "init_schema" | "pragma_connect" | "pragma_readonly_defense"
    );
    if !is_script && sanitized.trim_end_matches(';').contains(';') {
        return Err(invalid_data(format!("stacked SQL statements in '{name}'")));
    }
    if queries
        .insert(
            name.to_owned(),
            Query {
                text: text.trim_end_matches(';').to_owned(),
                parameters: sanitized.bytes().filter(|byte| *byte == b'?').count(),
            },
        )
        .is_some()
    {
        return Err(invalid_data(format!("duplicate SQL query '{name}'")));
    }
    Ok(())
}

fn strip_string_literals(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_literal = false;
    while let Some(character) = chars.next() {
        if character == '\'' {
            if in_literal && chars.peek() == Some(&'\'') {
                result.push(' ');
                result.push(' ');
                let _ = chars.next();
                continue;
            }
            in_literal = !in_literal;
            result.push(' ');
        } else if in_literal {
            result.push(' ');
        } else {
            result.push(character);
        }
    }
    result
}

fn read_nodes<P: Params>(
    connection: &Connection,
    query: &str,
    parameters: P,
) -> crate::Result<Vec<CodegraphNode>> {
    let mut statement = connection.prepare(query)?;
    let rows = statement.query_map(parameters, |row| {
        Ok(CodegraphNode {
            id: sqlite_identifier(row.get_ref(0)?)?,
            kind: row.get(1)?,
            name: row.get(2)?,
            qualified_name: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
            file_path: row.get(4)?,
            start_line: row.get(5)?,
            end_line: row.get(6)?,
            callers: Vec::new(),
            callees: Vec::new(),
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn sqlite_identifier(value: ValueRef<'_>) -> rusqlite::Result<String> {
    match value {
        ValueRef::Text(bytes) => String::from_utf8(bytes.to_vec()).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                bytes.len(),
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        }),
        ValueRef::Integer(value) => Ok(value.to_string()),
        other => Err(rusqlite::Error::InvalidColumnType(
            0,
            "id".to_owned(),
            other.data_type(),
        )),
    }
}

pub fn validate_text(value: &str, context: &str) -> crate::Result<()> {
    if value.contains('\0') {
        return Err(invalid_input(format!("NUL byte blocked in {context}")));
    }
    Ok(())
}

pub fn escape_like_suffix(symbol_name: &str) -> crate::Result<String> {
    validate_text(symbol_name, "LIKE pattern")?;
    let escaped = symbol_name
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    Ok(format!("%.{escaped}"))
}

pub fn readonly_sqlite_uri(db_path: &Path) -> crate::Result<String> {
    let resolved = db_path.canonicalize()?;
    let mut url = Url::from_file_path(&resolved)
        .map_err(|()| invalid_input("database path cannot be represented as a file URI"))?;
    url.set_query(Some("mode=ro&immutable=1"));
    Ok(url.to_string())
}

pub fn open_readonly_query_only(db_path: &Path) -> crate::Result<Connection> {
    let uri = readonly_sqlite_uri(db_path)?;
    let connection = Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let catalog = SqlCatalog::codegraph()?;
    for statement in catalog.script("pragma_readonly_defense")? {
        connection.execute_batch(statement)?;
    }
    Ok(connection)
}

fn invalid_data(message: impl Into<String>) -> crate::Error {
    IoError::new(ErrorKind::InvalidData, message.into()).into()
}

fn invalid_input(message: impl Into<String>) -> crate::Error {
    IoError::new(ErrorKind::InvalidInput, message.into()).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn rejects_stacked_and_dangerous_sql() {
        assert!(SqlCatalog::parse("-- name: bad\nSELECT 1; DELETE FROM jobs").is_err());
        assert!(SqlCatalog::parse("-- name: bad\nATTACH DATABASE ? AS evil").is_err());
        assert!(SqlCatalog::parse("-- name: bad\nATTACH/*hidden*/DATABASE ? AS evil").is_err());
        assert!(SqlCatalog::parse("-- name: bad\nPRAGMA writable_schema=ON").is_err());
        assert!(SqlCatalog::parse("-- name: bad\nSELECT load_extension(?)").is_err());
        assert!(SqlCatalog::parse("-- name: bad\nSELECT 1; -- hidden\nDELETE FROM jobs").is_err());
    }

    #[test]
    fn preserves_literal_semicolons_and_escapes_like() -> crate::Result<()> {
        let catalog = SqlCatalog::parse("-- name: safe\nSELECT ';' WHERE name = ?")?;
        assert_eq!(catalog.query("safe", 1)?, "SELECT ';' WHERE name = ?");
        assert_eq!(escape_like_suffix("a_%\\")?, "%.a\\_\\%\\\\");
        assert!(escape_like_suffix("bad\0value").is_err());
        Ok(())
    }

    #[test]
    fn external_database_is_immutable_and_query_only() -> crate::Result<()> {
        let directory = tempdir()?;
        let path = directory.path().join("code graph #1.db");
        let writer = Connection::open(&path)?;
        writer.execute_batch(
            "CREATE TABLE nodes (id INTEGER PRIMARY KEY); INSERT INTO nodes VALUES (1)",
        )?;
        drop(writer);

        let uri = readonly_sqlite_uri(&path)?;
        assert!(uri.contains("mode=ro&immutable=1"));
        assert!(uri.contains("%20"));
        assert!(uri.contains("%23"));
        let reader = open_readonly_query_only(&path)?;
        let query_only: i64 = reader.query_row("PRAGMA query_only", [], |row| row.get(0))?;
        assert_eq!(query_only, 1);
        assert!(reader.execute("DELETE FROM nodes", []).is_err());
        Ok(())
    }

    #[test]
    fn codegraph_reader_returns_lines_and_one_hop_neighbors() -> crate::Result<()> {
        let directory = tempdir()?;
        let path = directory.path().join("codegraph.db");
        let writer = Connection::open(&path)?;
        writer.execute_batch(
            "CREATE TABLE nodes (id TEXT PRIMARY KEY, kind TEXT NOT NULL, name TEXT NOT NULL, qualified_name TEXT NOT NULL, file_path TEXT NOT NULL, language TEXT NOT NULL, start_line INTEGER NOT NULL, end_line INTEGER NOT NULL, start_column INTEGER NOT NULL, end_column INTEGER NOT NULL, docstring TEXT, signature TEXT, visibility TEXT, is_exported INTEGER DEFAULT 0, is_async INTEGER DEFAULT 0, is_static INTEGER DEFAULT 0, is_abstract INTEGER DEFAULT 0, decorators TEXT, type_parameters TEXT, return_type TEXT, updated_at INTEGER NOT NULL);\
             CREATE TABLE edges (id INTEGER PRIMARY KEY AUTOINCREMENT, source TEXT NOT NULL, target TEXT NOT NULL, kind TEXT NOT NULL, metadata TEXT, line INTEGER, col INTEGER, provenance TEXT DEFAULT NULL, FOREIGN KEY (source) REFERENCES nodes(id) ON DELETE CASCADE, FOREIGN KEY (target) REFERENCES nodes(id) ON DELETE CASCADE);\
             INSERT INTO nodes (id,kind,name,qualified_name,file_path,language,start_line,end_line,start_column,end_column,updated_at) VALUES ('function:target', 'function', 'target', 'pkg.target', 'src/lib.rs', 'rust', 10, 20, 1, 2, 1);\
             INSERT INTO nodes (id,kind,name,qualified_name,file_path,language,start_line,end_line,start_column,end_column,updated_at) VALUES ('function:caller', 'function', 'caller', 'pkg.caller', 'src/lib.rs', 'rust', 1, 8, 1, 2, 1);\
             INSERT INTO nodes (id,kind,name,qualified_name,file_path,language,start_line,end_line,start_column,end_column,updated_at) VALUES ('function:callee', 'function', 'callee', 'pkg.callee', 'src/lib.rs', 'rust', 22, 30, 1, 2, 1);\
             INSERT INTO edges (source,target,kind) VALUES ('function:caller', 'function:target', 'calls');\
             INSERT INTO edges (source,target,kind) VALUES ('function:target', 'function:callee', 'calls');",
        )?;
        drop(writer);
        let reader = CodegraphReader::open(&path)?;
        let nodes = reader.find_symbol("target")?;
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].id, "function:target");
        assert_eq!(nodes[0].start_line, 10);
        assert_eq!(nodes[0].end_line, 20);
        assert_eq!(nodes[0].callers, vec!["caller"]);
        assert_eq!(nodes[0].callees, vec!["callee"]);
        let file_nodes = reader.find_file_symbols("src/lib.rs", None)?;
        assert_eq!(file_nodes.len(), 3);
        let hunk_nodes = reader.find_file_symbols("src/lib.rs", Some((15, 16)))?;
        assert_eq!(hunk_nodes.len(), 1);
        assert_eq!(hunk_nodes[0].name, "target");
        assert_eq!(hunk_nodes[0].callers, ["caller"]);
        assert!(reader.find_file_symbols("src/lib.rs\0", None).is_err());
        assert!(
            reader
                .find_file_symbols("src/lib.rs", Some((0, 4)))
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn codegraph_reader_normalizes_historical_integer_ids() -> crate::Result<()> {
        let directory = tempdir()?;
        let path = directory.path().join("historical.db");
        let writer = Connection::open(&path)?;
        writer.execute_batch(
            "CREATE TABLE nodes (id INTEGER PRIMARY KEY, kind TEXT NOT NULL, name TEXT NOT NULL, qualified_name TEXT NOT NULL, file_path TEXT NOT NULL, start_line INTEGER NOT NULL, end_line INTEGER NOT NULL);\
             CREATE TABLE edges (source INTEGER NOT NULL, target INTEGER NOT NULL);\
             INSERT INTO nodes VALUES (7, 'function', 'legacy', 'pkg.legacy', 'src/lib.rs', 1, 1);",
        )?;
        drop(writer);
        let nodes = CodegraphReader::open(&path)?.find_symbol("legacy")?;
        assert_eq!(nodes[0].id, "7");
        assert_eq!(
            CodegraphReader::open(&path)?.find_file_symbols("src/lib.rs", None)?[0].id,
            "7"
        );
        Ok(())
    }

    #[test]
    fn current_codegraph_1_6_text_ids_are_read_when_index_is_present() -> crate::Result<()> {
        let database = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(".codegraph/codegraph.db");
        if !database.is_file() {
            return Ok(());
        }
        let nodes = CodegraphReader::open(&database)?.find_symbol("Store")?;
        assert!(!nodes.is_empty());
        assert!(nodes.iter().all(|node| node.id.contains(':')));
        assert!(nodes.iter().all(|node| node.start_line > 0));
        Ok(())
    }
}
