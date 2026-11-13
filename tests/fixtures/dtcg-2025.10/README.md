# Pinned DTCG Format fixtures

`manifest.json`, `positive/`, and `negative/` are copied unchanged from
[`design-tokens/community-group`, commit `882ebd6716abef9a46d8ef5fb12e82009d6cee2e`](https://github.com/design-tokens/community-group/tree/882ebd6716abef9a46d8ef5fb12e82009d6cee2e/test-suite/tests/2025.10/format).
The source directory is `test-suite/tests/2025.10/format/`.

`LICENSE.md` is the unchanged [repository-root license notice](https://github.com/design-tokens/community-group/blob/882ebd6716abef9a46d8ef5fb12e82009d6cee2e/LICENSE.md)
from that same commit. It identifies the W3C 3-clause BSD License for test-suite
and other software contributions.

The manifest contains 220 entries: 93 `PositiveEvaluationTest` entries and 127
`NegativeEvaluationTest` entries. Of these, 48 have the `preprocessing-required`
feature. `tests/conformance.rs` passes every entry directly to the public
`dtcg_validator::validate` function, including those requiring preprocessing,
and reports all mismatching fixture IDs together. It asserts the pinned counts,
that every entry was exercised, and that all three override IDs remain present
and declared negative upstream.

## Published-report disagreements

The conformance authority is the published [Format 2025.10 report](https://www.designtokens.org/TR/2025.10/format/)
and its referenced [Color 2025.10 report](https://www.designtokens.org/TR/2025.10/color/),
not the later suite or JSON Schema. Exactly these three upstream negative
expectations are overridden to require acceptance:

| Fixture ID | Published-report interpretation |
| --- | --- |
| `gradient-empty-array` | No minimum gradient-array length is specified. |
| `strokeStyle-empty-dashArray` | No minimum dash-array length is specified. |
| `metadata-unsupported-dollar-property` | No blanket rejection rule is specified for unknown scalar `$` metadata. |

These exceptions change only the harness expectation. The upstream manifest and
fixture bytes are not modified, and no entries are skipped. Other conflicts,
such as RFC 6901 root/empty-key pointers, alias-derived type precedence,
`$root` aliases, and clamped gradient positions, have independent public-core
coverage in `tests/edge_cases.rs` or the existing unit tests.

Run the conformance and supplemental integration tests from the repository root:

```sh
cargo test --test conformance --test edge_cases
```
