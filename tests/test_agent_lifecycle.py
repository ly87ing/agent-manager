import unittest
from unittest import mock

from agent_lifecycle_import import sync_agents


class DetectAllProvidersTests(unittest.TestCase):
    @mock.patch.object(sync_agents, "ProviderCache")
    @mock.patch.object(sync_agents.os.path, "exists", return_value=False)
    @mock.patch.object(sync_agents.os.path, "realpath")
    @mock.patch.object(sync_agents.shutil, "which", return_value="/usr/local/bin/claude")
    def test_detects_npm_when_bin_resolves_into_node_modules(
        self,
        _which,
        realpath,
        _exists,
        provider_cache_cls,
    ):
        cache = provider_cache_cls.return_value
        cache.get_gh_extensions.return_value = ""
        cache.get_brew_formulae.return_value = set()
        cache.get_brew_casks.return_value = set()
        realpath.return_value = "/usr/local/lib/node_modules/@anthropic-ai/claude-code/bin/claude.js"

        providers = sync_agents.detect_all_providers(
            "claude",
            {
                "providers": {
                    "brew": {
                        "install": "brew install --cask claude-code",
                    }
                }
            },
        )

        self.assertIn("npm", providers)
