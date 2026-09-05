"""Contract tests for contract/engine-v1 (Engine Contract v1).

Three layers, per docs/goal-rust-gui.md Phase 0 Slice A:

- ContractValidatorTests: self-tests of the zero-dependency subset validator
  (tests/contract_validator.py), proving each supported keyword behaves and
  each misuse raises SchemaError.
- EngineContractSchemaTests: the shipped schema files are valid JSON, stay
  within the subset keyword whitelist, use resolvable local-only $refs, and
  keep their inlined cross-file copies (status inlines element, plan inlines
  plan-action) equal to the single-source schema files.
- EngineContractFixtureTests: every fixture under tests/fixtures/engine-v1/
  is exercised; *.valid.json must validate clean, *.invalid.json must fail at
  exactly the JSON path recorded in EXPECTED_INVALID_PATHS.
"""

import json
import re
import unittest
from pathlib import Path

from tests.contract_validator import (
    SCHEMA_KEYWORDS,
    SchemaError,
    check_schema,
    load_schema,
    validate,
)

ROOT = Path(__file__).resolve().parent.parent
CONTRACT_DIR = ROOT / "contract" / "engine-v1"
FIXTURE_DIR = ROOT / "tests" / "fixtures" / "engine-v1"

SCHEMA_FILES = {
    "envelope": "envelope.schema.json",
    "element": "element.schema.json",
    "plan-action": "plan-action.schema.json",
    "status": "status.schema.json",
    "plan": "plan.schema.json",
    "apply-result": "apply-result.schema.json",
    "converge": "converge.schema.json",
    "runs": "runs.schema.json",
    "snapshots": "snapshots.schema.json",
    "policy": "policy.schema.json",
}

FIXTURE_NAME_RE = re.compile(r"^(envelope|element|plan-action|apply-result|status|plan|converge|runs|snapshots|policy)-.+\.(valid|invalid)\.json$")

# Every *.invalid.json fixture maps to the exact JSON path it must fail at.
# Missing required properties are reported at the containing object; type/enum
# violations at the offending value; extra keys at the key itself.
EXPECTED_INVALID_PATHS = {
    "envelope-missing-ok.invalid.json": "$",
    "envelope-bad-contract.invalid.json": "$.contract",
    "envelope-extra-property.invalid.json": "$.unexpected",
    "element-bad-state.invalid.json": "$.state",
    "element-missing-freshness.invalid.json": "$",
    "element-bad-action-risk.invalid.json": "$.actions[0].risk",
    "plan-action-bad-op.invalid.json": "$.op",
    "plan-action-missing-risk.invalid.json": "$",
    "status-missing-elements.invalid.json": "$",
    "status-bad-freshness-state.invalid.json": "$.elements[0].freshness.state",
    "plan-missing-summary.invalid.json": "$",
    "plan-bad-action-op.invalid.json": "$.actions[0].op",
    "plan-summary-missing-risk-key.invalid.json": "$.summary",
    "apply-result-missing-snapshot.invalid.json": "$",
    "apply-result-bad-step-exit.invalid.json": "$.steps[0].exit_code",
    "converge-bad-pipeline.invalid.json": "$.pipeline",
    "converge-missing-steps.invalid.json": "$",
    "runs-bad-is-latest.invalid.json": "$.runs[0].is_latest",
    "runs-missing-tail.invalid.json": "$",
    "snapshots-bad-file-count.invalid.json": "$.snapshots[0].file_count",
    "snapshots-restore-missing-resnapshot.invalid.json": "$.restore",
    "policy-missing-attribution.invalid.json": "$",
    "policy-bad-layer-type.invalid.json": "$.layers.baseline",
}


def _load_schema(schema_key):
    return load_schema(CONTRACT_DIR / SCHEMA_FILES[schema_key])


def _load_fixture(name):
    return json.loads((FIXTURE_DIR / name).read_text(encoding="utf-8"))


