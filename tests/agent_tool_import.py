import importlib.util
from pathlib import Path


MODULE_PATH = Path(__file__).resolve().parent.parent / "agent-tool" / "sync_tools.py"
SPEC = importlib.util.spec_from_file_location("sync_tools", MODULE_PATH)
sync_tools = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(sync_tools)
