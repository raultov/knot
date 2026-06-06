#!/usr/bin/env bash
# Helper functions for asserting Neo4j relationships in E2E tests.
#
# Connection parameters must be supplied via environment variables:
#   NEO4J_URI       (e.g. bolt://localhost:17687)
#   NEO4J_USER      (e.g. neo4j)
#   NEO4J_PASSWORD  (e.g. e2e_test_password)
#
# Functions:
#   assert_no_edge       <source_fqn>  <target_fqn>  <rel_type>
#   assert_edge_exists   <source_fqn>  <target_fqn>  <rel_type>
#   assert_edge_count    <source_fqn>  <rel_type>    <expected_count>
#
# All functions return 0 on success and 1 on failure, printing a clear
# diagnostic on failure. They use cypher-shell to talk to Neo4j.

set -u

# Resolve cypher-shell: prefer host binary (path required for Docker-based
# databases whose Bolt port is mapped to localhost).
: "${CYPHER_SHELL:=cypher-shell}"

# Make sure required connection parameters are set
_require_neo4j_env() {
    if [ -z "${NEO4J_URI:-}" ] || [ -z "${NEO4J_USER:-}" ] || [ -z "${NEO4J_PASSWORD:-}" ]; then
        echo "ASSERTION ERROR: NEO4J_URI, NEO4J_USER and NEO4J_PASSWORD must be set" >&2
        return 1
    fi
}

# Run a Cypher query and return the value of the first column of the first row.
# Strips the header/footer noise produced by cypher-shell's default output.
_run_cypher_value() {
    local query="$1"

    _require_neo4j_env

    echo "$query" | "$CYPHER_SHELL" \
        -a "$NEO4J_URI" \
        -u "$NEO4J_USER" \
        -p "$NEO4J_PASSWORD" \
        --format plain \
        2>/dev/null \
        | awk 'NF && NR > 1 && $0 !~ /^(Available|neo4j>|Connection|Disconnect|Connected)/ { print; exit }'
}

# Escape single quotes for safe embedding in Cypher string literals.
_escape_cypher_string() {
    local s="$1"
    s="${s//\'/\'\'}"
    printf '%s' "$s"
}

# Assert that NO relationship of <rel_type> exists from <source_fqn> to <target_fqn>.
#
# Args:
#   source_fqn  - Fully-qualified name (or unique identifier) of the source node
#   target_fqn  - Fully-qualified name of the target node
#   rel_type    - Relationship type (e.g. REFERENCES, CALLS)
assert_no_edge() {
    local source_name
    local target_name
    local rel_type="$3"
    source_name=$(_escape_cypher_string "$1")
    target_name=$(_escape_cypher_string "$2")

    local query="MATCH (a)-[r:${rel_type}]->(b) WHERE a.fqn = '${source_name}' AND b.fqn = '${target_name}' RETURN count(r) AS cnt;"
    local result
    result=$(_run_cypher_value "$query")

    # Default to 0 when cypher-shell returned no value
    if [ -z "$result" ]; then
        result="0"
    fi

    if [ "$result" != "0" ]; then
        echo "ASSERTION FAILED: Expected no ${rel_type} edge from '${1}' to '${2}', but found ${result}" >&2
        return 1
    fi
}

# Assert that AT LEAST one relationship of <rel_type> exists from <source_fqn> to <target_fqn>.
#
# Args:
#   source_fqn  - Fully-qualified name of the source node
#   target_fqn  - Fully-qualified name of the target node
#   rel_type    - Relationship type (e.g. CALLS, REFERENCES)
assert_edge_exists() {
    local source_name
    local target_name
    local rel_type="$3"
    source_name=$(_escape_cypher_string "$1")
    target_name=$(_escape_cypher_string "$2")

    local query="MATCH (a)-[r:${rel_type}]->(b) WHERE a.fqn = '${source_name}' AND b.fqn = '${target_name}' RETURN count(r) AS cnt;"
    local result
    result=$(_run_cypher_value "$query")

    if [ -z "$result" ]; then
        result="0"
    fi

    if [ "$result" = "0" ]; then
        echo "ASSERTION FAILED: Expected at least one ${rel_type} edge from '${1}' to '${2}', but found none" >&2
        return 1
    fi
}

# Assert that the number of outgoing <rel_type> relationships from <source_fqn>
# matches <expected_count> exactly.
#
# Args:
#   source_fqn       - Fully-qualified name of the source node
#   rel_type         - Relationship type (e.g. CALLS, REFERENCES)
#   expected_count   - Expected number of outgoing edges (integer)
assert_edge_count() {
    local source_name
    local rel_type="$2"
    local expected_count="$3"
    source_name=$(_escape_cypher_string "$1")

    local query="MATCH (a)-[r:${rel_type}]->() WHERE a.fqn = '${source_name}' RETURN count(r) AS cnt;"
    local result
    result=$(_run_cypher_value "$query")

    if [ -z "$result" ]; then
        result="0"
    fi

    if [ "$result" != "$expected_count" ]; then
        echo "ASSERTION FAILED: Expected exactly ${expected_count} ${rel_type} edge(s) from '${1}', but found ${result}" >&2
        return 1
    fi
}
