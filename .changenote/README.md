# Application release notes

Use changenote v0.10.0 at commit `f4423760122463bf03a8d161c6f0bc4173afb6e0`.
The Action and CLI pins must stay compatible.

Install the CLI:

```sh
cargo install --git https://github.com/narumiruna/changenote \
  --rev f4423760122463bf03a8d161c6f0bc4173afb6e0 --locked
```

For a change that affects users or release packages, add a note:

```sh
changenote add fix-description --package mangameeya-rust --bump patch \
  --message "Describe the effect on users."
changenote status
```

Use `minor` for new features and `major` for breaking changes. Stable SemVer bump rules
also apply before 1.0.0. Documentation and test changes without release impact need
no note. Do not use empty notes as exemptions. Commit the note with the change.
Only top-level note files are consumed; this README is ignored.

`Cargo.toml` is the version source. Do not add a second VERSION file or update
`.changenote/releases.json` by hand. The Release workflow manages the version PR,
Cargo lockfile update, changelog, release batch, and note removal. Multiple notes
for one package produce one version bump at the highest requested level.

See [the release workflow guide](../packaging/README.md#release-workflow) for setup,
publication, retries, package signing limits, and the required repository permission.
