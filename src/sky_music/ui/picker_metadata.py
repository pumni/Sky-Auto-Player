# pyright: reportUnusedImport=false

"""Compatibility alias for the shared song metadata service.

The module alias keeps legacy imports, including tests that inspect the
service's bounded caches, attached to the same implementation module.
"""

# Type-only compatibility names below intentionally mirror the legacy module.
# They make static analysis understand imports whose runtime module is aliased.
# ruff: noqa: F401

import sys as _sys
from typing import TYPE_CHECKING

from sky_music.orchestration import song_metadata_service as _service

if TYPE_CHECKING:
    from sky_music.orchestration.song_metadata_service import (
        SongUiMetadata,
        _cache_lock,
        _effective_policy_signature,
        _metadata_cache,
        _metadata_to_payload,
        _path_session_ram_cache,
        _path_session_ram_lock,
        _persistent_cache,
        _persistent_cache_key,
        _persistent_loaded,
        _pkey_ram_cache,
        _pkey_ram_lock,
        _raw_cache,
        clear_metadata_cache,
        compute_raw_song_ui_metadata,
        get_cached_song_ui_metadata,
        hydrate_persistent_metadata_for_paths,
        invalidate_policy_metadata,
        peek_cached_song_ui_metadata,
        populate_raw_song_ui_metadata_for_paths,
        store_computed_song_ui_metadata_payloads,
        warm_persistent_metadata_cache,
        worker_process_warmup,
    )

_sys.modules[__name__] = _service
