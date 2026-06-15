"""Dataclasses modelling resolved browser / exit / runtime state."""
from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class BrowserRoot:
    name: str
    user_data_dir: Path


@dataclass(frozen=True)
class ExitProfile:
    ip: str | None
    country_code: str | None
    timezone_id: str | None
    latitude: float | None
    longitude: float | None
    city: str | None
    region: str | None
    source: str

    def is_us(self) -> bool:
        return self.country_code == "US"


@dataclass(frozen=True)
class ExitResolution:
    return_code: int
    exit_profile: ExitProfile | None
    verification_state: str
    errors: tuple[str, ...] = ()


@dataclass(frozen=True)
class GoogleDomainResolution:
    return_code: int
    verification_state: str
    final_url: str | None = None
    hk_markers: tuple[str, ...] = ()
    errors: tuple[str, ...] = ()


@dataclass(frozen=True)
class BrowserGoogleResolution:
    return_code: int
    verification_state: str
    final_url: str | None = None
    title: str | None = None
    hk_markers: tuple[str, ...] = ()
    process_state: str | None = None
    errors: tuple[str, ...] = ()


@dataclass(frozen=True)
class RuntimeResolution:
    return_code: int
    verification_state: str
    observed: dict | None = None
    process_state: str | None = None
    errors: tuple[str, ...] = ()


@dataclass(frozen=True)
class AutomationContextResolution:
    return_code: int
    verification_state: str
    observed: dict | None = None
    process_state: str | None = None
    errors: tuple[str, ...] = ()

