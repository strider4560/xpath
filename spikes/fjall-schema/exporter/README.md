# libyang import-time exporter

`schema-export.c` compiles YANG with **libyang v2.1.148** and projects the
compiled `lysc_node` tree into [the version 1 contract](../FORMAT.md). It never
parses YANG text itself or asks an XPath engine to reconstruct the schema.
The checked-in JSON fixtures are the only input the Rust spike needs: building
or running Rust does not require libyang, CMake, a C compiler, or YANG files.

## Build the pinned compiler

Tested source: official CESNET/libyang tag `v2.1.148`, commit
`fc4dbd923e044006c93df020590a1e5a8656c09e`.
The commands below assume a C11 compiler, CMake, PCRE2 headers, and a PCRE2
runtime. They install entirely inside `SPIKE_WORK`.

Run from `spikes/fjall-schema`:

```sh
SPIKE_DIR="$PWD"
SPIKE_WORK="/absolute/path/to/scratch/flat-schema-tools"
CMAKE_BIN="cmake"
LY_SOURCE="$SPIKE_WORK/libyang-src"
LY_BUILD="$SPIKE_WORK/libyang-build"
LY_PREFIX="$SPIKE_WORK/libyang-install"
EXPORTER_BIN="$SPIKE_WORK/schema-export"

mkdir -p "$SPIKE_WORK"
git clone --branch v2.1.148 --depth 1 https://github.com/CESNET/libyang.git "$LY_SOURCE"
test "$(git -C "$LY_SOURCE" rev-parse HEAD)" = fc4dbd923e044006c93df020590a1e5a8656c09e

"$CMAKE_BIN" -S "$LY_SOURCE" -B "$LY_BUILD" \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_INSTALL_PREFIX="$LY_PREFIX" -DCMAKE_INSTALL_LIBDIR=lib \
  -DENABLE_TESTS=OFF -DENABLE_VALGRIND_TESTS=OFF \
  -DENABLE_PERF_TESTS=OFF -DENABLE_FUZZ_TARGETS=OFF
"$CMAKE_BIN" --build "$LY_BUILD" -j 4
"$CMAKE_BIN" --install "$LY_BUILD"

cc -std=c11 -O2 -g -Wall -Wextra -Wpedantic -Werror \
  -I"$LY_PREFIX/include" "$SPIKE_DIR/exporter/schema-export.c" \
  -L"$LY_PREFIX/lib" -Wl,-rpath,"$LY_PREFIX/lib" -lyang \
  -o "$EXPORTER_BIN"
```

If the PCRE2 runtime is installed without an unversioned `libpcre2-8.so`,
create a local development link and add its directory to the CMake configure
command. The exact workaround used in this environment was:

```sh
mkdir -p "$SPIKE_WORK/pcre2-lib"
ln -s /usr/lib/x86_64-linux-gnu/libpcre2-8.so.0.11.2 "$SPIKE_WORK/pcre2-lib/libpcre2-8.so"
# Add this argument to the configure command above:
# -DCMAKE_LIBRARY_PATH="$SPIKE_WORK/pcre2-lib"
```

For the build that generated these fixtures, the existing clone was at
`/workspace/scratch/ed787c970b90/libyang-spike-upstream`, CMake was
`/workspace/scratch/ed787c970b90/flat-spike-tools/cmake/data/bin/cmake`, the build
and install directories were `libyang-spike-build` and `libyang-spike-install`
under the same scratch directory, and the exporter was
`/workspace/scratch/ed787c970b90/flat-spike-tools/schema-export`.
The exporter compiles warning-free with the flags above. The upstream library
build completed with GCC warnings in `lyd_trim_xpath` and the empty debug-tool
translation unit; upstream tests were disabled for this import-time build.

The pinned release's `LY_VERSION` header reports its ABI version **2.46.3**,
not its release version 2.1.148. The exporter checks that header ABI at compile
time and records `"libyang 2.1.148"` as compiler provenance. Use the pinned
source and installed library together; the ABI guard is not a cryptographic
attestation of the runtime library.

## Run and regenerate

```sh
"$EXPORTER_BIN" fixtures/demo-if.yang fixtures/demo-augment.yang \
  > fixtures/demo-schema.json
"$EXPORTER_BIN" --feature demo-if:diagnostics \
  fixtures/demo-if.yang fixtures/demo-augment.yang \
  > fixtures/demo-schema-feature.json
python3 exporter/test_exporter.py "$EXPORTER_BIN"
```

CLI: `schema-export [--search-dir DIR] [--feature MODULE:FEATURE] MODULE.yang...`.
Both options can repeat. Each input's directory is automatically added as an
import/include search directory. Features are disabled by default; each
requested feature must exist. Use actual module names in feature options,
not YANG prefixes; wildcard feature names are not supported. Input modules
are sorted by path before compilation. An imported module given as an input
is explicitly implemented. Imports and includes are resolved by libyang.
Only YANG input syntax is accepted by this executable.

