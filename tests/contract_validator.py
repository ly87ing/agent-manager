"""Zero-dependency validator for the contract/engine-v1 JSON Schema subset.

Pure stdlib; runs on Python 3.9+ (CI matrix) with no third-party packages such as
jsonschema. The supported subset is deliberately small and is the same whitelist
enforced on the schema files themselves (see contract/README.md):

    type / properties / required / additionalProperties / items / enum /
    $defs / $ref (local '#/...' JSON pointers only) / description

Validation collects all errors instead of failing fast. Every error carries a
JSON path pointing at the offending instance value, e.g.
``$.data.elements[0].freshness.state``. A missing required property is reported
at the containing object; an unexpected additional property is reported at the
extra key itself.

Schemas with structural problems (keywords outside the subset, non-local $ref,
unresolvable or circular $ref) raise SchemaError rather than producing
validation errors: those are contract bugs, not data bugs.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Dict, List, Set, Tuple

SCHEMA_KEYWORDS = frozenset(
    {
        "type",
        "properties",
        "required",
        "additionalProperties",
        "items",
        "enum",
        "$defs",
        "$ref",
        "description",
    }
)

JSON_TYPE_NAMES = frozenset({"object", "array", "string", "integer", "number", "boolean", "null"})


class SchemaError(ValueError):
    """Raised when a schema is outside the supported subset or malformed."""


@dataclass(frozen=True)
class ValidationError:
    """One instance/schema mismatch at a concrete JSON path."""

    path: str
    message: str

    def __str__(self) -> str:
        return "%s: %s" % (self.path, self.message)


def load_schema(path: Path) -> Dict[str, Any]:
    """Read a schema file, parse it as JSON and check it against the subset."""
    schema = json.loads(Path(path).read_text(encoding="utf-8"))
    check_schema(schema)
    return schema


def check_schema(schema: Any, _path: str = "#") -> None:
    """Raise SchemaError unless every node of ``schema`` stays within the subset."""
    if not isinstance(schema, dict):
        raise SchemaError("%s: schema node must be an object, got %s" % (_path, _type_name(schema)))
    unknown = sorted(set(schema) - SCHEMA_KEYWORDS)
    if unknown:
        raise SchemaError(
            "%s: keyword(s) outside the contract subset: %s (allowed: %s)"
            % (_path, ", ".join(unknown), ", ".join(sorted(SCHEMA_KEYWORDS)))
        )

    if "$ref" in schema:
        ref = schema["$ref"]
        if not isinstance(ref, str) or not ref.startswith("#/"):
            raise SchemaError("%s.$ref: only local '#/...' references are allowed, got %r" % (_path, ref))
        # $defs is a definition container, not a validation keyword, so it is a
        # legitimate sibling of $ref (e.g. a root-level $ref plus local $defs).
        siblings = set(schema) - {"$ref", "$defs", "description"}
        if siblings:
            raise SchemaError(
                "%s: $ref nodes must not carry validation siblings: %s" % (_path, ", ".join(sorted(siblings)))
            )

    if "description" in schema and not isinstance(schema["description"], str):
        raise SchemaError("%s.description: must be a string" % _path)

    if "type" in schema:
        type_value = schema["type"]
        type_names = type_value if isinstance(type_value, list) else [type_value]
        if not type_names or any(t not in JSON_TYPE_NAMES for t in type_names):
            raise SchemaError("%s.type: must be one or more of %s" % (_path, sorted(JSON_TYPE_NAMES)))

    if "enum" in schema:
        enum_value = schema["enum"]
        if not isinstance(enum_value, list) or not enum_value:
            raise SchemaError("%s.enum: must be a non-empty array" % _path)

    if "required" in schema:
        required = schema["required"]
        if not isinstance(required, list) or any(not isinstance(name, str) for name in required):
            raise SchemaError("%s.required: must be an array of strings" % _path)

    if "properties" in schema:
        properties = schema["properties"]
        if not isinstance(properties, dict):
            raise SchemaError("%s.properties: must be an object" % _path)
        for name, subschema in properties.items():
            check_schema(subschema, "%s.properties.%s" % (_path, name))

    if "items" in schema:
        check_schema(schema["items"], "%s.items" % _path)

    if "additionalProperties" in schema:
        extra = schema["additionalProperties"]
        if not isinstance(extra, bool):
            check_schema(extra, "%s.additionalProperties" % _path)

    if "$defs" in schema:
        defs = schema["$defs"]
        if not isinstance(defs, dict):
            raise SchemaError("%s.$defs: must be an object" % _path)
        for name, subschema in defs.items():
            check_schema(subschema, "%s.$defs.%s" % (_path, name))


def validate(schema: Dict[str, Any], instance: Any) -> List[ValidationError]:
    """Validate ``instance`` against ``schema``; return all errors (empty = valid).

    Raises SchemaError if the schema itself leaves the supported subset.
    """
    check_schema(schema)
    errors: List[ValidationError] = []
    _validate(schema, instance, "$", schema, set(), errors)
    return errors


def _validate(
    schema: Dict[str, Any],
    instance: Any,
    path: str,
    root: Dict[str, Any],
    active_refs: Set[Tuple[int, int]],
    errors: List[ValidationError],
) -> None:
    if "$ref" in schema:
        ref = schema["$ref"]
        target = _resolve_ref(root, ref)
        marker = (id(target), id(instance))
        if marker in active_refs:
            raise SchemaError("circular $ref %r at instance path %s" % (ref, path))
        active_refs.add(marker)
        try:
            _validate(target, instance, path, root, active_refs, errors)
        finally:
            active_refs.discard(marker)
        return

    if "enum" in schema:
        if any(_json_equal(instance, candidate) for candidate in schema["enum"]):
            pass
        else:
            errors.append(
                ValidationError(
                    path,
                    "expected one of %s, got %s"
                    % (json.dumps(schema["enum"], ensure_ascii=False), json.dumps(instance, ensure_ascii=False)),
                )
            )
            return

    if "type" in schema and not _matches_type(instance, schema["type"]):
        errors.append(
            ValidationError(
                path,
                "expected %s, got %s" % (_type_expectation(schema["type"]), _type_name(instance)),
            )
        )
        return

    if isinstance(instance, dict):
        for name in schema.get("required", []):
            if name not in instance:
                errors.append(ValidationError(path, "missing required property %r" % name))
        properties = schema.get("properties", {})
        for name, subschema in properties.items():
            if name in instance:
                _validate(subschema, instance[name], "%s.%s" % (path, name), root, active_refs, errors)
        additional = schema.get("additionalProperties", True)
        for name in sorted(set(instance) - set(properties)):
            if additional is False:
                errors.append(ValidationError("%s.%s" % (path, name), "unexpected additional property %r" % name))
            elif isinstance(additional, dict):
                _validate(additional, instance[name], "%s.%s" % (path, name), root, active_refs, errors)

    if isinstance(instance, list) and "items" in schema:
        for index, item in enumerate(instance):
            _validate(schema["items"], item, "%s[%d]" % (path, index), root, active_refs, errors)


def _resolve_ref(root: Dict[str, Any], ref: str) -> Dict[str, Any]:
    """Resolve a local '#/...' JSON pointer against the schema document root."""
    node: Any = root
    for raw_token in ref[2:].split("/"):
        token = raw_token.replace("~1", "/").replace("~0", "~")
        if not isinstance(node, dict) or token not in node:
            raise SchemaError("unresolvable $ref %r" % ref)
        node = node[token]
    if not isinstance(node, dict):
        raise SchemaError("$ref %r does not point at a schema object" % ref)
    return node


def _matches_type(instance: Any, type_value: Any) -> bool:
    type_names = type_value if isinstance(type_value, list) else [type_value]
    return any(_matches_single_type(instance, name) for name in type_names)


def _matches_single_type(instance: Any, type_name: str) -> bool:
    if type_name == "object":
        return isinstance(instance, dict)
    if type_name == "array":
        return isinstance(instance, list)
    if type_name == "string":
        return isinstance(instance, str)
    if type_name == "boolean":
        return isinstance(instance, bool)
    if type_name == "null":
        return instance is None
    if type_name == "integer":
        # JSON Schema counts any number with zero fractional part as integer.
        if isinstance(instance, bool):
            return False
        if isinstance(instance, int):
            return True
        return isinstance(instance, float) and instance.is_integer()
    if type_name == "number":
        return isinstance(instance, (int, float)) and not isinstance(instance, bool)
    raise SchemaError("unknown type %r" % type_name)


def _type_expectation(type_value: Any) -> str:
    if isinstance(type_value, list):
        return " or ".join(type_value)
    return str(type_value)


def _type_name(instance: Any) -> str:
    if instance is None:
        return "null"
    if isinstance(instance, bool):
        return "boolean"
    if isinstance(instance, dict):
        return "object"
    if isinstance(instance, list):
        return "array"
    if isinstance(instance, str):
        return "string"
    if isinstance(instance, int):
        return "integer"
    if isinstance(instance, float):
        return "number"
    return type(instance).__name__


def _json_equal(left: Any, right: Any) -> bool:
    """JSON-semantic equality: booleans never equal numbers; 1 equals 1.0."""
    if isinstance(left, bool) or isinstance(right, bool):
        return isinstance(left, bool) and isinstance(right, bool) and left == right
    if isinstance(left, list) and isinstance(right, list):
        return len(left) == len(right) and all(_json_equal(l_item, r_item) for l_item, r_item in zip(left, right))
    if isinstance(left, dict) and isinstance(right, dict):
        if set(left) != set(right):
            return False
        return all(_json_equal(left[key], right[key]) for key in left)
    return left == right
