#!/usr/bin/env python3
"""Exercise the real exporter and compiled YANG semantics, without an XPath engine."""

import json
import pathlib
import subprocess
import sys
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
FIXTURES = HERE.parent / "fixtures"
EXPORTER = pathlib.Path(sys.argv.pop(1)).resolve() if len(sys.argv) > 1 else HERE / "schema-export"


class ExporterTests(unittest.TestCase):
    def run_export(self, *options, modules=None):
        self.assertTrue(EXPORTER.is_file(), f"exporter has not been built: {EXPORTER}")
        paths = modules if modules is not None else [FIXTURES / "demo-if.yang", FIXTURES / "demo-augment.yang"]
        return subprocess.run([str(EXPORTER), *options, *map(str, paths)], text=True, capture_output=True)

    def document(self, *options, modules=None):
        result = self.run_export(*options, modules=modules)
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout), result.stdout

    def test_compiled_nodes_have_physical_preorder_and_expanded_namespaces(self):
        doc, _ = self.document()
        self.assertEqual(doc["format_version"], 1)
        self.assertEqual(doc["compiler"], "libyang 2.1.148")
        self.assertEqual([m["name"] for m in doc["modules"]], ["demo-augment", "demo-if"])
        self.assertEqual(doc["nodes"][0]["kind"], "root")
        nodes = doc["nodes"]
        self.assertEqual([n["id"] for n in nodes], list(range(len(nodes))))
        by_name = {n["name"]: n for n in nodes}
        self.assertEqual(by_name["static-address"]["parent"], by_name["static"]["id"])
        self.assertEqual(by_name["static-prefix"]["parent"], by_name["static"]["id"])
        self.assertEqual(by_name["dhcp"]["parent"], by_name["dynamic"]["id"])
        self.assertEqual(by_name["dynamic"]["parent"], by_name["addressing"]["id"])
        self.assertEqual(by_name["tag"]["parent"], by_name["interface"]["id"])
        self.assertEqual(by_name["tag"]["module"], "demo-augment")
        self.assertEqual(by_name["tag"]["namespace"], "urn:demo:augment")
        self.assertEqual(by_name["description"]["module"], "demo-if")
        self.assertNotIn("common-settings", by_name)
        self.assertNotIn("diagnostic-mode", by_name)
        self.assertNotIn("reset", by_name)
        self.assertNotIn("changed", by_name)
        # A case child's next pointer can lead into the next case in libyang.
        # Checking uniqueness and contiguous parent ancestry catches that trap.
        self.assertEqual(len(by_name), len(nodes))
        open_ancestors = [0]
        for node in nodes[1:]:
            self.assertLess(node["parent"], node["id"])
            while open_ancestors and open_ancestors[-1] != node["parent"]:
                open_ancestors.pop()
            self.assertTrue(open_ancestors, f"noncontiguous subtree at {node}")
            open_ancestors.append(node["id"])

    def test_effective_metadata_and_constraint_json_escaping(self):
        doc, output = self.document()
        n = {n["name"]: n for n in doc["nodes"]}
        self.assertTrue(n["name"]["key"])
        self.assertTrue(n["name"]["mandatory"])
        self.assertTrue(n["mtu"]["mandatory"])
        self.assertTrue(n["addressing"]["mandatory"])
        self.assertTrue(n["mtu"]["config"])
        self.assertFalse(n["packets"]["config"])
        self.assertTrue(n["inventory"]["presence"])
        self.assertFalse(n["packets"]["presence"])
        self.assertEqual(n["description"]["status"], "deprecated")
        self.assertEqual(n["legacy-name"]["status"], "obsolete")
        self.assertEqual(n["peer"]["type_name"], "leafref")
        self.assertEqual(n["oper-status"]["type_name"], "enumeration")
        self.assertEqual(n["temperature"]["type_name"], "decimal64")
        self.assertEqual(n["interface"]["must"], ["not(enabled = 'true') or mtu >= 576"])
        self.assertEqual(n["description"]["must"], ['not(contains(., \'"\'))\n\tor string-length(.) <= 80'])
        self.assertEqual(n["enabled-label"]["when"], ["../enabled = 'true'"])
        self.assertEqual(n["tag"]["when"], ["if:enabled = 'true'"])
        self.assertIn('\\n\\t', output)

    def test_features_change_compiled_selection_and_metadata(self):
        doc, _ = self.document("--feature", "demo-if:diagnostics")
        n = {n["name"]: n for n in doc["nodes"]}
        self.assertEqual(n["diagnostic-mode"]["type_name"], "empty")
        self.assertEqual(doc["modules"][1]["enabled_features"], ["diagnostics"])
        default, _ = self.document()
        self.assertEqual(len(doc["nodes"]), len(default["nodes"]) + 1)
        self.assertEqual(default["modules"][1]["enabled_features"], [])

    def test_bytes_are_deterministic_across_module_argument_order(self):
        _, first = self.document()
        _, second = self.document(modules=[FIXTURES / "demo-augment.yang", FIXTURES / "demo-if.yang"])
        self.assertEqual(first, second)

    def test_bad_inputs_fail_without_partial_json(self):
        for options in [("--feature", "demo-if:missing"), ("--feature", "missing:x"), ("--feature", "broken"), ("--invalid",)]:
            result = self.run_export(*options)
            self.assertNotEqual(result.returncode, 0, options)
            self.assertEqual(result.stdout, "")
        with tempfile.TemporaryDirectory() as tmp:
            invalid = pathlib.Path(tmp) / "invalid.yang"
            invalid.write_text("module invalid { namespace 'urn:invalid'; prefix invalid; leaf bad { type missing; } }")
            result = self.run_export(modules=[invalid])
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(result.stdout, "")

    def test_presence_means_explicit_statement_including_uses_refine(self):
        # The pinned source sets LYS_PRESENCE only for a presence statement.
        # Its header describes broader historical semantics; exercise the
        # meaningful NP cases and a refinement instead of trusting that comment.
        schema = '''module presence-check {
          yang-version 1.1;
          namespace "urn:presence-check";
          prefix pc;
          leaf flag { type boolean; }
          container conditional {
            when "../flag = 'true'";
            leaf value { type string; }
          }
          container operations {
            action run {}
            notification event {}
          }
          container explicit { presence ""; leaf value { type string; } }
          grouping group { container grouped { leaf value { type string; } } }
          container owner {
            uses group { refine grouped { presence "Enable grouped."; } }
          }
        }'''
        with tempfile.TemporaryDirectory() as tmp:
            source = pathlib.Path(tmp) / "presence-check.yang"
            source.write_text(schema)
            doc, _ = self.document(modules=[source])
        containers = {n["name"]: n for n in doc["nodes"] if n["kind"] == "container"}
        self.assertFalse(containers["conditional"]["presence"])
        self.assertFalse(containers["operations"]["presence"])
        self.assertFalse(containers["owner"]["presence"])
        self.assertTrue(containers["explicit"]["presence"])
        self.assertTrue(containers["grouped"]["presence"])

    def test_depth_limit_counts_the_virtual_root_and_rejects_before_output(self):
        def schema(containers):
            return (
                'module depth-check { namespace "urn:depth-check"; prefix d; '
                + ''.join(f'container c{i} {{ ' for i in range(containers))
                + 'leaf end { type string; } '
                + '} ' * containers
                + '}'
            )

        with tempfile.TemporaryDirectory() as tmp:
            source = pathlib.Path(tmp) / "depth-check.yang"
            # Root + 254 containers + terminal leaf = 256 rows on the path.
            source.write_text(schema(254))
            doc, _ = self.document(modules=[source])
            self.assertEqual(len(doc["nodes"]), 256)
            self.assertEqual(doc["nodes"][-1]["name"], "end")
            self.assertEqual(doc["nodes"][-1]["parent"], 254)

            # One additional container exceeds the importer's path limit;
            # also cover the review's 256-container + terminal-leaf example.
            for containers in (255, 256):
                with self.subTest(containers=containers):
                    source.write_text(schema(containers))
                    result = self.run_export(modules=[source])
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn("depth", result.stderr)
                    self.assertEqual(result.stdout, "")

    def test_xml_is_a_valid_module_name_and_keeps_its_own_namespace(self):
        with tempfile.TemporaryDirectory() as tmp:
            source = pathlib.Path(tmp) / "xml.yang"
            source.write_text('module xml { namespace "urn:xml"; prefix z; leaf item { type string; } }')
            doc, _ = self.document(modules=[source])
        self.assertEqual(doc["modules"], [{
            "name": "xml", "namespace": "urn:xml", "prefix": "z",
            "revision": None, "enabled_features": [],
        }])
        self.assertEqual(doc["nodes"][1]["module"], "xml")
        self.assertEqual(doc["nodes"][1]["namespace"], "urn:xml")
        self.assertEqual(doc["nodes"][1]["name"], "item")


if __name__ == "__main__":
    unittest.main()