Valid JSON goes to stdout; errors go to stderr with a nonzero exit status.
Parsing, compilation, feature lookup, and projection failures occur before
JSON output begins. An output I/O or allocation failure while emitting can
leave a partial stream and still returns nonzero; check the exit status when
installing an export. JSON strings escape quotes, backslashes, and all ASCII
control characters while retaining valid UTF-8 text.

## Projection semantics and limits

The scope is **compiled datastore data roots**. Emitted kinds are `root`,
`container`, `list`, `leaf`, `leaf-list`, `choice`, `case`, `anydata`, and
`anyxml`. RPCs, actions, input/output nodes, notifications, and extension-owned
trees are excluded, even though the format reserves names for operation
nodes. `demo-if.yang` deliberately includes an RPC and a notification to
verify that boundary.

- Modules and top-level module groups are sorted by module name. Children
  retain libyang's compiled schema order, including keys reordered by the
  compiler and augmented children. There is no claimed order between source
  statements belonging to different modules.
- IDs are dense preorder indices beginning at the synthetic root 0. Physical
  choice and case rows remain. Case child traversal stops at the physical
  parent boundary because libyang links children across sibling cases.
- The node's `module` pointer determines its name and namespace, preserving
  foreign augment identity. The export includes implemented user modules
  even if they add no roots, plus every defining module of an emitted node.
  Internal support modules and imported-only modules with no emitted nodes
  are omitted. This is not a complete module dependency manifest.
- Module `revision` is a string or null when absent. `enabled_features` is
  sorted and reports libyang's effective feature state, including features
  declared in included submodules.
- `config` comes from compiled effective config bits, with null for the root
  or an inapplicable config context. `mandatory` includes the compiler's
  mandatory flag plus list keys, whose values are required even when
  libyang's mandatory bit is unset. `presence` means an explicit effective
  YANG presence statement, including one introduced by `uses/refine`.
  In the pinned release, the sole assignment of `LYS_PRESENCE` in
  `schema_compile_node.c` is guarded by the effective parsed presence
  statement. Despite the broader historical header comment, containers
  with only a condition, action, or notification retain `presence: false`;
  the regression test verifies all of these cases. Reading an original
  parsed node through `LY_CTX_SET_PRIV_PARSED` would miss presence added by
  a refinement. Other node kinds always have `presence: false`, avoiding
  overlapping libyang flag meanings. Recheck this mapping for another
  compiler version.
- `type_name` records the builtin base type. A leafref remains `leafref`,
  and a union remains `union`; resolved target types, paths, ranges,
  patterns, enum members, defaults, and other restrictions are omitted.
- `when` and `must` are retrieved with `lyxp_get_expr` from compiled
  constraints. Expanded uses and augment conditions may appear on compiled
  nodes. The strings preserve the parsed expression, including whitespace,
  rather than its original YANG quoting syntax. They lack prefix maps and
  evaluation contexts, so they cannot be executed as standalone constraints.
- The synthetic root has null config/type/parent, empty names, false flags,
  current status, and no constraints. The exporter and Rust importer cap the
  projection at **1,000,000 total rows including the virtual root**, and each
  ancestor path at **256 rows including the root**. The virtual root is depth
  0, top-level YANG nodes enter `collect_node` at depth 1, and the largest
  accepted non-root depth is 255. Limits are checked while collecting rows,
  before any JSON is written.

The format intentionally omits validation and data-compatibility information;
it is a planning projection. See `FORMAT.md` for the shared boundary and
`fixtures/expected-selections.json` for manually specified fixture expectations.

## Verification

The eight Python tests execute the compiled exporter against real YANG and
check physical preorder, case boundaries, grouping expansion, foreign augment
identity, effective metadata, escaped constraint strings, feature selection,
byte stability under reversed CLI module order, nonzero errors for invalid
features, options, and YANG, explicit presence semantics including
`uses/refine`, the accepted/rejected depth boundary, and a valid module named
`xml` with namespace `urn:xml` and prefix `z`. The exporter preserves that
module identity; namespace aliases used for XPath belong to the consumer.
These tests contain no XPath parser or evaluator.

Default fixture: **29 rows**, **2 modules**, **16 leaves**, **1 leaf-list**,
**5 containers**, **1 list**, **1 choice**, **2 cases**, **1 anydata**,
**1 anyxml**, and **1 synthetic root**. Six rows have `config: false`, four
have `mandatory: true`, and two have `presence: true`. Three nodes each carry
one `when`, and three nodes each carry one `must`. The enabled fixture has
**30 rows** and **17 leaves**, with all other counts unchanged except one
additional `config: true` row.
