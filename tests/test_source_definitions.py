import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

from src import engine_v1, source_definitions


class SourceDefinitionsTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        env = patch.dict(os.environ, {"AGENT_MANAGER_LOCAL_CONFIG": str(self.root / "config/manager.local.json"), "AGENT_MANAGER_SYNCED_CONFIG": str(self.root / "config/manager.synced.json"), "AGENT_MANAGER_BASE_CONFIG": str(self.root / "config/manager.json")})
        env.start()
        self.addCleanup(env.stop)
        self.snapshot = Mock()
        for key, (relative, field, _) in source_definitions.SOURCES.items():
            path = self.root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(json.dumps({field: {}, "other": "preserved"}) if field else "Existing rule\n", encoding="utf-8")

    def save(self, key, value, **kwargs):
        current = source_definitions.get_source(self.root, key)
        return source_definitions.set_source(self.root, key, {"revision": current["revision"], "value": value},
            snapshot=self.snapshot, validate_hooks=engine_v1._sync_claude_hooks._validate_hooks_map, **kwargs)

    def test_alias_add_edit_remove_preserves_other_fields_and_snapshots(self):
        for value in ({"demo": "echo first"}, {"demo": "echo edited"}, {}):
            self.assertEqual(self.save("aliases.shell", value)["value"], value)
        document = json.loads((self.root / "config/shell-aliases.json").read_text())
        self.assertEqual(document["other"], "preserved")
        self.assertEqual(self.snapshot.call_count, 3)

    def test_rules_edit_clear_and_dry_run_only_touch_source(self):
        self.save("rules.shared", "Changed rule\n", dry_run=True)
        self.assertEqual(source_definitions.get_source(self.root, "rules.shared")["value"], "Existing rule\n")
        self.snapshot.assert_not_called()
        self.assertEqual(self.save("rules.shared", "New rule\n")["value"], "New rule\n")
        self.assertEqual(self.save("rules.shared", "")["value"], "")
        self.assertFalse((self.root / "agent-rule/rules/dist").exists())

    def test_hook_removal_cleans_original_command_on_sync(self):
        old = {"Stop": [{"hooks": [{"type": "command", "command": "echo old"}]}]}
        new = {"Stop": [{"hooks": [{"type": "command", "command": "echo new"}]}]}
        self.save("hooks.claude", old)
        self.save("hooks.claude", new)
        document = json.loads((self.root / "config/claude-hooks.json").read_text())
        retired = engine_v1._sync_claude_hooks._validate_removed_signatures(document["removed_signatures"])
        self.assertEqual(engine_v1._sync_claude_hooks._strip_removed_signatures(old, retired), {})
        self.assertEqual(engine_v1._sync_claude_hooks._strip_removed_signatures(new, retired), new)

    def test_stale_revision_invalid_hooks_and_traversal_rejected_without_write(self):
        with self.assertRaisesRegex(ValueError, "改变"):
            source_definitions.set_source(self.root, "aliases.shell", {"revision": "old", "value": {}}, snapshot=self.snapshot, validate_hooks=Mock())
        with self.assertRaises(ValueError):
            self.save("hooks.codex", {"Stop": "invalid"})
        with self.assertRaises(ValueError):
            source_definitions.get_source(self.root, "../../secrets")
        self.snapshot.assert_not_called()