class ContractValidatorTests(unittest.TestCase):
    """Self-tests of the subset validator; no files involved."""

    def test_type_keyword_accepts_a_list_of_types(self):
        schema = {"type": ["integer", "null"]}
        self.assertEqual(validate(schema, None), [])
        self.assertEqual(validate(schema, 7), [])
        errors = validate(schema, "7")
        self.assertEqual([e.path for e in errors], ["$"])
        self.assertIn("expected integer or null", errors[0].message)
        self.assertIn("got string", errors[0].message)

    def test_integer_type_rejects_boolean_and_fractional_float(self):
        schema = {"type": "integer"}
        self.assertEqual(validate(schema, 3), [])
        self.assertEqual(validate(schema, 3.0), [], "JSON Schema counts 3.0 as integer")
        self.assertEqual(len(validate(schema, True)), 1)
        self.assertEqual(len(validate(schema, 3.5)), 1)

    def test_number_type_accepts_int_and_float_but_not_bool(self):
        schema = {"type": "number"}
        self.assertEqual(validate(schema, 3), [])
        self.assertEqual(validate(schema, 3.5), [])
        self.assertEqual(len(validate(schema, False)), 1)

    def test_required_reports_parent_path_and_names_property(self):
        schema = {"type": "object", "required": ["ok"]}
        errors = validate(schema, {})
        self.assertEqual(len(errors), 1)
        self.assertEqual(errors[0].path, "$")
        self.assertIn("'ok'", errors[0].message)

    def test_additional_properties_false_flags_extra_key_at_key_path(self):
        schema = {
            "type": "object",
            "properties": {"ok": {"type": "boolean"}},
            "additionalProperties": False,
        }
        errors = validate(schema, {"ok": True, "bogus": 1})
        self.assertEqual([e.path for e in errors], ["$.bogus"])
        self.assertIn("bogus", errors[0].message)

    def test_properties_recurse_into_nested_paths(self):
        schema = {
            "type": "object",
            "properties": {
                "lock": {
                    "type": "object",
                    "properties": {"held": {"type": "boolean"}},
                }
            },
        }
        errors = validate(schema, {"lock": {"held": "yes"}})
        self.assertEqual([e.path for e in errors], ["$.lock.held"])

    def test_items_reports_element_index_in_path(self):
        schema = {"type": "array", "items": {"type": "string"}}
        errors = validate(schema, ["a", 1, "c"])
        self.assertEqual([e.path for e in errors], ["$[1]"])

    def test_enum_distinguishes_boolean_from_integer(self):
        self.assertEqual(validate({"enum": [1]}, 1), [])
        self.assertEqual(len(validate({"enum": [1]}, True)), 1, "true must not satisfy enum [1]")
        self.assertEqual(len(validate({"enum": [True]}, 1)), 1, "1 must not satisfy enum [true]")
        self.assertEqual(validate({"enum": [1]}, 1.0), [], "1.0 equals 1 in JSON semantics")

    def test_enum_error_message_lists_allowed_values(self):
        errors = validate({"enum": ["current", "behind"]}, "stale")
        self.assertEqual(len(errors), 1)
        self.assertIn('"current"', errors[0].message)
        self.assertIn('"stale"', errors[0].message)

    def test_enum_and_type_both_apply(self):
        schema = {"type": "integer", "enum": [1, 2]}
        self.assertEqual(validate(schema, 1), [])
        self.assertEqual(len(validate(schema, "1")), 1, "type check still runs when enum passes are impossible")
        self.assertEqual(len(validate(schema, 3)), 1)

    def test_local_ref_resolves_against_document_root(self):
        schema = {
            "type": "object",
            "properties": {"warnings": {"type": "array", "items": {"$ref": "#/$defs/diagnostic"}}},
            "$defs": {
                "diagnostic": {"type": "object", "required": ["code"]},
            },
        }
        self.assertEqual(validate(schema, {"warnings": [{"code": "X"}]}), [])
        errors = validate(schema, {"warnings": [{}]})
        self.assertEqual([e.path for e in errors], ["$.warnings[0]"])

    def test_ref_inside_inlined_defs_resolves_from_embedding_document_root(self):
        # Mirrors how status.schema.json inlines element.schema.json: the copy
        # keeps its '#/...' refs, which must resolve at the embedding document.
        schema = {
            "type": "object",
            "properties": {"elements": {"type": "array", "items": {"$ref": "#/$defs/element"}}},
            "$defs": {
                "element": {
                    "type": "object",
                    "properties": {"freshness": {"$ref": "#/$defs/freshness"}},
                },
                "freshness": {"type": "object", "required": ["state"]},
            },
        }
        errors = validate(schema, {"elements": [{"freshness": {}}]})
        self.assertEqual([e.path for e in errors], ["$.elements[0].freshness"])

    def test_non_local_ref_is_rejected_as_schema_error(self):
        for ref in ("./element.schema.json#/$defs/element", "https://example.com/s.json#/$defs/x"):
            with self.subTest(ref=ref):
                with self.assertRaises(SchemaError):
                    validate({"$ref": ref}, {})

    def test_unresolvable_ref_raises_schema_error(self):
        with self.assertRaisesRegex(SchemaError, "unresolvable"):
            validate({"$ref": "#/$defs/nope"}, {})

    def test_circular_ref_raises_schema_error(self):
        schema = {
            "$defs": {
                "loop_a": {"$ref": "#/$defs/loop_b"},
                "loop_b": {"$ref": "#/$defs/loop_a"},
            },
            "$ref": "#/$defs/loop_a",
        }
        with self.assertRaisesRegex(SchemaError, "circular"):
            validate(schema, {})

    def test_check_schema_rejects_keywords_outside_the_subset(self):
        for keyword in ("oneOf", "anyOf", "allOf", "pattern", "format", "minimum", "minLength", "$id"):
            with self.subTest(keyword=keyword):
                with self.assertRaisesRegex(SchemaError, keyword.replace("$", r"\$")):
                    check_schema({"type": "object", keyword: "anything"})

    def test_check_schema_rejects_ref_with_validation_siblings(self):
        with self.assertRaisesRegex(SchemaError, "siblings"):
            check_schema({"$ref": "#/$defs/x", "type": "object"})

    def test_validate_collects_multiple_errors_instead_of_failing_fast(self):
        schema = {
            "type": "object",
            "required": ["a", "b"],
            "properties": {"c": {"type": "integer"}},
        }
        errors = validate(schema, {"c": "nope"})
        self.assertEqual(len(errors), 3)
        self.assertEqual([e.path for e in errors], ["$", "$", "$.c"])

    def test_unknown_type_value_is_a_schema_error(self):
        with self.assertRaises(SchemaError):
            validate({"type": "uuid"}, "x")


