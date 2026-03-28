import types
import unittest
from unittest import mock

from src import toml_compat


class TomlCompatTests(unittest.TestCase):
    def setUp(self):
        toml_compat.load_tomllib.cache_clear()

    def tearDown(self):
        toml_compat.load_tomllib.cache_clear()

    def test_falls_back_to_tomli_when_tomllib_is_unavailable(self):
        fake_tomli = types.SimpleNamespace(
            loads=lambda value: {"parsed": value},
            TOMLDecodeError=ValueError,
        )

        def fake_import(name):
            if name == "tomllib":
                raise ModuleNotFoundError("tomllib")
            if name == "tomli":
                return fake_tomli
            raise AssertionError(f"unexpected import: {name}")

        with mock.patch("src.toml_compat.importlib.import_module", side_effect=fake_import):
            module = toml_compat.load_tomllib()
            self.assertIs(module, fake_tomli)
            self.assertEqual(toml_compat.tomllib.loads("title='demo'"), {"parsed": "title='demo'"})
            self.assertIs(toml_compat.tomllib.TOMLDecodeError, ValueError)

    def test_raises_helpful_error_when_no_toml_parser_is_available(self):
        with mock.patch("src.toml_compat.importlib.import_module", side_effect=ModuleNotFoundError("missing")):
            with self.assertRaises(ModuleNotFoundError) as ctx:
                toml_compat.load_tomllib()

        self.assertIn("Python 3.11+", str(ctx.exception))
        self.assertIn("tomli", str(ctx.exception))
