#!/bin/sh
# Download what fhirkit's conformance tests read, into tmp/fhir/ at the
# repository root. Nothing here is committed.
#
#   core/package      hl7.fhir.r4.core 4.0.1      (the generator's input)
#   examples/package  hl7.fhir.r4.examples 4.0.1  (5,309 round-trip examples)
#   sql-on-fhir/      FHIR/sql-on-fhir.js tests/  (the SQL-on-FHIR suite)
#
# The two packages are fixed releases: downloaded once, then reused. The
# SQL-on-FHIR suite changes often, so it is fetched fresh from main on
# every run; the commit it came from is written to sql-on-fhir/COMMIT.
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
dir="$root/tmp/fhir"
mkdir -p "$dir"

package() {
    name=$1 into=$2
    if [ -f "$dir/$into/package/package.json" ]; then
        echo "$name: cached"
        return
    fi
    rm -rf "${dir:?}/$into"
    mkdir -p "$dir/$into"
    curl -fsSL "https://packages.fhir.org/$name/4.0.1" | tar -xz -C "$dir/$into"
    echo "$name: $(ls "$dir/$into/package" | wc -l | tr -d ' ') files"
}

package hl7.fhir.r4.core core
package hl7.fhir.r4.examples examples

repo=https://github.com/FHIR/sql-on-fhir.js
commit=$(git ls-remote "$repo" refs/heads/main | cut -f1)
rm -rf "$dir/sql-on-fhir"
mkdir -p "$dir/sql-on-fhir"
curl -fsSL "https://codeload.github.com/FHIR/sql-on-fhir.js/tar.gz/$commit" \
    | tar -xz -C "$dir/sql-on-fhir" --strip-components=1 "sql-on-fhir.js-$commit/tests"
echo "$commit" > "$dir/sql-on-fhir/COMMIT"
count=$(ls "$dir/sql-on-fhir/tests"/*.json | wc -l | tr -d ' ')
[ "$count" -gt 0 ] || { echo "sql-on-fhir: no test files found" >&2; exit 1; }
echo "sql-on-fhir tests: $count files at $commit"