class EngineContractSchemaTests(unittest.TestCase):
    """The shipped schema files themselves."""

    def test_schema_files_exist(self):
        for filename in sorted(SCHEMA_FILES.values()):
            self.assertTrue((CONTRACT_DIR / filename).is_file(), "missing %s" % filename)

    def test_schemas_are_valid_json_within_the_subset_whitelist(self):
        for filename in sorted(SCHEMA_FILES.values()):
            with self.subTest(schema=filename):
                schema = _load_schema_by_name(filename)
                check_schema(schema)  # raises unless every node stays in SCHEMA_KEYWORDS

    def test_every_ref_is_local_and_resolvable(self):
        for filename in sorted(SCHEMA_FILES.values()):
            with self.subTest(schema=filename):
                schema = _load_schema_by_name(filename)
                for ref in _collect_refs(schema):
                    self.assertTrue(ref.startswith("#/"), "%s uses non-local ref %r" % (filename, ref))
                    node = schema
                    for token in ref[2:].split("/"):
                        node = node[token]

    def test_status_schema_inlines_element_schema_verbatim(self):
        status = _load_schema("status")
        element = _load_schema("element")
        element_root = {key: value for key, value in element.items() if key != "$defs"}
        self.assertEqual(status["$defs"]["element"], element_root)
        self.assertEqual(status["$defs"]["freshness"], element["$defs"]["freshness"])
        self.assertEqual(status["$defs"]["action"], element["$defs"]["action"])

    def test_plan_schema_inlines_plan_action_schema_verbatim(self):
        plan = _load_schema("plan")
        plan_action = _load_schema("plan-action")
        self.assertEqual(plan["$defs"]["plan_action"], plan_action)

    def test_keyword_whitelist_matches_readme(self):
        readme = (ROOT / "contract" / "README.md").read_text(encoding="utf-8")
        for keyword in sorted(SCHEMA_KEYWORDS):
            self.assertIn("`%s`" % keyword, readme, "README.md must list keyword %s" % keyword)


