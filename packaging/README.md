# Packaging & release

muckdb is distributed as prebuilt binaries through a Homebrew tap. Releases are
fully automated by `.github/workflows/release.yml`.

## One-time setup

1. **Create the tap repo** `nickkaltner/homebrew-muckdb` on GitHub (it can start
   empty — the release workflow writes `Formula/muckdb.rb` into it).

2. **Create a token** with write access to the tap repo and add it to the
   `nickkaltner/muckdb` repo secrets as `HOMEBREW_TAP_TOKEN`:
   - A fine-grained PAT scoped to `nickkaltner/homebrew-muckdb` with
     **Contents: Read and write**, or
   - A classic PAT with `repo` scope.

That's it. CI (`ci.yml`) needs no secrets.

## Cutting a release

```sh
# bump version in Cargo.toml, commit, then:
git tag v0.1.0
git push origin v0.1.0
```

The release workflow then:

1. Cross-builds `muckdb` for `aarch64-apple-darwin` and
   `x86_64-unknown-linux-gnu`.
2. Publishes a GitHub Release with the `muckdb-<version>-<target>.tar.gz`
   tarballs.
3. Regenerates `Formula/muckdb.rb` (via `scripts/render-formula.sh`, with the
   real sha256 sums) and commits it to the tap.

Users get it with:

```sh
brew install nickkaltner/muckdb/muckdb
brew upgrade muckdb   # after later releases
```

## Rendering the formula locally

```sh
scripts/render-formula.sh 0.1.0 path/to/dist-dir
```

where the dist dir holds the three release tarballs.
