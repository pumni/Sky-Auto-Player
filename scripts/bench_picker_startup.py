"""Benchmark PickerScreen startup and search latency with 1000/5000 libraries."""
import asyncio
import time
import tracemalloc

from sky_music.config import AppConfig
from sky_music.ui.textual_app.app import SkyPickerApp


async def bench(num_songs: int) -> None:
    from pathlib import Path
    from unittest.mock import patch
    
    print(f"\n--- Benchmarking App with {num_songs} songs ---")
    tracemalloc.start()
    
    fake_paths = [Path(f"songs/fake_{i}.json") for i in range(num_songs)]
    t0 = time.perf_counter()
    with patch("sky_music.ui.picker_helpers.get_song_choices", return_value=fake_paths):
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
                if getattr(picker, "_search_timer", None) is None:
                    break
            t4 = time.perf_counter()
            print(f"Search 'a' latency: {(t4 - t3) * 1000:.2f}ms")
            
            await pilot.press("escape")
            
    _current, peak = tracemalloc.get_traced_memory()
    tracemalloc.stop()
    print(f"Peak memory usage: {peak / 10**6:.2f} MB")

if __name__ == "__main__":
    asyncio.run(bench(1000))
    asyncio.run(bench(5000))