class EngineContractFixtureTests(unittest.TestCase):
    """Every fixture is exercised; valids pass, invalids fail at the recorded path."""

    def test_fixture_inventory_matches_schema_files_and_expectation_map(self):
        fixture_files = sorted(path.name for path in FIXTURE_DIR.glob("*.json"))
        self.assertTrue(fixture_files, "no fixtures found under %s" % FIXTURE_DIR)

        counts = {key: {"valid": 0, "invalid": 0} for key in SCHEMA_FILES}
        for name in fixture_files:
            match = FIXTURE_NAME_RE.match(name)
            self.assertIsNotNone(match, "fixture %r does not follow <schema>-<case>.(valid|invalid).json" % name)
            schema_key, kind = match.groups()
            counts[schema_key][kind] += 1
            if kind == "invalid":
                self.assertIn(name, EXPECTED_INVALID_PATHS, "invalid fixture %r lacks an expected path" % name)

        for name in EXPECTED_INVALID_PATHS:
            self.assertIn(name, fixture_files, "expected-path entry %r has no fixture file" % name)

        for schema_key, tally in sorted(counts.items()):
            self.assertGreaterEqual(tally["valid"], 2, "schema %r needs at least 2 valid fixtures" % schema_key)
            self.assertGreaterEqual(tally["invalid"], 2, "schema %r needs at least 2 invalid fixtures" % schema_key)

    def test_valid_fixtures_validate_clean(self):
        for name in sorted(FIXTURE_DIR.glob("*.valid.json")):
            schema_key = FIXTURE_NAME_RE.match(name.name).group(1)
            with self.subTest(fixture=name.name):
                errors = validate(_load_schema(schema_key), _load_fixture(name.name))
                self.assertEqual(errors, [], "unexpected errors: %s" % [str(e) for e in errors])

    def test_invalid_fixtures_fail_at_the_expected_path(self):
        for name, expected_path in sorted(EXPECTED_INVALID_PATHS.items()):
            schema_key = FIXTURE_NAME_RE.match(name).group(1)
            with self.subTest(fixture=name):
                errors = validate(_load_schema(schema_key), _load_fixture(name))
                self.assertTrue(errors, "fixture %r unexpectedly validated clean" % name)
                self.assertIn(
                    expected_path,
                    [e.path for e in errors],
                    "fixture %r: expected an error at %s, got: %s"
                    % (name, expected_path, [str(e) for e in errors]),
                )


def _load_schema_by_name(filename):
    return json.loads((CONTRACT_DIR / filename).read_text(encoding="utf-8"))


def _collect_refs(node):
    refs = []
    if isinstance(node, dict):
        for key, value in node.items():
            if key == "$ref":
                refs.append(value)
            else:
                refs.extend(_collect_refs(value))
    elif isinstance(node, list):
        for item in node:
            refs.extend(_collect_refs(item))
    return refs


if __name__ == "__main__":
    unittest.main()
