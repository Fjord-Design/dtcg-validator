# dtcg-validator

A Design Tokens Community Group **2025.10 Format** validator with a shared Rust
core, native CLI, and browser WebAssembly API. References resolve only within the
input document; the validator performs no filesystem or network I/O and never
fetches external references. The CLI reads the requested file or standard input.

## Rust API

```rust
use dtcg_validator::{validate, ValidationReport};

let report: ValidationReport = validate(r#"{"n":{"$type":"number","$value":1}}"#);
assert!(report.valid);
assert!(report.diagnostics.is_empty());
```

`pub fn validate(input: &str) -> ValidationReport` accepts a complete JSON text.
The public report types implement `serde::Serialize`:

```rust
pub struct ValidationReport {
    pub valid: bool,
    pub diagnostics: Vec<Diagnostic>,
}

pub struct Diagnostic {
    pub path: String,
    pub code: String,
    pub message: String,
}
```

`valid` is true exactly when `diagnostics` is empty. Diagnostics are sorted by
`path`, then `code`, then `message`. Codes are the categories `json`, `structure`,
`metadata`, `type`, `value`, `reference`, and `cycle`. Paths are
[RFC 6901 JSON Pointers](https://www.rfc-editor.org/rfc/rfc6901): `""` identifies
the document root, and `/n/$value` identifies a token value. Path segments escape
`~` as `~0` and `/` as `~1`. Malformed JSON returns a root-level `json` diagnostic
with line and column, rather than a separate error result.

### Supported values and input semantics

All 13 token types are supported: `number`, `dimension`, `duration`, `fontFamily`,
`fontWeight`, `cubicBezier`, `color`, `strokeStyle`, `border`, `transition`,
`shadow`, `gradient`, and `typography`.

Color validation covers all 14 spaces: `srgb`, `srgb-linear`, `display-p3`,
`a98-rgb`, `prophoto-rgb`, `rec2020`, `xyz-d65`, `xyz-d50`, `hsl`, `hwb`, `lab`,
`lch`, `oklab`, and `oklch`, with their specified component ranges and `none`.

Numbers retain exact decimal precision, including large exponents; bounded
comparisons do not round through `f64`. For example, a `number` value of `1e1000`
is accepted, while a `fontWeight` of `1000.0000000000000001` is out of range.
Duplicate object members are preserved, not treated as malformed JSON or silently
collapsed. Conflicting duplicate structural metadata produces a `structure`
diagnostic; a reference lookup through a non-unique member fails even when the
duplicate values are identical.

## Native CLI

Requires Rust/Cargo with Rust 2024 edition support. Run from the repository root:

```sh
cargo run --quiet -- tokens.json
printf '%s' '{"n":{"$type":"number","$value":1}}' | cargo run --quiet -- --json -
cargo run --quiet -- -- -tokens.json
cargo run --quiet -- --help
```

The executable syntax is `dtcg-validator [--json] <FILE|->`. Exactly one operand
is required; `-` reads standard input. `--` ends option parsing, allowing file
names such as `-tokens.json`. Missing or multiple operands and unknown options
are usage errors. `--help`/`-h` and `--version`/`-V` print to stdout and exit 0.

| Result | Exit status | Normal output | With `--json` |
| --- | --- | --- | --- |
| Valid document | 0 | Silent | Shared report on stdout |
| Invalid document, including malformed JSON | 1 | Diagnostics on stderr: path, category, message | Shared report on stdout; no stderr |
| Usage or input I/O error | 2 | Error on stderr | Error on stderr; no JSON report |

JSON mode writes exactly one report followed by a newline, for example
`{"valid":true,"diagnostics":[]}`. It uses the same report contract as the Rust
and browser APIs.

## Browser / WebAssembly

Install `wasm-pack` and the Rust target, then build the web-target package:

```sh
rustup target add wasm32-unknown-unknown
wasm-pack build --target web
```

From a browser ES module served at the repository root:

```js
import init, { validate, version } from './pkg/dtcg_validator.js';

await init();
const report = validate('{"n":{"$type":"number","$value":1}}');
console.log(version(), report.valid, report.diagnostics);
```

Call `init()` before the other exports. `validate(input)` accepts JSON **text**
and returns the shared report as a JavaScript object, not a JSON string.
Invalid documents return `valid: false`; initialization or report-conversion
errors throw. `version()` returns the crate version string. No external
JavaScript runtime dependencies are required beyond the generated JS/WASM files.

Serve the repository root over HTTP, rather than opening an HTML file directly:

```sh
python3 -m http.server 8000
```

Open <http://127.0.0.1:8000/examples/web/> to paste JSON and inspect diagnostics
with their paths. The example displays initialization errors as well as validation
results. If port 8000 is occupied, choose another port and update the URLs below.

## Conformance and fixture provenance

The authority is the published [Format 2025.10 report](https://www.designtokens.org/TR/2025.10/format/)
and its referenced [Color 2025.10 report](https://www.designtokens.org/TR/2025.10/color/),
not unreleased drafts or the later test suite/schema. Normative rules take
precedence over examples; violating a `SHOULD` alone does not make input invalid.
The reports contain conflicting passages, resolved here as follows:

- `$extends` deep-merges groups and replaces whole token definitions at matching
  paths. Opaque metadata, including vendor extension contents, is not evaluated
  for references.
- Token-level `$ref` aliases without `$value` and aliases to `$root` are supported.
  An explicit token type wins; otherwise a whole-token alias takes its target's
  type before an inherited group type.
- JSON Pointer follows RFC 6901: `#` addresses the root, `#/` addresses an
  empty-name member, and a pointer returns exactly its target without implicitly
  appending `$value`.
- Gradient positions outside `[0,1]` are accepted because the report specifies
  clamping. Arrays without a stated minimum length may be empty. Unknown `$`
  metadata is opaque unless it disguises a forbidden token/group name;
  `$extensions` remains opaque regardless of contents.

The official Format manifest and fixtures are vendored unchanged in
[`tests/fixtures/dtcg-2025.10/`](tests/fixtures/dtcg-2025.10/) from
[`design-tokens/community-group` commit `882ebd6716abef9a46d8ef5fb12e82009d6cee2e`](https://github.com/design-tokens/community-group/tree/882ebd6716abef9a46d8ef5fb12e82009d6cee2e/test-suite/tests/2025.10/format),
with the upstream [license notice](tests/fixtures/dtcg-2025.10/LICENSE.md).
The harness exercises all 220 entries, including `preprocessing-required` cases.
Exactly three upstream negative expectations are overridden to require acceptance:

| Fixture ID | Reason |
| --- | --- |
| `gradient-empty-array` | No minimum gradient length is specified. |
| `strokeStyle-empty-dashArray` | No minimum dash-array length is specified. |
| `metadata-unsupported-dollar-property` | No blanket ban on unknown scalar `$` metadata is specified. |

The manifest and fixture bytes remain unchanged; entries are not skipped.
See the [fixture notes](tests/fixtures/dtcg-2025.10/README.md) for provenance and
harness details. Core and adapter tests use shared checked-in fixtures; the
browser smoke test fetches them from the local HTTP server, not from upstream.

## Verification

From the repository root, with Rust/Cargo, the WASM target, `wasm-pack`, Python 3,
and a browser installed:

```sh
cargo test
wasm-pack build --target web
python3 -m http.server 8000
```

Keep the server running. In another terminal, open
<http://127.0.0.1:8000/tests/web/validation.html> or run this macOS Chrome smoke
command (adjust the browser executable on other systems):

```sh
'/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' \
  --headless --disable-gpu --user-data-dir="$(mktemp -d)" \
  --virtual-time-budget=10000 --dump-dom \
  http://127.0.0.1:8000/tests/web/validation.html
```

The rendered body must say `PASS`; `FAIL` includes the error. This exercises the
built web-target module in a browser. Also use the interactive example with valid,
invalid, and malformed JSON to inspect the displayed results and paths.
