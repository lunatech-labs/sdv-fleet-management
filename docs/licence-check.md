# Third-party licence check

The Eclipse Foundation requires every third-party dependency of a contributed
component to be vetted with the [Eclipse Dash][dash] tool. Anything Dash marks
`restricted` needs an IP review ticket before the code can be merged upstream.

Run the check with:

```
scripts/check-3rd-party-licenses.sh
```

Add a review token to open the tickets automatically:

```
scripts/check-3rd-party-licenses.sh <gitlab-token>
```

The script writes the dependency list to `DEPS.txt` and the verdict to
`DASH_SUMMARY.txt`. Both are generated and are not committed.

## Why this script differs from the blueprint one

The blueprint script at `.github/scripts/check-3rd-party-licenses.sh` reads
`cargo tree` only. This project adds the operator dashboard, which is the first
JavaScript component in the stack, so the script here adds a second producer
that reads `package-lock.json` and emits npm coordinates.

Both producers exclude development dependencies. The cargo side uses
`-e no-build,no-dev`, and the npm side filters on the lockfile `dev` flag. Only
distributed code needs a review.

The script also replaces the GNU-only `sed -n '2~1p'` with `tail -n +2`, so it
runs on macOS as well as in CI.

## Result

Measured against 429 distributed dependencies:

| Status | Count |
|---|---|
| approved | 424 |
| restricted | 5 |

The npm surface is fully approved. No JavaScript dependency is restricted, which
answers the concern raised in issue #69 about the cost of adding a Node
toolchain.

The five restricted entries are all Rust crates:

| Crate | Declared licence |
|---|---|
| reqwest 0.12.28 | MIT OR Apache-2.0 |
| tokio 1.52.0 | none recorded |
| tower-http 0.5.2 | MIT |
| utoipa 4.2.3 | MIT OR Apache-2.0 |
| utoipa-gen 4.3.1 | MIT OR Apache-2.0 |

None of these is a licence problem. Every one is permissive, and `tokio` is
restricted only because ClearlyDefined holds no declared licence for that
version. Each needs an Eclipse IP ticket rather than a code change.

Note that `ring`, which the plan expected to be the likely blocker because
`reqwest` pulls it through `rustls`, is approved.

`utoipa` and `utoipa-gen` are the only two that a code change could remove, by
dropping the OpenAPI document. That is probably not worth it. The Swagger UI
package, which vendors web assets and carried the largest licence surface, was
already removed.

[dash]: https://github.com/eclipse/dash-licenses
