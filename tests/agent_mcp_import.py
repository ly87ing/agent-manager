import importlib.util
from pathlib import Path


MODULE_PATH = Path(__file__).resolve().parent.parent / "agent-mcp" / "sync_mcp.py"
SPEC = importlib.util.spec_from_file_location("sync_mcp", MODULE_PATH)
sync_mcp = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(sync_mcp)
