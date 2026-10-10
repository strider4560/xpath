# Experimental compiled-schema interchange, version 1

This is a planning projection of a compiled schema, not a complete serialization
of libyang or a YANG validator. A compiler produces a flat JSON table; Rust
validates it and derives indexes. No native pointers or libyang node constants
cross this boundary. The spike tests a future compiler-replacement seam.

Top-level object (all fields required):

```json
{
  "format_version": 1,
  "compiler": "libyang 2.1.148",
  "modules": [
    {"name": "demo-if", "namespace": "urn:demo:if", "prefix": "if",
     "revision": "2026-10-10", "enabled_features": []}
  ],
  "nodes": [
    {"id": 0, "parent": null, "kind": "root", "module": "",
     "namespace": "", "name": "", "config": null,
     "key": false, "mandatory": false, "presence": false,
     "status": "current", "type_name": null, "when": [], "must": []}
  ]
}
```

- `id` is a dense u32 preorder index. Root is exactly ID 0. Every other node's
  `parent` is the immediate compiled-schema parent and precedes it; subtrees
  must be contiguous. Rust calculates exclusive subtree ends and a second
  parent/index that makes choice/case nodes transparent.
- Version 1 accepts at most 1,000,000 rows, including root, and at most 256
  rows along a root-to-node path (root depth 0, maximum non-root depth 255).
- `kind` is one of root, container, list, leaf, leaf-list, choice, case,
  anydata, anyxml, rpc, action, input, output, notification. The exporter must
  document which of these it emits.
- Names retain defining-module namespace identity, including augmented nodes.
  Module records are sorted by module name; children keep compiler schema order.
  Include metadata for every defining module used by emitted nodes.
- `config` is an effective boolean, or null where not applicable. `key`,
  `mandatory`, and `presence` are effective boolean flags. Keys are mandatory;
  presence means an explicit effective YANG presence statement, including
  one introduced by uses/refine. `status` is current,
  deprecated or obsolete. `type_name` is the builtin base type name for leaves
  and leaf-lists; null otherwise.
- `when` and `must` contain the original expression strings for discovery and
  inspection. They are not standalone executable YANG constraints: defining
  contexts, resolved prefix maps, accessible-tree rules, and other validation
  metadata are outside this prototype format.
- Feature state and module revisions describe the compiled selection. Disabled
  nodes are absent; uses/augments are already expanded by the compiler. Full
  type restrictions, deviations, defaults, extensions, identities, and constraint
  contexts are not represented here. This projection must not be treated as a
  complete schema-set identity for validation or data compatibility.

The supported builtin type vocabulary is binary, bits, boolean, decimal64,
empty, enumeration, identityref, instance-identifier, int8, int16, int32, int64,
leafref, string, uint8, uint16, uint32, uint64, and union. Unknown types fail
import rather than silently creating metadata the planner does not understand.

Storage uses a SHA-256 identifier of the normalized planning projection. Node IDs
are meaningful only with that identifier, and are not stable across schema edits.
Compiler provenance is retained but excluded from the structural fingerprint so
equivalent producers can be compared. Replacing libyang is a matter of producing
the same projection; expanding this format later requires a new format version.
