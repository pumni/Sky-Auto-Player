"""Single source of truth for update channel behavior.

This module defines the authoritative policy for how each update channel
behaves with respect to pre-release versions and GitHub API endpoints.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

Channel = Literal["stable", "beta"]


@dataclass(frozen=True, slots=True)
class UpdatePolicy:
    """Policy for a specific update channel."""

    include_prerelease: bool
    """Whether to include pre-release versions (rc, beta, alpha, dev)."""

    github_api_path: str
    """GitHub Releases API path: '/releases/latest' or '/releases?per_page=10'."""


# Policy table: single source of truth for channel behavior
POLICIES: dict[Channel, UpdatePolicy] = {
    "stable": UpdatePolicy(
        include_prerelease=False,
        github_api_path="/releases/latest",
    ),
    "beta": UpdatePolicy(
        include_prerelease=True,
        github_api_path="/releases?per_page=10",
    ),
}

DEFAULT_CHANNEL: Channel = "stable"


def get_policy(channel: Channel | str) -> UpdatePolicy:
    """Return the policy for a given channel, defaulting to stable."""
    normalized = channel.strip().lower() if isinstance(channel, str) else DEFAULT_CHANNEL
    if normalized in POLICIES:
        return POLICIES[normalized]
    return POLICIES[DEFAULT_CHANNEL]


def channel_from_config(channel: str | None) -> Channel:
    """Normalize a channel string from config to a valid Channel."""
    if channel is None:
        return DEFAULT_CHANNEL
    normalized = channel.strip().lower()
    if normalized in ("stable", "beta"):
        return normalized
    return DEFAULT_CHANNEL