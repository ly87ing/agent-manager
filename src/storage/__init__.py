"""Unified SQLite storage for Agent Manager (Phase 1A).

Provides manager.db backing for local user definitions, configurations,
and tombstones with atomic migration from legacy 3-layer files,
read/write operations, and safe non-sensitive runtime projection.
"""

from src.storage.db import (
    DatabaseNotReadyError,
    connect_db,
    connect_ready_db,
    ensure_db_ready,
    get_data_dir,
    get_db_path,
    init_schema,
    is_sqlite_active,
)
from src.storage.migration import (
    migrate_if_needed,
    MigrationError,
)
from src.storage.projector import (
    project_config_from_db,
    write_projected_config,
)
from src.storage.operations import (
    record_operation,
    policy_get_db,
    policy_set_db,
    policy_delete_db,
)

__all__ = [
    "get_data_dir",
    "get_db_path",
    "is_sqlite_active",
    "connect_db",
    "connect_ready_db",
    "ensure_db_ready",
    "DatabaseNotReadyError",
    "init_schema",
    "migrate_if_needed",
    "MigrationError",
    "project_config_from_db",
    "write_projected_config",
    "record_operation",
    "policy_get_db",
    "policy_set_db",
    "policy_delete_db",
]
