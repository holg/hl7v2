# Releasing

Three kinds of artefacts leave this repository, each from its own tag. All
publishing runs in GitHub Actions with Trusted Publishing; no tokens are
stored anywhere.

| What | Tag | Workflow | Goes to |
|---|---|---|---|
| `hl7kit` Rust crate | `hl7kit-vX.Y.Z` | `publish.yml` | crates.io |
| `mwlkit` Rust crate | `mwlkit-vX.Y.Z` | `publish.yml` | crates.io |
| `hl7kit` Python wheels | `hl7kit-py-vX.Y.Z` | `python.yml` | PyPI + GitHub release |
| `mwlkit` Python wheels | `mwlkit-py-vX.Y.Z` | `python.yml` | PyPI + GitHub release |

The browser demo deploys from every push to `main` (`pages.yml`).

## Python wheels

The binding crates live in `crates/hl7kit-py` and `crates/mwlkit-py`. Each
has two version fields that must agree: `version` in `Cargo.toml` and
`version` in `pyproject.toml`. The workflow refuses to run when they differ,
and refuses a tag whose version differs from the crate's.

1. Bump both files of the crate you release, e.g. for hl7kit:
   `crates/hl7kit-py/Cargo.toml` and `crates/hl7kit-py/pyproject.toml`.
   The Python package version does not have to equal the Rust crate's
   version, but keeping them in lockstep (hl7kit 0.2.x wheels wrap hl7kit
   0.2.x) is the convention.
2. Commit, push to `main`, and let the `python` workflow go green: it builds
   every wheel and runs the test suite against each one, but publishes
   nothing from a branch.
3. Tag and push:

   ```sh
   git tag hl7kit-py-v0.2.0 && git push origin hl7kit-py-v0.2.0
   # or
   git tag mwlkit-py-v0.1.0 && git push origin mwlkit-py-v0.1.0
   ```

   The tag selects the package. The workflow builds both (mwlkit's tests use
   the hl7kit wheel from the same run), publishes only the tagged one to PyPI
   with attestations, and creates a GitHub release named after the tag that
   carries the same wheels and sdist.

Wheels: manylinux_2_28 x86_64 and aarch64, musllinux_1_2 x86_64, macOS
universal2, Windows x64, all `abi3-py39` (one wheel per platform for
Python 3.9 and newer). Every native platform runs the test suite against
the wheel it built, installed with `--only-binary=:all:`; the musllinux
wheel is tested inside `python:3.9-alpine`; the aarch64 wheel is built under
emulation and not smoke-tested.

## One-time setup (done once per package, by the repository owner)

Trusted Publishing needs a "pending publisher" on PyPI for each package,
and the GitHub environment the workflow names. Both packages are published
since 2026-09-16; the entries exist.

PyPI (https://pypi.org/manage/account/publishing/), one entry per package:

| PyPI project name | Owner | Repository | Workflow name | Environment name |
|---|---|---|---|---|
| `hl7kit` | `holg` | `hl7v2` | `python.yml` | `pypi` |
| `mwlkit` | `holg` | `hl7v2` | `python.yml` | `pypi` |

GitHub: Settings > Environments, `pypi` (created on first use). Recommended:
restrict its deployment branches and tags to `hl7kit-py-v*` and
`mwlkit-py-v*` and add yourself as a required reviewer.

## Rust crates

See the "Releasing" section of the root README: tag `hl7kit-vX.Y.Z` or
`mwlkit-vX.Y.Z` with the version in the crate's `Cargo.toml`.
