from __future__ import annotations

from concurrent.futures import ProcessPoolExecutor, ThreadPoolExecutor
from typing import Any
from sky_music.infrastructure.background import BackgroundScope, ExecutorResource

class ClassicPickerBackgroundHelper:
    """Helper owning background lifecycle setup, fallback, and dọn dẹp for the classic picker."""
    def __init__(self) -> None:
        self.picker_scope = BackgroundScope(phase="picker")
        self.metadata_resource: ExecutorResource | None = None
        self.metadata_uses_process_pool = True
        self.cache_resource: ExecutorResource | None = None

    def setup_resources(self, worker_process_warmup: Any) -> None:
        try:
            process_exec = ProcessPoolExecutor(max_workers=2)
            self.metadata_resource = self.picker_scope.register(
                ExecutorResource(
                    name="classic-picker-metadata-process",
                    phase="picker",
                    executor=process_exec,
                )
            )
            try:
                self.metadata_resource.submit(worker_process_warmup)
            except Exception:
                pass
        except Exception:
            self.metadata_uses_process_pool = False
            thread_exec = ThreadPoolExecutor(max_workers=2, thread_name_prefix="sky-picker-meta")
            self.metadata_resource = self.picker_scope.register(
                ExecutorResource(
                    name="classic-picker-metadata-thread",
                    phase="picker",
                    executor=thread_exec,
                )
            )

        cache_exec = ThreadPoolExecutor(max_workers=1, thread_name_prefix="sky-picker-cache")
        self.cache_resource = self.picker_scope.register(
            ExecutorResource(
                name="classic-picker-cache",
                phase="picker",
                executor=cache_exec,
            )
        )

    def handle_fallback(self) -> None:
        """Trigger thread fallback if the process worker failed."""
        if self.metadata_uses_process_pool:
            self.metadata_uses_process_pool = False
            try:
                if self.metadata_resource is not None:
                    self.picker_scope.retire(self.metadata_resource)
                    self.metadata_resource.cancel()
            except Exception:
                pass
            thread_exec = ThreadPoolExecutor(max_workers=2, thread_name_prefix="sky-picker-meta")
            self.metadata_resource = self.picker_scope.register(
                ExecutorResource(
                    name="classic-picker-metadata-thread",
                    phase="picker",
                    executor=thread_exec,
                )
            )

    def close(self) -> None:
        self.picker_scope.close_all(wait=True)
