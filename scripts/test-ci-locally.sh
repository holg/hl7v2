#!/bin/sh
# Run what .github/workflows/ci.yml runs, locally, in the same order.
#
#   scripts/test-ci-locally.sh           format, clippy (host + wasm), fetch, tests
#   scripts/test-ci-locally.sh --full    ... and the trunk release build of the demo
#
# Then the Python wheels, as .github/workflows/python.yml builds them, when
# a Python environment with maturin and pytest is configured: set
# LOCAL_CI_PYENV to its directory, or put LOCAL_CI_PYENV=... in
# .local-ci.env (git-ignored). That environment is only used to build; the
# wheels are installed and tested in a throwaway venv under tmp/.
#
# Test fixtures (FHIR packages, the SQL-on-FHIR suite) are fetched into
# tmp/fhir/ first; the SQL-on-FHIR suite is fetched fresh every run.
set -eu
cd "$(dirname "$0")/.."
full=0
[ "${1:-}" = "--full" ] && full=1

step() { printf '\n== %s\n' "$*"; }

step Format
cargo fmt --all -- --check

step "Clippy (host)"
cargo clippy --workspace --all-targets -- -D warnings

step "Clippy (wasm32)"
cargo clippy -p dicomscope --target wasm32-unknown-unknown -- -D warnings
cargo clippy -p fhirkit --target wasm32-unknown-unknown -- -D warnings

step "Fetch FHIR fixtures"
crates/fhirkit/fetch-fixtures.sh

step "Test (host)"
cargo test --workspace

# GitHub's runners have no graphics adapter. Asking wgpu for a backend this
# machine does not have reproduces that: the GPU tests must skip or degrade,
# never fail. (dx12 is only compiled in on Windows; there, metal is absent.)
step "Test (host, no GPU adapter, as on the CI runner)"
nogpu=dx12
case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*|Windows_NT) nogpu=metal ;; esac
WGPU_BACKEND=$nogpu cargo test -p dicomscope-core -p dicomscope-tui -p dicomscope-desktop

step "No network APIs in the demo"
if grep -rnE '(^|[^A-Za-z0-9_])(fetch\(|XMLHttpRequest|WebSocket|EventSource)' demo/dicomscope/src demo/dicomscope-core/src demo/dicomscope-desktop/src crates/hl7kit/src crates/mwlkit/src crates/fhirkit/src; then
    echo "network API reference found"; exit 1
fi
if grep -nE '"(Request|Response|Fetch|WebSocket|XmlHttpRequest|Headers)"' demo/dicomscope/Cargo.toml; then
    echo "web-sys network feature enabled"; exit 1
fi
if grep -nE '<script|<link[^>]+href="https?:' demo/dicomscope/index.html; then
    echo "external resource in index.html"; exit 1
fi

step "Python wheels"
[ -f .local-ci.env ] && . ./.local-ci.env
pyenv=${LOCAL_CI_PYENV:-}
if [ -z "$pyenv" ] || [ ! -x "$pyenv/bin/maturin" ]; then
    echo "skipped: set LOCAL_CI_PYENV (or .local-ci.env) to a Python environment with maturin and pytest"
else
    py="$pyenv/bin/python"
    crates="hl7kit-py mwlkit-py fhirkit-py"
    for c in $crates; do
        cv=$(sed -n 's/^version = "\([^"]*\)".*/\1/p' "crates/$c/Cargo.toml" | head -1)
        pv=$(sed -n 's/^version = "\([^"]*\)".*/\1/p' "crates/$c/pyproject.toml" | head -1)
        [ "$cv" = "$pv" ] || { echo "$c: Cargo.toml says $cv, pyproject.toml says $pv"; exit 1; }
        echo "$c: $cv"
    done
    # The binding crates' Rust tests embed the interpreter; tell it where
    # its standard library and shared library live.
    base=$("$py" -c 'import sys; print(sys.base_prefix)')
    libdir=$("$py" -c 'import sysconfig; print(sysconfig.get_config_var("LIBDIR") or "")')
    for c in $crates; do
        (cd "crates/$c" &&
            PYO3_PYTHON="$py" cargo fmt -- --check &&
            PYO3_PYTHON="$py" cargo clippy --all-targets -- -D warnings &&
            PYO3_PYTHON="$py" PYTHONHOME="$base" \
                DYLD_LIBRARY_PATH="$libdir" LD_LIBRARY_PATH="$libdir" cargo test -q)
    done
    rm -rf tmp/wheels
    for c in $crates; do
        "$pyenv/bin/maturin" build --release -q -o tmp/wheels -i "$py" -m "crates/$c/Cargo.toml"
    done
    ls tmp/wheels
    venv=tmp/venv-wheels
    [ -x "$venv/bin/python" ] || "$py" -m venv "$venv"
    "$venv/bin/python" -m pip install -q --disable-pip-version-check pytest
    "$venv/bin/python" -m pip install -q --force-reinstall --no-deps --no-index \
        --only-binary=:all: --find-links tmp/wheels hl7kit mwlkit fhirkit-rs
    # Then the test extras (pydicom for mwlkit ...). The packages themselves
    # are already the local wheels, so pip only adds what is missing.
    "$venv/bin/python" -m pip install -q "hl7kit[test]" "mwlkit[test]" "fhirkit-rs[test]"
    "$venv/bin/python" -m pip show hl7kit mwlkit fhirkit-rs | grep -E '^(Name|Version|Location)'
    for c in $crates; do
        "$venv/bin/python" -m pytest -q "crates/$c/tests"
    done
fi

if [ "$full" = 1 ]; then
    step "Build demo"
    (cd demo/dicomscope && trunk build --release)
fi

step "local CI passed"
