"""Benchmark PickerScreen startup and search latency."""
import asyncio
import time
from pathlib import Path

from sky_music.config import AppConfig
from sky_music.ui.textual_app.app import SkyPickerApp


async def bench() -> None:
    print("Initializing App...")
    t0 = time.perf_counter()
    app = SkyPickerApp(initial_dry_run=True, cfg=AppConfig())
    t1 = time.perf_counter()
    print(f"App initialization: {(t1 - t0) * 1000:.2f}ms")

    async with app.run_test() as pilot:
        await pilot.pause()
        t2 = time.perf_counter()
        print(f"App mount & first frame: {(t2 - t1) * 1000:.2f}ms")

        picker = app._find_picker_screen()
        assert picker is not None
        
        print("Simulating search...")
        t3 = time.perf_counter()
        await pilot.click("#search")
        await pilot.press("a")
        # Wait for debounce and search result
        for _ in range(20):
            await pilot.pause(0.05)
            if picker._search_timer is None:
                break
        t4 = time.perf_counter()
        print(f"Search 'a' latency: {(t4 - t3) * 1000:.2f}ms")
        
        await pilot.press("escape")

if __name__ == "__main__":
    asyncio.run(bench())
